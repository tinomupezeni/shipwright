use anyhow::Result;
use bollard::Docker;
use bollard::container::ListContainersOptions;
use bollard::network::ListNetworksOptions;
use std::collections::HashMap;
use tracing::{info, debug};

#[derive(Debug, Clone)]
pub struct InfrastructureInfo {
    /// Detected proxy type and container name
    pub proxy: Option<(String, String)>,

    /// Available Docker networks
    pub networks: Vec<NetworkInfo>,

    /// Shared resources detected
    pub shared_resources: SharedResources,

    /// Existing deployment directory structure
    pub deploy_directories: Vec<String>,

    /// Whether this appears to be a multi-project setup
    pub is_multi_project: bool,
}

#[derive(Debug, Clone)]
pub struct NetworkInfo {
    pub name: String,
    pub id: String,
    pub driver: String,
    pub scope: String,
}

#[derive(Debug, Clone, Default)]
pub struct SharedResources {
    pub postgres: Option<ServiceInfo>,
    pub redis: Option<ServiceInfo>,
    pub rabbitmq: Option<ServiceInfo>,
    pub other: Vec<ServiceInfo>,
}

#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub container_name: String,
    pub image: String,
    pub networks: Vec<String>,
    pub ports: Vec<u16>,
}

/// Auto-detect existing infrastructure on the VPS
pub async fn detect_infrastructure() -> Result<InfrastructureInfo> {
    info!("🔍 Detecting existing infrastructure...");

    let docker = Docker::connect_with_socket_defaults()?;

    // Detect proxy
    let proxy = detect_proxy(&docker).await?;
    if let Some((proxy_type, container)) = &proxy {
        info!("   ✓ Detected {} proxy: {}", proxy_type, container);
    }

    // Detect networks
    let networks = detect_networks(&docker).await?;
    info!("   ✓ Found {} Docker networks", networks.len());

    // Detect shared resources
    let shared_resources = detect_shared_resources(&docker).await?;
    if shared_resources.postgres.is_some() {
        info!("   ✓ Found shared PostgreSQL");
    }
    if shared_resources.redis.is_some() {
        info!("   ✓ Found shared Redis");
    }

    // Detect deployment directories
    let deploy_directories = detect_deploy_directories().await?;
    let is_multi_project = deploy_directories.len() > 1;

    if is_multi_project {
        info!("   ✓ Multi-project setup detected ({} projects)", deploy_directories.len());
    }

    Ok(InfrastructureInfo {
        proxy,
        networks,
        shared_resources,
        deploy_directories,
        is_multi_project,
    })
}

/// Detect reverse proxy (Caddy, Nginx, Traefik)
async fn detect_proxy(docker: &Docker) -> Result<Option<(String, String)>> {
    let mut filters = HashMap::new();
    filters.insert("status", vec!["running"]);

    let options = Some(ListContainersOptions {
        filters,
        ..Default::default()
    });

    let containers = docker.list_containers(options).await?;

    for container in containers {
        let name = container.names
            .and_then(|names| names.first().map(|n| n.trim_start_matches('/').to_string()))
            .unwrap_or_default();

        let image = container.image.unwrap_or_default().to_lowercase();

        // Check for Caddy
        if image.contains("caddy") || name.contains("caddy") {
            return Ok(Some(("caddy".to_string(), name)));
        }

        // Check for Nginx
        if image.contains("nginx") && (name.contains("proxy") || name.contains("nginx")) {
            return Ok(Some(("nginx".to_string(), name)));
        }

        // Check for Traefik
        if image.contains("traefik") {
            return Ok(Some(("traefik".to_string(), name)));
        }
    }

    Ok(None)
}

/// Detect Docker networks
async fn detect_networks(docker: &Docker) -> Result<Vec<NetworkInfo>> {
    let options = Some(ListNetworksOptions::<String> {
        ..Default::default()
    });

    let networks = docker.list_networks(options).await?;

    Ok(networks
        .into_iter()
        .filter_map(|net| {
            Some(NetworkInfo {
                name: net.name?,
                id: net.id?,
                driver: net.driver?,
                scope: net.scope?,
            })
        })
        .collect())
}

/// Detect shared resources (databases, caches, etc.)
async fn detect_shared_resources(docker: &Docker) -> Result<SharedResources> {
    let mut filters = HashMap::new();
    filters.insert("status", vec!["running"]);

    let options = Some(ListContainersOptions {
        filters,
        ..Default::default()
    });

    let containers = docker.list_containers(options).await?;

    let mut shared = SharedResources::default();

    for container in containers {
        let name = container.names
            .and_then(|names| names.first().map(|n| n.trim_start_matches('/').to_string()))
            .unwrap_or_default();

        let image = container.image.unwrap_or_default();
        let networks = container.network_settings
            .and_then(|ns| ns.networks)
            .map(|nets| nets.keys().cloned().collect())
            .unwrap_or_default();

        let ports: Vec<u16> = container.ports
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.private_port)
            .collect();

        let service_info = ServiceInfo {
            container_name: name.clone(),
            image: image.clone(),
            networks,
            ports,
        };

        // Check for PostgreSQL
        if name.contains("postgres") || image.contains("postgres") {
            // Prefer "shared" postgres over project-specific ones
            if name.contains("shared") || shared.postgres.is_none() {
                debug!("Found PostgreSQL: {}", name);
                shared.postgres = Some(service_info.clone());
            }
        }

        // Check for Redis
        if name.contains("redis") || image.contains("redis") {
            if name.contains("shared") || shared.redis.is_none() {
                debug!("Found Redis: {}", name);
                shared.redis = Some(service_info.clone());
            }
        }

        // Check for RabbitMQ
        if name.contains("rabbitmq") || image.contains("rabbitmq") {
            debug!("Found RabbitMQ: {}", name);
            shared.rabbitmq = Some(service_info.clone());
        }
    }

    Ok(shared)
}

/// Detect deployment directories (e.g., ~/apps/*)
async fn detect_deploy_directories() -> Result<Vec<String>> {
    let mut possible_dirs = vec![
        "/opt/apps".to_string(),
        "/var/www".to_string(),
    ];

    // Check all user home directories for apps/projects folders
    if let Ok(entries) = std::fs::read_dir("/home") {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    let user_home = entry.path();
                    possible_dirs.push(format!("{}/apps", user_home.display()));
                    possible_dirs.push(format!("{}/projects", user_home.display()));
                }
            }
        }
    }

    // Also check current user's home
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    possible_dirs.push(format!("{}/apps", home));
    possible_dirs.push(format!("{}/projects", home));

    let mut found_dirs = Vec::new();

    for dir in possible_dirs {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let project_count = entries.count();
            if project_count > 0 {
                found_dirs.push(dir);
            }
        }
    }

    Ok(found_dirs)
}

/// Recommend deployment strategy based on detected infrastructure
pub fn recommend_strategy(info: &InfrastructureInfo) -> String {
    if info.is_multi_project {
        // Multi-project setup: use compose with existing structure
        "compose".to_string()
    } else if info.proxy.is_some() {
        // Has proxy but single project: still use compose for consistency
        "compose".to_string()
    } else {
        // Simple setup: standalone containers
        "standalone".to_string()
    }
}

/// Get recommended deployment directory
pub fn recommend_deploy_dir(info: &InfrastructureInfo, project_name: &str) -> String {
    // First check if project already exists with a config file
    for base_dir in &info.deploy_directories {
        let potential_dir = format!("{}/{}", base_dir, project_name);
        let config_path = format!("{}/.shipwright.yml", potential_dir);
        if std::path::Path::new(&config_path).exists() {
            debug!("Found existing project with config at: {}", potential_dir);
            return potential_dir;
        }
    }

    // Otherwise use first available directory
    if let Some(base_dir) = info.deploy_directories.first() {
        format!("{}/{}", base_dir, project_name)
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{}/apps/{}", home, project_name)
    }
}

/// Get networks to join based on detected infrastructure
pub fn recommend_networks(info: &InfrastructureInfo) -> Vec<String> {
    let mut networks = Vec::new();

    // Add proxy network if exists
    if info.proxy.is_some() {
        // Look for common proxy network names
        for net in &info.networks {
            if net.name.contains("proxy") || net.name == "proxy-tier" {
                networks.push(net.name.clone());
                break;
            }
        }
    }

    // Add shared resources network if exists
    if info.shared_resources.postgres.is_some() || info.shared_resources.redis.is_some() {
        for net in &info.networks {
            if net.name.contains("shared") || net.name.contains("internal") {
                if !networks.contains(&net.name) {
                    networks.push(net.name.clone());
                }
                break;
            }
        }
    }

    networks
}

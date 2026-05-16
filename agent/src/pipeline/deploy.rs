use anyhow::{Result, Context};
use tokio::process::Command;
use tracing::{info, warn};
use std::path::Path;
use crate::infrastructure::{InfrastructureInfo, detect_infrastructure};
use crate::infrastructure::adapters::{RouteConfig, create_adapter};
use shipwright_common::config::Config;

#[derive(Debug, Clone)]
pub enum DeployStrategy {
    /// Standalone containers (simple mode)
    Standalone,
    /// Docker Compose deployment
    Compose { file: String },
    /// Hybrid: build with compose, deploy individually
    Hybrid,
}

#[derive(Clone)]
pub struct DeploymentContext {
    pub project_name: String,
    pub build_dir: String,
    pub strategy: DeployStrategy,
    pub infrastructure: InfrastructureInfo,
}

impl DeploymentContext {
    pub async fn new(project_name: &str, build_dir: &str, config: Option<&Config>) -> Result<Self> {
        // Detect existing infrastructure
        let infrastructure = detect_infrastructure().await?;

        // Determine strategy
        let strategy = determine_strategy(&infrastructure, build_dir, config).await?;

        Ok(Self {
            project_name: project_name.to_string(),
            build_dir: build_dir.to_string(),
            strategy,
            infrastructure,
        })
    }

    /// Execute deployment based on strategy
    pub async fn deploy(&self, config: Option<&Config>) -> Result<()> {
        use crate::smoke_tests::{SmokeTestRunner, SmokeTestConfig, TestCategory};

        // Configure smoke tests
        let smoke_config = if let Some(cfg) = config {
            if let Some(st_config) = &cfg.smoke_tests {
                SmokeTestConfig {
                    enabled: st_config.enabled,
                    fail_on_error: st_config.fail_on_error,
                    categories: st_config.categories.iter().map(|c| match c.as_str() {
                        "pre_deployment" => TestCategory::PreDeployment,
                        "post_build" => TestCategory::PostBuild,
                        "post_deployment" => TestCategory::PostDeployment,
                        "integration" => TestCategory::Integration,
                        _ => TestCategory::PostDeployment,
                    }).collect(),
                    disabled_tests: st_config.disabled_tests.clone(),
                }
            } else {
                SmokeTestConfig::default()
            }
        } else {
            SmokeTestConfig::default()
        };

        // Run pre-deployment smoke tests
        if smoke_config.enabled {
            info!("🧪 Running pre-deployment smoke tests...");
            let mut test_runner = SmokeTestRunner::new(self.clone(), smoke_config.clone());
            test_runner.run_category(TestCategory::PreDeployment).await?;
        }

        // Execute deployment
        match &self.strategy {
            DeployStrategy::Standalone => self.deploy_standalone().await?,
            DeployStrategy::Compose { file } => self.deploy_compose(file).await?,
            DeployStrategy::Hybrid => self.deploy_hybrid().await?,
        }

        // Update proxy configuration if needed
        if let Some(cfg) = config {
            self.update_proxy_config(cfg).await?;
        }

        // Run post-deployment smoke tests
        if smoke_config.enabled {
            info!("🧪 Running post-deployment smoke tests...");
            let mut test_runner = SmokeTestRunner::new(self.clone(), smoke_config.clone());
            test_runner.run_category(TestCategory::PostDeployment).await?;

            let report = test_runner.generate_report();
            info!("\n{}", report);

            if report.has_critical_failures() && smoke_config.fail_on_error {
                return Err(anyhow::anyhow!("🚨 Deployment failed smoke tests - {} critical failure(s)", report.critical_failures));
            }

            if report.warnings > 0 {
                warn!("⚠️  Deployment completed with {} warning(s) - review logs", report.warnings);
            }
        }

        Ok(())
    }

    /// Deploy using standalone containers
    async fn deploy_standalone(&self) -> Result<()> {
        info!("📦 Deploying in standalone mode...");

        let docker = bollard::Docker::connect_with_socket_defaults()?;
        let container_name = format!("shipwright-{}", self.project_name);
        let image_name = format!("{}:latest", self.project_name);

        // Stop and remove existing container
        use bollard::container::{StopContainerOptions, RemoveContainerOptions};
        let _ = docker.stop_container(&container_name, Some(StopContainerOptions { t: 10 })).await;
        let _ = docker.remove_container(&container_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;

        // Determine networks to join
        let networks = crate::infrastructure::detector::recommend_networks(&self.infrastructure);

        // Create container with proper network configuration
        use bollard::container::{Config as ContainerConfig, CreateContainerOptions, NetworkingConfig};
        use bollard::models::HostConfig;
        use std::collections::HashMap;

        let mut endpoints_config = HashMap::new();
        if let Some(first_network) = networks.first() {
            endpoints_config.insert(
                first_network.clone(),
                bollard::models::EndpointSettings::default(),
            );
        }

        let config = ContainerConfig {
            image: Some(image_name),
            hostname: Some(self.project_name.clone()),
            networking_config: Some(NetworkingConfig {
                endpoints_config,
            }),
            ..Default::default()
        };

        docker.create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        ).await?;

        // Connect to additional networks
        for network in networks.iter().skip(1) {
            use bollard::network::ConnectNetworkOptions;
            docker.connect_network(
                network,
                ConnectNetworkOptions {
                    container: container_name.clone(),
                    ..Default::default()
                },
            ).await?;
        }

        // Start container
        use bollard::container::StartContainerOptions;
        docker.start_container(&container_name, None::<StartContainerOptions<String>>).await?;

        info!("✅ Standalone deployment complete: {}", container_name);
        Ok(())
    }

    /// Deploy using docker-compose
    async fn deploy_compose(&self, compose_file: &str) -> Result<()> {
        info!("📦 Deploying with docker-compose: {}", compose_file);

        let output = Command::new("docker-compose")
            .arg("-f")
            .arg(compose_file)
            .arg("up")
            .arg("-d")
            .arg("--remove-orphans")
            .current_dir(&self.build_dir)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "Docker Compose deployment failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        info!("✅ Docker Compose deployment complete");
        Ok(())
    }

    /// Hybrid deployment (build with compose, deploy individually)
    async fn deploy_hybrid(&self) -> Result<()> {
        warn!("Hybrid mode not yet implemented, falling back to compose");
        self.deploy_compose("docker-compose.yml").await
    }

    /// Update reverse proxy configuration
    async fn update_proxy_config(&self, config: &Config) -> Result<()> {
        // Check if proxy integration is enabled
        let proxy_info = match &self.infrastructure.proxy {
            Some(p) => p,
            None => {
                info!("No proxy detected, skipping proxy configuration");
                return Ok(());
            }
        };

        let (proxy_type, container_name) = proxy_info;

        // Check if auto-update is enabled in config
        let auto_update = config.infrastructure.as_ref()
            .and_then(|i| i.proxy.as_ref())
            .map(|p| p.auto_update)
            .unwrap_or(true);

        if !auto_update {
            info!("Proxy auto-update disabled, skipping");
            return Ok(());
        }

        info!("🔧 Updating {} proxy configuration...", proxy_type);

        let adapter = create_adapter(proxy_type, container_name.clone());

        // Get service configurations
        let services = config.deploy.vps.as_ref()
            .map(|v| &v.services)
            .context("No VPS config found")?;

        for service in services.iter() {
            if !service.expose {
                continue;
            }

            let domain = service.domain.as_ref()
                .or(config.deploy.vps.as_ref().and_then(|v| v.domain.as_ref()))
                .context("No domain specified for service")?;

            let route_config = RouteConfig {
                domain: domain.clone(),
                service_name: service.name.clone(),
                port: service.port,
                path: service.path.clone(),
                enable_cors: false, // TODO: Make this configurable
                enable_tls: config.deploy.vps.as_ref()
                    .and_then(|v| v.acme_email.as_ref())
                    .is_some(),
            };

            adapter.add_route(route_config).await?;
        }

        Ok(())
    }
}

/// Determine appropriate deployment strategy
async fn determine_strategy(
    infrastructure: &InfrastructureInfo,
    build_dir: &str,
    config: Option<&Config>,
) -> Result<DeployStrategy> {
    // Check config for explicit strategy
    if let Some(cfg) = config {
        if let Some(infra) = &cfg.infrastructure {
            match infra.strategy.as_str() {
                "standalone" => return Ok(DeployStrategy::Standalone),
                "hybrid" => return Ok(DeployStrategy::Hybrid),
                "compose" | "docker-compose" => {
                    // Find compose file
                    let compose_file = find_compose_file(build_dir, &cfg.build.compose_file)?;
                    return Ok(DeployStrategy::Compose { file: compose_file });
                }
                _ => {} // Fall through to auto-detection
            }
        }
    }

    // Auto-detect based on infrastructure
    if infrastructure.is_multi_project {
        // Multi-project setup: prefer compose
        if let Ok(compose_file) = find_compose_file(build_dir, &None) {
            return Ok(DeployStrategy::Compose { file: compose_file });
        }
    }

    // Check if compose file exists
    if let Ok(compose_file) = find_compose_file(build_dir, &None) {
        return Ok(DeployStrategy::Compose { file: compose_file });
    }

    // Default to standalone
    Ok(DeployStrategy::Standalone)
}

/// Find appropriate docker-compose file
fn find_compose_file(build_dir: &str, preferred: &Option<String>) -> Result<String> {
    // Check preferred file first
    if let Some(file) = preferred {
        let path = Path::new(build_dir).join(file);
        if path.exists() {
            return Ok(file.clone());
        }
    }

    // Check common compose file names in order of preference
    // Including subdirectories like infra/, deploy/, etc.
    let candidates = [
        "docker-compose.deploy.yml",
        "docker-compose.vps.yml",
        "docker-compose.production.yml",
        "docker-compose.prod.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
        "infra/docker-compose.deploy.yml",
        "infra/docker-compose.yml",
        "deploy/docker-compose.yml",
        ".docker/docker-compose.yml",
    ];

    for candidate in candidates {
        let path = Path::new(build_dir).join(candidate);
        if path.exists() {
            return Ok(candidate.to_string());
        }
    }

    anyhow::bail!("No docker-compose file found in {}", build_dir)
}

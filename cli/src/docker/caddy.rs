use anyhow::{Result, Context};
use dialoguer::{Input, Select};
use regex::Regex;
use shipwright_common::config::{VpsConfig, ServiceConfig};

/// Detected service that may need domain configuration
#[derive(Debug, Clone)]
pub struct DetectedService {
    pub name: String,
    pub container_name: Option<String>,
    pub ports: Vec<u16>,
    pub is_frontend: bool,
}

/// Detect services from docker-compose content that might need reverse proxy
pub fn detect_exposed_services(compose_content: &str) -> Vec<DetectedService> {
    let mut services = Vec::new();
    let mut current_service = String::new();
    let mut current_container_name: Option<String> = None;
    let mut current_ports: Vec<u16> = Vec::new();
    let mut in_services = false;
    let mut in_ports = false;

    for line in compose_content.lines() {
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();

        // Track services section
        if trimmed == "services:" {
            in_services = true;
            continue;
        }

        if !in_services {
            continue;
        }

        // Detect service name (indent level 2)
        if indent == 2 && trimmed.ends_with(':') && !trimmed.contains(' ') {
            // Save previous service if it has exposed ports
            if !current_service.is_empty() && !current_ports.is_empty() {
                let is_frontend = current_service.contains("frontend")
                    || current_ports.contains(&80)
                    || current_ports.contains(&443)
                    || current_ports.contains(&3000);

                services.push(DetectedService {
                    name: current_service.clone(),
                    container_name: current_container_name.clone(),
                    ports: current_ports.clone(),
                    is_frontend,
                });
            }

            current_service = trimmed.trim_end_matches(':').to_string();
            current_container_name = None;
            current_ports = Vec::new();
            in_ports = false;
            continue;
        }

        // Track container_name
        if trimmed.starts_with("container_name:") {
            if let Some(name) = trimmed.strip_prefix("container_name:") {
                current_container_name = Some(name.trim().to_string());
            }
        }

        // Track ports section
        if trimmed == "ports:" {
            in_ports = true;
            continue;
        }

        // Parse port mappings
        if in_ports && trimmed.starts_with('-') {
            let port_str = trimmed.trim_start_matches('-').trim().trim_matches('"');
            // Parse "HOST:CONTAINER" or just "PORT"
            if let Some(container_port) = parse_container_port(port_str) {
                current_ports.push(container_port);
            }
        }

        // Exit ports section on non-list item
        if in_ports && !trimmed.starts_with('-') && !trimmed.is_empty() {
            in_ports = false;
        }

        // Track expose section (internal ports)
        if trimmed.starts_with("expose:") {
            in_ports = true;
            continue;
        }
    }

    // Don't forget the last service
    if !current_service.is_empty() && !current_ports.is_empty() {
        let is_frontend = current_service.contains("frontend")
            || current_ports.contains(&80)
            || current_ports.contains(&443)
            || current_ports.contains(&3000);

        services.push(DetectedService {
            name: current_service,
            container_name: current_container_name,
            ports: current_ports,
            is_frontend,
        });
    }

    services
}

/// Parse container port from port mapping string
fn parse_container_port(port_str: &str) -> Option<u16> {
    // Handle formats: "8080:80", "80", "${PREFIX}80:80"
    let port_str = port_str.trim();

    if port_str.contains(':') {
        // Get the container port (after the colon)
        let parts: Vec<&str> = port_str.split(':').collect();
        if parts.len() >= 2 {
            // Remove any variable substitution like ${PORT_PREFIX:-70}
            let container_part = parts.last().unwrap();
            let re = Regex::new(r"\d+").ok()?;
            if let Some(m) = re.find(container_part) {
                return m.as_str().parse().ok();
            }
        }
    } else {
        // Just a port number
        let re = Regex::new(r"\d+").ok()?;
        if let Some(m) = re.find(port_str) {
            return m.as_str().parse().ok();
        }
    }

    None
}

/// Prompt user for domain configuration for detected services
pub fn prompt_for_domains(
    services: &[DetectedService],
    vps_host: &str,
    existing_config: &[ServiceConfig],
) -> Result<Vec<ServiceConfig>> {
    let mut configs = Vec::new();

    // Filter to services that might need reverse proxy (frontends and APIs)
    let exposable: Vec<&DetectedService> = services.iter()
        .filter(|s| s.is_frontend || s.name.contains("backend") || s.name.contains("api"))
        .collect();

    if exposable.is_empty() {
        println!("No services detected that need reverse proxy configuration.");
        return Ok(configs);
    }

    println!("\n🌐 Domain Configuration");
    println!("========================");
    println!("Configure domains for your services. Caddy will handle HTTPS automatically.\n");

    for service in exposable {
        // Check if already configured
        if let Some(existing) = existing_config.iter().find(|c| c.name == service.name) {
            println!("✓ {} already configured: {}", service.name,
                existing.domain.as_deref().unwrap_or("(no domain)"));
            configs.push(existing.clone());
            continue;
        }

        let default_port = service.ports.first().copied().unwrap_or(80);

        println!("\n📦 Service: {} (port {})", service.name, default_port);

        let wildcard_option = format!(
            "Use wildcard DNS ({}.{}.nip.io)",
            service.name.replace("-", ""),
            vps_host
        );
        let ip_option = format!("Expose via IP only (http://{}:PORT)", vps_host);

        let options = vec![
            "Enter custom domain (e.g., app.example.com)",
            &wildcard_option,
            &ip_option,
            "Skip - don't expose this service",
        ];

        let selection = Select::new()
            .with_prompt("How should this service be accessed?")
            .items(&options)
            .default(0)
            .interact()?;

        let (domain, expose) = match selection {
            0 => {
                // Custom domain
                let domain: String = Input::new()
                    .with_prompt("Enter domain")
                    .interact_text()?;
                (Some(domain), true)
            }
            1 => {
                // nip.io wildcard
                let subdomain = service.name.replace("-", "");
                (Some(format!("{}.{}.nip.io", subdomain, vps_host)), true)
            }
            2 => {
                // IP only - no Caddy needed
                (None, false)
            }
            3 => {
                // Skip
                (None, false)
            }
            _ => (None, false),
        };

        configs.push(ServiceConfig {
            name: service.name.clone(),
            domain,
            port: default_port,
            path: None,
            expose,
        });
    }

    Ok(configs)
}

/// Check for domain conflicts across all projects on the VPS
pub fn check_domain_conflicts(
    vps: &VpsConfig,
    new_domains: &[ServiceConfig],
    project_name: &str,
) -> Result<Vec<String>> {
    let mut conflicts = Vec::new();

    // Read existing caddy configs from VPS
    let check_cmd = "ls /etc/caddy/sites/*.caddy 2>/dev/null | xargs -I {} sh -c 'echo \"=== {} ===\"  && cat {}'";

    let output = super::deploy::execute_remote_command(vps, check_cmd);

    if let Ok(existing_configs) = output {
        // Parse domains from existing configs
        let domain_re = Regex::new(r"^([a-zA-Z0-9][a-zA-Z0-9\-\.]+)\s*\{").unwrap();

        for new_service in new_domains {
            if let Some(domain) = &new_service.domain {
                // Check if this domain exists in another project's config
                for line in existing_configs.lines() {
                    if line.contains(&format!("{}.caddy", project_name)) {
                        // Skip our own project
                        continue;
                    }

                    if let Some(cap) = domain_re.captures(line) {
                        let existing_domain = &cap[1];
                        if existing_domain == domain {
                            conflicts.push(format!(
                                "Domain '{}' is already configured for another project",
                                domain
                            ));
                        }
                    }
                }
            }
        }
    }

    Ok(conflicts)
}

/// Generate Caddyfile content for a project
pub fn generate_caddyfile(
    project_name: &str,
    services: &[ServiceConfig],
    container_prefix: &str,
) -> String {
    let mut content = format!(
        "# Managed by Shipwright - Project: {}\n# Do not edit manually - changes will be overwritten\n\n",
        project_name
    );

    for service in services {
        if !service.expose {
            continue;
        }

        if let Some(domain) = &service.domain {
            let container_name = format!("{}-{}", container_prefix, service.name);

            content.push_str(&format!("{} {{\n", domain));

            if let Some(path) = &service.path {
                // Path-based routing
                content.push_str(&format!(
                    "    handle {} {{\n        reverse_proxy {}:{}\n    }}\n",
                    path, container_name, service.port
                ));
            } else {
                // Simple reverse proxy
                content.push_str(&format!(
                    "    reverse_proxy {}:{}\n",
                    container_name, service.port
                ));
            }

            content.push_str("}\n\n");
        }
    }

    content
}

/// Setup Caddy on the VPS with modular configuration
pub fn setup_caddy_infrastructure(vps: &VpsConfig) -> Result<()> {
    println!("\n🔧 Setting up Caddy infrastructure...");

    // Step 1: Create Caddy sites directory
    super::deploy::execute_sudo_command(vps, "mkdir -p /etc/caddy/sites")
        .context("Failed to create Caddy sites directory")?;

    // Step 2: Create main Caddyfile if it doesn't exist
    let caddyfile_check = super::deploy::execute_remote_command(vps, "test -f /etc/caddy/Caddyfile && echo exists || echo missing");
    if caddyfile_check.map(|s| s.trim() == "missing").unwrap_or(true) {
        let caddyfile_content = r#"{
    # Global options
    email admin@localhost
}

# Import all site configurations
import sites/*
"#;
        let create_cmd = format!("cat > /etc/caddy/Caddyfile << 'CADDY_EOF'\n{}\nCADDY_EOF", caddyfile_content);
        super::deploy::execute_sudo_command(vps, &create_cmd)
            .context("Failed to create Caddyfile")?;
    }

    // Step 3: Check if Caddy is installed
    let caddy_check = super::deploy::execute_remote_command(vps, "command -v caddy >/dev/null 2>&1 && echo installed || echo missing");
    if caddy_check.map(|s| s.trim() == "missing").unwrap_or(true) {
        println!("   Installing Caddy...");

        // Install Caddy step by step with non-interactive flags
        super::deploy::execute_sudo_command(vps, "apt-get update -qq")?;
        super::deploy::execute_sudo_command(vps, "DEBIAN_FRONTEND=noninteractive apt-get install -y -qq debian-keyring debian-archive-keyring apt-transport-https curl")?;

        // Add Caddy repository
        super::deploy::execute_sudo_command(vps,
            "curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg --yes"
        )?;

        super::deploy::execute_sudo_command(vps,
            "curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-stable.list > /dev/null"
        )?;

        super::deploy::execute_sudo_command(vps, "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq caddy")?;
    }

    // Step 4: Enable and start Caddy
    super::deploy::execute_sudo_command(vps, "systemctl enable caddy && systemctl start caddy")
        .context("Failed to start Caddy")?;

    println!("   ✓ Caddy infrastructure ready");
    Ok(())
}

/// Deploy Caddy configuration for a project
pub fn deploy_caddy_config(
    vps: &VpsConfig,
    project_name: &str,
    caddyfile_content: &str,
    acme_email: Option<&str>,
) -> Result<()> {
    if caddyfile_content.trim().lines().count() <= 3 {
        // Only comments, no actual config
        println!("   ℹ No services configured for reverse proxy");
        return Ok(());
    }

    println!("\n🌐 Deploying Caddy configuration...");

    // Write the project's Caddyfile using heredoc to avoid quoting issues
    let deploy_cmd = format!(
        "cat > /etc/caddy/sites/{}.caddy << 'CADDY_EOF'\n{}\nCADDY_EOF",
        project_name, caddyfile_content
    );

    super::deploy::execute_sudo_command(vps, &deploy_cmd)
        .context("Failed to write Caddyfile")?;

    // Update ACME email if provided
    if let Some(email) = acme_email {
        let email_cmd = format!(
            "sed -i 's/email .*/email {}/' /etc/caddy/Caddyfile",
            email
        );
        let _ = super::deploy::execute_sudo_command(vps, &email_cmd);
    }

    // Reload Caddy
    super::deploy::execute_sudo_command(vps, "systemctl reload caddy")
        .context("Failed to reload Caddy")?;

    println!("   ✓ Caddy configuration deployed");

    // List configured domains
    println!("\n   Configured domains:");
    for line in caddyfile_content.lines() {
        if line.ends_with('{') && !line.starts_with('#') {
            let domain = line.trim().trim_end_matches('{').trim();
            if !domain.is_empty() {
                println!("   • https://{}", domain);
            }
        }
    }

    Ok(())
}

/// Create shared Docker network for Caddy to reach containers
pub fn ensure_caddy_network(vps: &VpsConfig) -> Result<()> {
    let cmd = "docker network create caddy-proxy 2>/dev/null || true";
    super::deploy::execute_remote_command(vps, cmd)?;
    Ok(())
}

/// Remove Caddy configuration for a project
pub fn remove_caddy_config(vps: &VpsConfig, project_name: &str) -> Result<()> {
    let rm_cmd = format!("rm -f /etc/caddy/sites/{}.caddy", project_name);
    super::deploy::execute_sudo_command(vps, &rm_cmd)?;
    super::deploy::execute_sudo_command(vps, "systemctl reload caddy")?;
    Ok(())
}

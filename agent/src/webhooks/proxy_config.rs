/// Automatic reverse proxy configuration for HTTPS webhook routing
///
/// This module detects existing reverse proxies (Caddy, Nginx) and automatically
/// configures routes to enable HTTPS webhooks without manual setup.

use anyhow::{Result, Context};
use tracing::{info, warn, error};
use bollard::Docker;

/// Configure reverse proxy for HTTPS webhook routing
pub async fn configure_webhook_routing(domain: &str) -> Result<()> {
    info!("🔧 Configuring reverse proxy for HTTPS webhooks...");

    // Detect which proxy is running
    let proxy_type = detect_proxy().await?;

    match proxy_type {
        ProxyType::Caddy(container) => configure_caddy(&container, domain).await,
        ProxyType::Nginx(container) => configure_nginx(&container, domain).await,
        ProxyType::None => {
            warn!("No reverse proxy detected. Webhooks will only work over HTTP.");
            warn!("For production, set up Caddy or Nginx for automatic HTTPS.");
            Ok(())
        }
    }
}

#[derive(Debug)]
enum ProxyType {
    Caddy(String),
    Nginx(String),
    None,
}

/// Detect which reverse proxy is running
async fn detect_proxy() -> Result<ProxyType> {
    let docker = Docker::connect_with_socket_defaults()?;

    use bollard::container::ListContainersOptions;
    use std::collections::HashMap;

    let mut filters = HashMap::new();
    filters.insert("status", vec!["running"]);

    let containers = docker.list_containers(Some(ListContainersOptions {
        all: false,
        filters,
        ..Default::default()
    })).await?;

    // Check for Caddy
    for container in &containers {
        if let Some(image) = &container.image {
            if image.contains("caddy") {
                if let Some(names) = &container.names {
                    if let Some(name) = names.first() {
                        info!("✓ Detected Caddy proxy: {}", name);
                        return Ok(ProxyType::Caddy(name.trim_start_matches('/').to_string()));
                    }
                }
            }
        }
    }

    // Check for Nginx
    for container in &containers {
        if let Some(image) = &container.image {
            if image.contains("nginx") {
                if let Some(names) = &container.names {
                    if let Some(name) = names.first() {
                        info!("✓ Detected Nginx proxy: {}", name);
                        return Ok(ProxyType::Nginx(name.trim_start_matches('/').to_string()));
                    }
                }
            }
        }
    }

    Ok(ProxyType::None)
}

/// Configure Caddy for webhook routing
async fn configure_caddy(container: &str, domain: &str) -> Result<()> {
    info!("Configuring Caddy for HTTPS webhooks at {}", domain);

    // Caddy configuration for webhook routing
    let caddy_config = format!(
        r#"
# Shipwright webhook routing (auto-generated)
{domain} {{
    # Webhook endpoints
    handle /shipwright/webhooks/* {{
        reverse_proxy shipwright-agent:8084
    }}

    # Health check
    handle /shipwright/health {{
        reverse_proxy shipwright-agent:8084
    }}

    # Fallback for other routes
    handle {{
        respond "Shipwright Agent" 200
    }}
}}
"#
    );

    // Write configuration to Caddyfile
    let config_path = format!("/etc/caddy/Caddyfile.d/shipwright-webhooks.conf");

    // Try to add config via docker exec
    let docker = Docker::connect_with_socket_defaults()?;

    use bollard::exec::CreateExecOptions;

    // Create config directory if not exists
    let create_dir_exec = docker.create_exec(
        container,
        CreateExecOptions {
            cmd: Some(vec!["sh", "-c", "mkdir -p /etc/caddy/Caddyfile.d"]),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        },
    ).await?;

    let _start = docker.start_exec(&create_dir_exec.id, None).await?;

    // Write config file
    let write_config_cmd = format!(
        "cat > {} << 'EOF'\n{}\nEOF",
        config_path, caddy_config
    );

    let write_exec = docker.create_exec(
        container,
        CreateExecOptions {
            cmd: Some(vec!["sh", "-c", &write_config_cmd]),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        },
    ).await?;

    let _start = docker.start_exec(&write_exec.id, None).await?;

    // Reload Caddy configuration
    info!("Reloading Caddy configuration...");

    let reload_exec = docker.create_exec(
        container,
        CreateExecOptions {
            cmd: Some(vec!["caddy", "reload", "--config", "/etc/caddy/Caddyfile"]),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        },
    ).await?;

    let _start = docker.start_exec(&reload_exec.id, None).await?;

    info!("✅ Caddy configured for HTTPS webhooks");
    info!("Webhook URL: https://{}/shipwright/webhooks/github", domain);

    Ok(())
}

/// Configure Nginx for webhook routing
async fn configure_nginx(container: &str, domain: &str) -> Result<()> {
    info!("Configuring Nginx for HTTPS webhooks at {}", domain);

    // Nginx configuration for webhook routing
    let nginx_config = format!(
        r#"
# Shipwright webhook routing (auto-generated)
server {{
    listen 80;
    listen [::]:80;
    server_name {domain};

    # Webhook endpoints
    location /shipwright/webhooks/ {{
        proxy_pass http://shipwright-agent:8084/webhooks/;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }}

    # Health check
    location /shipwright/health {{
        proxy_pass http://shipwright-agent:8084/health;
        proxy_set_header Host $host;
    }}
}}
"#
    );

    // Write configuration
    let config_path = "/etc/nginx/conf.d/shipwright-webhooks.conf";

    let docker = Docker::connect_with_socket_defaults()?;

    use bollard::exec::CreateExecOptions;

    // Write config file
    let write_config_cmd = format!(
        "cat > {} << 'EOF'\n{}\nEOF",
        config_path, nginx_config
    );

    let write_exec = docker.create_exec(
        container,
        CreateExecOptions {
            cmd: Some(vec!["sh", "-c", &write_config_cmd]),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        },
    ).await?;

    let _start = docker.start_exec(&write_exec.id, None).await?;

    // Reload Nginx configuration
    info!("Reloading Nginx configuration...");

    let reload_exec = docker.create_exec(
        container,
        CreateExecOptions {
            cmd: Some(vec!["nginx", "-s", "reload"]),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        },
    ).await?;

    let _start = docker.start_exec(&reload_exec.id, None).await?;

    info!("✅ Nginx configured for HTTPS webhooks");
    info!("Webhook URL: https://{}/shipwright/webhooks/github", domain);

    Ok(())
}

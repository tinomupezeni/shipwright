use anyhow::Result;
use async_trait::async_trait;
use tokio::process::Command;
use tracing::{info, warn, error};

/// Trait for reverse proxy adapters
#[async_trait]
pub trait ProxyAdapter: Send + Sync {
    /// Add routing for a new service
    async fn add_route(&self, config: RouteConfig) -> Result<()>;

    /// Remove routing for a service
    async fn remove_route(&self, domain: &str) -> Result<()>;

    /// Reload proxy configuration
    async fn reload(&self) -> Result<()>;

    /// Check if proxy is healthy
    async fn health_check(&self) -> Result<bool>;
}

#[derive(Debug, Clone)]
pub struct RouteConfig {
    pub domain: String,
    pub service_name: String,
    pub port: u16,
    pub path: Option<String>,
    pub enable_cors: bool,
    pub enable_tls: bool,
}

/// Caddy proxy adapter
pub struct CaddyAdapter {
    container_name: String,
    config_path: String,
}

impl CaddyAdapter {
    pub fn new(container_name: String) -> Self {
        Self {
            container_name,
            config_path: "/etc/caddy/Caddyfile".to_string(),
        }
    }

    async fn read_caddyfile(&self) -> Result<String> {
        let output = Command::new("docker")
            .args(["exec", &self.container_name, "cat", &self.config_path])
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!("Failed to read Caddyfile: {}", String::from_utf8_lossy(&output.stderr));
        }

        Ok(String::from_utf8(output.stdout)?)
    }

    async fn write_caddyfile(&self, content: &str) -> Result<()> {
        // Write to temp file
        let temp_path = "/tmp/Caddyfile.shipwright";
        tokio::fs::write(temp_path, content).await?;

        // Copy to container
        let output = Command::new("docker")
            .args(["cp", temp_path, &format!("{}:{}", self.container_name, self.config_path)])
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!("Failed to update Caddyfile: {}", String::from_utf8_lossy(&output.stderr));
        }

        Ok(())
    }

    fn generate_caddy_block(&self, config: &RouteConfig) -> String {
        let mut block = format!("\n# --- {} (Shipwright) ---\n", config.service_name);
        block.push_str(&format!("{} {{\n", config.domain));
        block.push_str("    import security\n");

        if config.enable_cors {
            block.push_str("    import cors_handle\n");
        }

        if let Some(path) = &config.path {
            block.push_str(&format!("    handle {}/* {{\n", path));
            block.push_str(&format!("        reverse_proxy {}:{}\n", config.service_name, config.port));
            block.push_str("    }\n");
        } else {
            block.push_str(&format!("    reverse_proxy {}:{}\n", config.service_name, config.port));
        }

        block.push_str("}\n");
        block
    }
}

#[async_trait]
impl ProxyAdapter for CaddyAdapter {
    async fn add_route(&self, config: RouteConfig) -> Result<()> {
        info!("📝 Adding Caddy route for {} -> {}:{}", config.domain, config.service_name, config.port);

        let mut caddyfile = self.read_caddyfile().await?;

        // Check if route already exists
        if caddyfile.contains(&format!("# --- {} (Shipwright) ---", config.service_name)) {
            warn!("Route for {} already exists, skipping", config.service_name);
            return Ok(());
        }

        // Append new block
        let new_block = self.generate_caddy_block(&config);
        caddyfile.push_str(&new_block);

        // Write back
        self.write_caddyfile(&caddyfile).await?;

        // Reload
        self.reload().await?;

        info!("✅ Caddy route added successfully");
        Ok(())
    }

    async fn remove_route(&self, domain: &str) -> Result<()> {
        info!("🗑️  Removing Caddy route for {}", domain);

        let caddyfile = self.read_caddyfile().await?;

        // Find and remove the block
        let lines: Vec<&str> = caddyfile.lines().collect();
        let mut new_lines = Vec::new();
        let mut skip_block = false;
        let mut brace_count = 0;

        for line in lines {
            if line.contains("(Shipwright)") {
                skip_block = true;
                continue;
            }

            if skip_block {
                if line.contains('{') {
                    brace_count += 1;
                }
                if line.contains('}') {
                    brace_count -= 1;
                    if brace_count == 0 {
                        skip_block = false;
                    }
                }
                continue;
            }

            new_lines.push(line);
        }

        let new_content = new_lines.join("\n");
        self.write_caddyfile(&new_content).await?;
        self.reload().await?;

        info!("✅ Caddy route removed successfully");
        Ok(())
    }

    async fn reload(&self) -> Result<()> {
        info!("🔄 Reloading Caddy...");

        let output = Command::new("docker")
            .args(["exec", &self.container_name, "caddy", "reload", "--config", &self.config_path])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Caddy reload failed: {}", stderr);
            anyhow::bail!("Caddy reload failed: {}", stderr);
        }

        info!("✅ Caddy reloaded successfully");
        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        let output = Command::new("docker")
            .args(["exec", &self.container_name, "caddy", "version"])
            .output()
            .await?;

        Ok(output.status.success())
    }
}

/// Nginx proxy adapter
pub struct NginxAdapter {
    container_name: String,
    config_dir: String,
}

impl NginxAdapter {
    pub fn new(container_name: String) -> Self {
        Self {
            container_name,
            config_dir: "/etc/nginx/conf.d".to_string(),
        }
    }

    fn generate_nginx_block(&self, config: &RouteConfig) -> String {
        let mut block = format!("# {} (Shipwright)\n", config.service_name);
        block.push_str("server {\n");
        block.push_str("    listen 80;\n");
        block.push_str(&format!("    server_name {};\n\n", config.domain));

        if config.enable_cors {
            block.push_str("    add_header 'Access-Control-Allow-Origin' '$http_origin' always;\n");
            block.push_str("    add_header 'Access-Control-Allow-Credentials' 'true' always;\n");
            block.push_str("    add_header 'Access-Control-Allow-Methods' 'GET, POST, PUT, DELETE, OPTIONS' always;\n\n");
        }

        let location = config.path.as_ref().map(|p| format!("{}/*", p)).unwrap_or_else(|| "/".to_string());
        block.push_str(&format!("    location {} {{\n", location));
        block.push_str(&format!("        proxy_pass http://{}:{};\n", config.service_name, config.port));
        block.push_str("        proxy_set_header Host $host;\n");
        block.push_str("        proxy_set_header X-Real-IP $remote_addr;\n");
        block.push_str("        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n");
        block.push_str("    }\n");
        block.push_str("}\n");

        block
    }
}

#[async_trait]
impl ProxyAdapter for NginxAdapter {
    async fn add_route(&self, config: RouteConfig) -> Result<()> {
        info!("📝 Adding Nginx route for {} -> {}:{}", config.domain, config.service_name, config.port);

        let filename = format!("{}.conf", config.service_name);
        let config_content = self.generate_nginx_block(&config);

        // Write to temp file
        let temp_path = format!("/tmp/{}", filename);
        tokio::fs::write(&temp_path, &config_content).await?;

        // Copy to container
        let dest_path = format!("{}/{}", self.config_dir, filename);
        Command::new("docker")
            .args(["cp", &temp_path, &format!("{}:{}", self.container_name, dest_path)])
            .output()
            .await?;

        self.reload().await?;

        info!("✅ Nginx route added successfully");
        Ok(())
    }

    async fn remove_route(&self, domain: &str) -> Result<()> {
        info!("🗑️  Removing Nginx route for {}", domain);

        // Find config file by domain
        let output = Command::new("docker")
            .args(["exec", &self.container_name, "grep", "-l", domain, &format!("{}/*.conf", self.config_dir)])
            .output()
            .await?;

        if output.status.success() {
            let file_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Command::new("docker")
                .args(["exec", &self.container_name, "rm", &file_path])
                .output()
                .await?;

            self.reload().await?;
            info!("✅ Nginx route removed successfully");
        }

        Ok(())
    }

    async fn reload(&self) -> Result<()> {
        info!("🔄 Reloading Nginx...");

        let output = Command::new("docker")
            .args(["exec", &self.container_name, "nginx", "-s", "reload"])
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!("Nginx reload failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        info!("✅ Nginx reloaded successfully");
        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        let output = Command::new("docker")
            .args(["exec", &self.container_name, "nginx", "-t"])
            .output()
            .await?;

        Ok(output.status.success())
    }
}

/// Traefik proxy adapter
pub struct TraefikAdapter {
    container_name: String,
}

impl TraefikAdapter {
    pub fn new(container_name: String) -> Self {
        Self { container_name }
    }
}

#[async_trait]
impl ProxyAdapter for TraefikAdapter {
    async fn add_route(&self, config: RouteConfig) -> Result<()> {
        info!("📝 Traefik uses Docker labels for routing. Please add the following labels to your docker-compose.yml:");
        println!("\nlabels:");
        println!("  - \"traefik.enable=true\"");
        println!("  - \"traefik.http.routers.{}.rule=Host(`{}`)\"", config.service_name, config.domain);
        println!("  - \"traefik.http.services.{}.loadbalancer.server.port={}\"", config.service_name, config.port);

        if config.enable_tls {
            println!("  - \"traefik.http.routers.{}.tls=true\"", config.service_name);
            println!("  - \"traefik.http.routers.{}.tls.certresolver=letsencrypt\"", config.service_name);
        }

        warn!("Traefik configuration requires manual label addition");
        Ok(())
    }

    async fn remove_route(&self, _domain: &str) -> Result<()> {
        warn!("Traefik route removal requires removing labels from docker-compose.yml");
        Ok(())
    }

    async fn reload(&self) -> Result<()> {
        // Traefik auto-reloads when Docker labels change
        info!("Traefik automatically reloads on label changes");
        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        let output = Command::new("docker")
            .args(["exec", &self.container_name, "traefik", "version"])
            .output()
            .await?;

        Ok(output.status.success())
    }
}

/// Create appropriate proxy adapter based on detected type
pub fn create_adapter(proxy_type: &str, container_name: String) -> Box<dyn ProxyAdapter> {
    match proxy_type {
        "caddy" => Box::new(CaddyAdapter::new(container_name)),
        "nginx" => Box::new(NginxAdapter::new(container_name)),
        "traefik" => Box::new(TraefikAdapter::new(container_name)),
        _ => Box::new(CaddyAdapter::new(container_name)), // Default to Caddy
    }
}

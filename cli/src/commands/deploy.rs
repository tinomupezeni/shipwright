use anyhow::Result;
use shipwright_common::config::Config;
use std::fs;
use std::path::Path;
use crate::docker::build::build_image;
use crate::docker::deploy::deploy_image;
use crate::docker::pipeline::{build_and_push_services, discover_services};
use tokio::time::{sleep, Duration};

pub async fn run(dry_run: bool) -> Result<()> {
    let config_path = Path::new(".shipwright.yml");

    if !config_path.exists() {
        println!(".shipwright.yml not found. Run 'shipwright init' first.");
        return Ok(());
    }

    let config_content = fs::read_to_string(config_path)?;
    let mut config: Config = serde_yaml::from_str(&config_content)?;

    // Environment variable overrides
    if let Ok(host) = std::env::var("SHIPWRIGHT_VPS_HOST") {
        if let Some(vps) = &mut config.deploy.vps {
            vps.host = host;
        }
    }
    if let Ok(url) = std::env::var("SHIPWRIGHT_REGISTRY_URL") {
        if let Some(registry) = &mut config.deploy.registry {
            registry.url = url;
        }
    }

    println!("Project: {}", config.project.name);

    if dry_run {
        println!("Performing dry run for {}...", config.project.name);
        println!("Deploy type: {}", config.deploy.deploy_type);
        if let Some(registry) = &config.deploy.registry {
            println!("Deploying to: {}", registry.url);
        }
        
        if config.deploy.deploy_type == "docker-compose" {
            // Find the compose file to check for buildable services
            let candidates = [
                "docker-compose.production.yml",
                "docker-compose.prod.yml",
                "docker-compose.yml",
                "docker-compose.yaml",
                "compose.yml",
                "compose.yaml",
            ];
            
            for candidate in candidates {
                if fs::metadata(candidate).is_ok() {
                    let services = discover_services(candidate)?;
                    if !services.is_empty() {
                        println!("Found {} buildable service(s) in {}:", services.len(), candidate);
                        for svc in services {
                            println!("  • {} ({})", svc.name, svc.context);
                        }
                    }
                    break;
                }
            }
        }

        if let Some(vps) = &config.deploy.vps {
            println!("VPS: {}@{}", vps.user, vps.host);
        }
    } else {
        println!("Starting deployment for {}...", config.project.name);

        // Handle building phase
        if config.deploy.deploy_type == "docker-compose" {
            // Check for buildable services in docker-compose
            let candidates = [
                "docker-compose.production.yml",
                "docker-compose.prod.yml",
                "docker-compose.yml",
                "docker-compose.yaml",
                "compose.yml",
                "compose.yaml",
            ];
            
            let mut compose_file = None;
            for candidate in candidates {
                if fs::metadata(candidate).is_ok() {
                    compose_file = Some(candidate.to_string());
                    break;
                }
            }
            
            if let Some(file) = compose_file {
                let services = discover_services(&file)?;
                if !services.is_empty() {
                    println!("Found {} buildable services. Proceeding with multi-service build...", services.len());
                    let registry_url = config.deploy.registry.as_ref().map(|r| r.url.as_str()).unwrap_or("local");
                    build_and_push_services(&config, &file, registry_url).await?;
                }
            }
        } else {
            // Single container deployment - build local image
            build_image(&config).await?;
        }

        deploy_image(&config).await?;
        
        if let Some(health) = &config.deploy.health {
            if let Some(vps) = &config.deploy.vps {
                let remote_dir = format!("/home/{}/apps/{}", vps.user, config.project.name);
                check_health(vps, health, &remote_dir).await?;
            }
        }

        if let Some(smoke_tests) = &config.deploy.smoke_tests {
            if let Some(vps) = &config.deploy.vps {
                run_smoke_tests(vps, smoke_tests).await?;
            }
        }
    }

    Ok(())
}

async fn run_smoke_tests(vps: &shipwright_common::config::VpsConfig, tests: &Vec<String>) -> Result<()> {
    println!("Running smoke tests...");
    let client = reqwest::Client::new();
    
    for test in tests {
        // Simple parser for "GET /path expect=200"
        let parts: Vec<&str> = test.split_whitespace().collect();
        if parts.len() < 2 { continue; }
        
        let method = parts[0];
        let path = parts[1];
        let mut expected_status = 200;
        
        for part in &parts[2..] {
            if part.starts_with("expect=") {
                expected_status = part.replace("expect=", "").parse()?;
            }
        }
        
        let url = format!("http://{}{}", vps.host, path);
        println!("Smoke Test: {} {} -> expect {}", method, path, expected_status);
        
        let res = match method {
            "GET" => client.get(&url).send().await?,
            _ => continue,
        };
        
        if res.status().as_u16() == expected_status {
            println!("● PASSED: {} {}", method, path);
        } else {
            println!("○ FAILED: {} {} (Got {})", method, path, res.status());
            anyhow::bail!("Smoke test failed");
        }
    }
    
    Ok(())
}

async fn check_health(vps: &shipwright_common::config::VpsConfig, health: &shipwright_common::config::HealthConfig, remote_dir: &str) -> Result<()> {
    // 1. Verify container is Running via Docker API
    println!("Checking container states...");
    let statuses = crate::docker::diagnostics::get_container_statuses(vps, remote_dir)?;
    for status in statuses {
        if status.status.to_lowercase().contains("exited") || status.status.to_lowercase().contains("restarting") {
            println!("○ UNHEALTHY: Container {} is not running (State: {})", status.name, status.status);
            // Diagnostic X-Ray
            println!("🔍 Fetching recent logs for {}...", status.name);
            if let Ok(logs) = crate::docker::deploy::execute_remote_command(vps, &format!("cd {} && docker compose logs --tail 50 {}", remote_dir, status.name)) {
                println!("--- LOGS ---\n{}\n--- END LOGS ---", logs);
            }
            anyhow::bail!("Container {} is not running", status.name);
        }
    }

    if let Some(http) = &health.http {
        println!("Checking health at http://{}{}", vps.host, http.path);
        
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        let url = format!("http://{}{}", vps.host, http.path);
        
        // Wait a bit for container to start
        sleep(Duration::from_secs(5)).await;
        
        let mut expected_codes = Vec::new();
        if let Some(expect) = http.expect {
            expected_codes.push(expect);
        }
        if let Some(codes) = &http.success_codes {
            expected_codes.extend(codes);
        }
        if expected_codes.is_empty() {
            expected_codes.push(200); // Default
        }
        
        for i in 1..=5 {
            println!("Attempt {}/5...", i);
            let res = client.get(&url).send().await;
            
            match res {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if expected_codes.contains(&status) {
                        println!("● HEALTHY: Application responded with {} (Expected one of {:?})", status, expected_codes);
                        return Ok(());
                    } else {
                        println!("○ UNHEALTHY: Got status {} (Expected one of {:?})", status, expected_codes);
                    }
                }
                Err(e) => {
                    println!("○ UNHEALTHY: Connection failed: {}", e);
                }
            }
            sleep(Duration::from_secs(2)).await;
        }
        
        anyhow::bail!("Health check failed after 5 attempts");
    }
    
    Ok(())
}

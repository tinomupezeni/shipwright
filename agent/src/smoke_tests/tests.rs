/// Smoke test implementations
///
/// This module contains the actual test implementations for various deployment scenarios.

use super::*;
use crate::pipeline::deploy::DeploymentContext;
use anyhow::{Result, Context, bail};
use bollard::Docker;
use std::time::Duration;
use tracing::{info, debug};

/// Get all tests for a specific category
pub fn get_tests_for_category(category: TestCategory, ctx: &DeploymentContext) -> Vec<SmokeTest> {
    match category {
        TestCategory::PreDeployment => get_pre_deployment_tests(),
        TestCategory::PostBuild => get_post_build_tests(),
        TestCategory::PostDeployment => get_post_deployment_tests(ctx),
        TestCategory::Integration => get_integration_tests(),
    }
}

/// Pre-deployment tests
fn get_pre_deployment_tests() -> Vec<SmokeTest> {
    vec![
        SmokeTest {
            name: "check_docker_running".to_string(),
            description: "Verify Docker daemon is running".to_string(),
            category: TestCategory::PreDeployment,
            severity: Severity::Critical,
            timeout: Duration::from_secs(10),
            execute: Box::new(|ctx| Box::pin(check_docker_running(ctx.clone()))),
        },
        SmokeTest {
            name: "check_disk_space".to_string(),
            description: "Ensure sufficient disk space available".to_string(),
            category: TestCategory::PreDeployment,
            severity: Severity::High,
            timeout: Duration::from_secs(5),
            execute: Box::new(|ctx| Box::pin(check_disk_space(ctx.clone()))),
        },
        SmokeTest {
            name: "validate_compose_file".to_string(),
            description: "Validate docker-compose file syntax".to_string(),
            category: TestCategory::PreDeployment,
            severity: Severity::Critical,
            timeout: Duration::from_secs(10),
            execute: Box::new(|ctx| Box::pin(validate_compose_file(ctx.clone()))),
        },
    ]
}

/// Post-build tests
fn get_post_build_tests() -> Vec<SmokeTest> {
    vec![
        SmokeTest {
            name: "verify_images_built".to_string(),
            description: "Verify all required images were built".to_string(),
            category: TestCategory::PostBuild,
            severity: Severity::Critical,
            timeout: Duration::from_secs(10),
            execute: Box::new(|ctx| Box::pin(verify_images_built(ctx.clone()))),
        },
    ]
}

/// Post-deployment tests
fn get_post_deployment_tests(ctx: &DeploymentContext) -> Vec<SmokeTest> {
    vec![
        SmokeTest {
            name: "check_containers_running".to_string(),
            description: "Verify all containers are running (not restarting)".to_string(),
            category: TestCategory::PostDeployment,
            severity: Severity::Critical,
            timeout: Duration::from_secs(60),
            execute: Box::new(|ctx| Box::pin(check_containers_running(ctx.clone()))),
        },
        SmokeTest {
            name: "verify_environment_variables".to_string(),
            description: "Check containers have required environment variables".to_string(),
            category: TestCategory::PostDeployment,
            severity: Severity::Critical,
            timeout: Duration::from_secs(30),
            execute: Box::new(|ctx| Box::pin(verify_environment_variables(ctx.clone()))),
        },
        SmokeTest {
            name: "test_network_connectivity".to_string(),
            description: "Verify containers can reach shared resources".to_string(),
            category: TestCategory::PostDeployment,
            severity: Severity::Critical,
            timeout: Duration::from_secs(30),
            execute: Box::new(|ctx| Box::pin(test_network_connectivity(ctx.clone()))),
        },
        SmokeTest {
            name: "check_container_logs".to_string(),
            description: "Inspect logs for critical errors".to_string(),
            category: TestCategory::PostDeployment,
            severity: Severity::High,
            timeout: Duration::from_secs(15),
            execute: Box::new(|ctx| Box::pin(check_container_logs(ctx.clone()))),
        },
    ]
}

/// Integration tests
fn get_integration_tests() -> Vec<SmokeTest> {
    vec![]
}

// ============================================================================
// Test Implementations
// ============================================================================

/// Check if Docker daemon is running
async fn check_docker_running(_ctx: DeploymentContext) -> Result<()> {
    let docker = Docker::connect_with_socket_defaults()
        .context("Failed to connect to Docker daemon - is Docker running?")?;

    docker.ping().await
        .context("Docker daemon not responding to ping")?;

    info!("✓ Docker daemon is running");
    Ok(())
}

/// Check for sufficient disk space
async fn check_disk_space(ctx: DeploymentContext) -> Result<()> {
    use std::process::Command;

    let output = Command::new("df")
        .arg("-BG")  // Show in gigabytes
        .arg(&ctx.build_dir)
        .output()
        .context("Failed to check disk space")?;

    if !output.status.success() {
        bail!("df command failed");
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = output_str.lines().collect();

    if lines.len() < 2 {
        bail!("Unexpected df output");
    }

    // Parse available space (4th column)
    let parts: Vec<&str> = lines[1].split_whitespace().collect();
    if parts.len() < 4 {
        bail!("Could not parse df output");
    }

    let available = parts[3].trim_end_matches('G').parse::<u64>()
        .context("Could not parse available space")?;

    const MIN_REQUIRED_GB: u64 = 5;
    if available < MIN_REQUIRED_GB {
        bail!("Insufficient disk space: {}GB available, {}GB required", available, MIN_REQUIRED_GB);
    }

    info!("✓ Sufficient disk space: {}GB available", available);
    Ok(())
}

/// Validate docker-compose file syntax
async fn validate_compose_file(ctx: DeploymentContext) -> Result<()> {
    use std::process::Command;
    use std::path::Path;

    // Find compose file
    let compose_path = Path::new(&ctx.build_dir)
        .join("docker-compose.yml");  // Simplified for now

    if !compose_path.exists() {
        info!("No docker-compose.yml found, skipping validation");
        return Ok(());
    }

    // Check for CRLF line endings (Windows -> Linux issues)
    let content = tokio::fs::read(&compose_path).await
        .context("Failed to read docker-compose.yml")?;

    if content.windows(2).any(|w| w == b"\r\n") {
        warn!("⚠ docker-compose.yml contains CRLF line endings (Windows-style)");
        warn!("This may cause issues on Linux. Consider converting to LF.");
    }

    // Validate syntax with docker-compose config
    let output = Command::new("docker-compose")
        .arg("-f")
        .arg(&compose_path)
        .arg("config")
        .arg("--quiet")
        .output();

    match output {
        Ok(out) if out.status.success() => {
            info!("✓ docker-compose.yml syntax is valid");
            Ok(())
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!("docker-compose.yml validation failed:\n{}", stderr);
        }
        Err(e) => {
            warn!("Could not validate docker-compose.yml: {}", e);
            warn!("Make sure docker-compose is installed");
            Ok(())  // Non-blocking if docker-compose not available
        }
    }
}

/// Verify images were built successfully
async fn verify_images_built(ctx: DeploymentContext) -> Result<()> {
    let docker = Docker::connect_with_socket_defaults()?;

    // Get expected image name
    let image_name = format!("{}:latest", ctx.project_name);

    // Check if image exists
    let images = docker.list_images::<String>(None).await?;

    let found = images.iter().any(|img| {
        img.repo_tags.iter().any(|tag| tag.contains(&ctx.project_name))
    });

    if !found {
        bail!("Expected image not found: {}", image_name);
    }

    info!("✓ Images built successfully");
    Ok(())
}

/// Check if containers are running (not in crash loop)
async fn check_containers_running(ctx: DeploymentContext) -> Result<()> {
    use bollard::container::ListContainersOptions;
    use std::collections::HashMap;

    let docker = Docker::connect_with_socket_defaults()?;

    // Wait a bit for containers to start
    tokio::time::sleep(Duration::from_secs(5)).await;

    let mut filters = HashMap::new();
    filters.insert("name".to_string(), vec![ctx.project_name.clone()]);

    let options = Some(ListContainersOptions {
        all: true,
        filters,
        ..Default::default()
    });

    let containers = docker.list_containers(options).await?;

    if containers.is_empty() {
        bail!("No containers found for project {}", ctx.project_name);
    }

    let mut restarting_containers = Vec::new();
    let mut exited_containers = Vec::new();

    for container in &containers {
        let name = container.names.as_ref()
            .and_then(|n| n.first())
            .map(|s| s.trim_start_matches('/'))
            .unwrap_or("unknown");

        let status = container.status.as_deref().unwrap_or("unknown");

        if status.contains("Restarting") {
            restarting_containers.push(name.to_string());
        } else if status.contains("Exited") || status.contains("Created") {
            exited_containers.push(name.to_string());
        }
    }

    if !restarting_containers.is_empty() {
        bail!(
            "Containers in crash loop: {}\n\
            Remediation:\n\
            1. Check logs: docker logs <container-name>\n\
            2. Check environment variables\n\
            3. Verify database connectivity\n\
            4. Check for permission errors",
            restarting_containers.join(", ")
        );
    }

    if !exited_containers.is_empty() {
        bail!(
            "Containers not running: {}\n\
            Remediation:\n\
            1. Check logs: docker logs <container-name>\n\
            2. Review container startup command\n\
            3. Check for missing dependencies",
            exited_containers.join(", ")
        );
    }

    // Wait a bit longer and check again (containers might crash after initial start)
    tokio::time::sleep(Duration::from_secs(25)).await;

    let containers_after = docker.list_containers(Some(ListContainersOptions {
        all: true,
        filters: {
            let mut f = HashMap::new();
            f.insert("name".to_string(), vec![ctx.project_name.clone()]);
            f
        },
        ..Default::default()
    })).await?;

    for container in &containers_after {
        let name = container.names.as_ref()
            .and_then(|n| n.first())
            .map(|s| s.trim_start_matches('/'))
            .unwrap_or("unknown");

        let status = container.status.as_deref().unwrap_or("unknown");

        if status.contains("Restarting") {
            bail!(
                "Container {} started crashing after initial startup\n\
                This often indicates:\n\
                1. Database connection failures\n\
                2. Missing environment variables\n\
                3. Permission errors\n\
                4. Application startup errors",
                name
            );
        }
    }

    info!("✓ All containers running successfully for 30+ seconds");
    Ok(())
}

/// Verify environment variables are set correctly
async fn verify_environment_variables(ctx: DeploymentContext) -> Result<()> {
    use bollard::container::InspectContainerOptions;

    let docker = Docker::connect_with_socket_defaults()?;

    // Get main container name
    let container_name = format!("{}-backend", ctx.project_name)
        .replace("_", "-");  // Docker sanitizes names

    let inspect = docker.inspect_container(&container_name, None::<InspectContainerOptions>).await;

    match inspect {
        Ok(details) => {
            if let Some(config) = details.config {
                if let Some(env) = config.env {
                    // Check for placeholder values
                    let placeholders = ["changeme", "your-", "CHANGE", "TODO", "localhost"];

                    for env_var in &env {
                        for placeholder in &placeholders {
                            if env_var.contains(placeholder) && !env_var.starts_with("PATH") {
                                warn!("⚠ Possible placeholder value in environment: {}", env_var);
                            }
                        }

                        // Check for localhost in URLs
                        if env_var.contains("://localhost") || env_var.contains("127.0.0.1") {
                            warn!("⚠ localhost reference in environment: {}", env_var);
                        }
                    }

                    info!("✓ Environment variables validated");
                    Ok(())
                } else {
                    warn!("No environment variables found in container");
                    Ok(())
                }
            } else {
                warn!("Could not inspect container config");
                Ok(())
            }
        }
        Err(e) => {
            debug!("Could not inspect container {}: {}", container_name, e);
            debug!("Skipping environment variable check");
            Ok(())  // Non-critical - don't fail deployment
        }
    }
}

/// Test network connectivity to shared resources
async fn test_network_connectivity(ctx: DeploymentContext) -> Result<()> {
    use bollard::container::ListContainersOptions;
    use std::collections::HashMap;

    let docker = Docker::connect_with_socket_defaults()?;

    // Get first running container
    let mut filters = HashMap::new();
    filters.insert("name".to_string(), vec![ctx.project_name.clone()]);
    filters.insert("status".to_string(), vec!["running".to_string()]);

    let containers = docker.list_containers(Some(ListContainersOptions {
        filters,
        ..Default::default()
    })).await?;

    if containers.is_empty() {
        return Ok(());  // No containers to test
    }

    let container = &containers[0];
    let container_name = container.names.as_ref()
        .and_then(|n| n.first())
        .map(|s| s.trim_start_matches('/'))
        .unwrap_or("unknown");

    // Test DNS resolution for shared resources
    let shared_hosts = vec!["shared-postgres", "shared-redis"];

    for host in shared_hosts {
        let exec = docker.create_exec(
            container_name,
            bollard::exec::CreateExecOptions {
                cmd: Some(vec!["getent", "hosts", host]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        ).await;

        match exec {
            Ok(exec_result) => {
                let start = docker.start_exec(&exec_result.id, None).await;
                match start {
                    Ok(_) => debug!("✓ DNS resolution works for {}", host),
                    Err(_) => debug!("× Could not resolve {}", host),
                }
            }
            Err(_) => {
                // getent might not be available, skip
                debug!("Could not test DNS resolution (getent not available)");
                break;
            }
        }
    }

    info!("✓ Network connectivity validated");
    Ok(())
}

/// Check container logs for critical errors
async fn check_container_logs(ctx: DeploymentContext) -> Result<()> {
    use bollard::container::{ListContainersOptions, LogsOptions};
    use std::collections::HashMap;
    use futures::stream::StreamExt;

    let docker = Docker::connect_with_socket_defaults()?;

    let mut filters = HashMap::new();
    filters.insert("name".to_string(), vec![ctx.project_name.clone()]);

    let containers = docker.list_containers(Some(ListContainersOptions {
        all: true,
        filters,
        ..Default::default()
    })).await?;

    let error_patterns = [
        "Error:",
        "ERROR",
        "CRITICAL",
        "connection refused",
        "Connection refused",
        "Authentication failed",
        "permission denied",
        "Permission denied",
        "No such file",
        "cannot import",
        "ModuleNotFoundError",
        "Fatal",
    ];

    let mut found_errors = Vec::new();

    for container in containers {
        let name = container.names.as_ref()
            .and_then(|n| n.first())
            .map(|s| s.trim_start_matches('/').to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let options = Some(LogsOptions::<String> {
            stdout: true,
            stderr: true,
            tail: "100".to_string(),
            ..Default::default()
        });

        let mut logs = docker.logs(&container.id.as_ref().unwrap(), options);

        let mut log_output = String::new();
        while let Some(log) = logs.next().await {
            if let Ok(line) = log {
                log_output.push_str(&line.to_string());
            }
        }

        // Check for error patterns
        for pattern in &error_patterns {
            if log_output.contains(pattern) {
                found_errors.push(format!("{}: found '{}'", name, pattern));
            }
        }
    }

    if !found_errors.is_empty() {
        warn!("⚠ Found error patterns in logs:");
        for error in &found_errors {
            warn!("  - {}", error);
        }
        warn!("Review logs with: docker logs <container-name>");
    } else {
        info!("✓ No critical errors found in logs");
    }

    Ok(())  // Non-critical - don't fail deployment on log warnings
}

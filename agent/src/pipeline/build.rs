use anyhow::{Result, Context};
use bollard::Docker;
use bollard::image::BuildImageOptions;
use futures_util::stream::StreamExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use rusqlite::Connection;
use tokio::process::Command;
use tracing::{info, error, warn};
use tokio::sync::broadcast;
use shipwright_common::protocol::{AgentMessage, BuildEvent};
use shipwright_common::config::Config;
use crate::infrastructure::{detect_infrastructure, detector::recommend_deploy_dir};
use crate::deployment_tracking::{DeploymentTracker, DeploymentStatus};

/// Run the full pipeline for a project
pub async fn run_pipeline(
    project_id: &str,
    project_name: &str,
    repo_url: &str,
    tx: broadcast::Sender<AgentMessage>,
    db: Arc<Mutex<Connection>>,
    config: Option<Config>,
    source_dir: Option<String>,
) -> Result<String> {
    info!("Starting infrastructure-aware pipeline for project: {}", project_name);

    let tracker = DeploymentTracker::new(db.clone());

    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: project_name.to_string(),
        event: BuildEvent::Started,
    });

    // 1. Determine build directory (clone or local)
    let build_dir = if let Some(local_dir) = source_dir {
        info!("📦 Using local source directory: {}", local_dir);
        PathBuf::from(local_dir)
    } else {
        match clone_repo(repo_url, project_name).await {
            Ok(dir) => dir,
            Err(e) => {
                let _ = tx.send(AgentMessage::BuildUpdate {
                    project_name: project_name.to_string(),
                    event: BuildEvent::Failed(format!("Clone failed: {}", e)),
                });
                return Err(e);
            }
        }
    };

    let build_dir_str = build_dir.to_string_lossy().to_string();

    // 2. Build and deploy project
    if let Err(e) = build_and_deploy_project(project_name, &build_dir_str, tx.clone(), config.as_ref(), db.clone()).await {
        let _ = tx.send(AgentMessage::BuildUpdate {
            project_name: project_name.to_string(),
            event: BuildEvent::Failed(format!("Build failed: {}", e)),
        });
        
        // Note: project_id here is actually the attempt_id from the database
        let _ = tracker.update_status(project_id, DeploymentStatus::Failed);
        return Err(e);
    }

    // 3. Execute deployment
    info!("🚀 Executing deployment for project: {}", project_name);
    let ctx = crate::pipeline::deploy::DeploymentContext::new(
        project_name,
        &build_dir_str,
        config.as_ref(),
        Some(tx.clone()),
    ).await?;

    if let Err(e) = ctx.deploy(config.as_ref()).await {
        let _ = tx.send(AgentMessage::BuildUpdate {
            project_name: project_name.to_string(),
            event: BuildEvent::Failed(format!("Deployment failed: {}", e)),
        });
        
        let _ = tracker.update_status(project_id, DeploymentStatus::Failed);
        return Err(e);
    }

    info!("✅ Pipeline and deployment completed for project: {}", project_name);

    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: project_name.to_string(),
        event: BuildEvent::Success,
    });
    
    let _ = tracker.update_status(project_id, DeploymentStatus::Success);

    Ok(build_dir_str)
}

/// Clone repository to appropriate location (infrastructure-aware)
async fn clone_repo(repo_url: &str, project_name: &str) -> Result<PathBuf> {
    // Detect infrastructure to determine best clone location
    let infrastructure = detect_infrastructure().await?;
    let deploy_dir = recommend_deploy_dir(&infrastructure, project_name);
    let build_dir = PathBuf::from(&deploy_dir);

    // Convert HTTPS URLs to SSH for private repo support
    let clone_url = convert_to_ssh_url(repo_url);

    // Ensure git trusts the directory (prevent "dubious ownership" errors)
    let _ = Command::new("git")
        .args(&["config", "--global", "--add", "safe.directory", &deploy_dir])
        .output()
        .await;

    // Check if directory exists (existing project)
    if build_dir.exists() && build_dir.join(".git").exists() {
        info!("📂 Project exists at {}. Pulling latest changes...", deploy_dir);
        
        let output = Command::new("git")
            .arg("pull")
            .current_dir(&build_dir)
            .output()
            .await?;

        if output.status.success() {
            info!("✅ Successfully updated existing project");
            return Ok(build_dir);
        } else {
            warn!("Git pull failed, performing fresh clone");
            // Delete and continue to fresh clone
            let _ = tokio::fs::remove_dir_all(&build_dir).await;
        }
    }

    // Create directory if it doesn't exist
    if !build_dir.exists() {
        tokio::fs::create_dir_all(&build_dir)
            .await
            .context("Failed to create build directory")?;
    }

    info!("Cloning {} into {:?}", clone_url, build_dir);

    let output = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(&clone_url)
        .arg(".")
        .current_dir(&build_dir)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Git clone failed: {}", stderr);
    }

    Ok(build_dir)
}

/// Helper to convert HTTPS GitHub URLs to SSH
fn convert_to_ssh_url(url: &str) -> String {
    if url.starts_with("https://github.com/") {
        let path = &url[19..];
        format!("git@github.com:{}", path)
    } else {
        url.to_string()
    }
}

/// Build and deploy project based on its structure
async fn build_and_deploy_project(
    project_name: &str,
    build_dir: &str,
    tx: broadcast::Sender<AgentMessage>,
    config: Option<&Config>,
    _db: Arc<Mutex<Connection>>,
) -> Result<()> {
    // 1. Detect project type
    let has_dockerfile = Path::new(build_dir).join("Dockerfile").exists();
    
    // Use configured compose file or detect common ones
    let compose_file = config.as_ref()
        .and_then(|c| c.build.compose_file.clone())
        .or_else(|| {
            let candidates = [
                "docker-compose.deploy.yml",
                "docker-compose.vps.yml",
                "docker-compose.production.yml",
                "docker-compose.yml",
                "infra/docker-compose.deploy.yml",
                "infra/docker-compose.yml",
            ];
            for candidate in candidates {
                if Path::new(build_dir).join(candidate).exists() {
                    return Some(candidate.to_string());
                }
            }
            None
        });

    // 2. Execute build/deploy strategy
    if let Some(file) = compose_file {
        info!("📦 Building with docker compose: {}", file);
        build_with_compose(build_dir, &file, tx.clone(), config).await?;
    } else if has_dockerfile {
        info!("🐳 Building with Dockerfile");
        build_with_docker(build_dir, project_name, tx.clone()).await?;
    } else {
        anyhow::bail!("No Dockerfile or docker-compose.yml found");
    }

    Ok(())
}

/// Build using docker-compose
async fn build_with_compose(
    build_dir: &str,
    compose_file: &str,
    tx: broadcast::Sender<AgentMessage>,
    config: Option<&Config>,
) -> Result<()> {
    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: "docker-compose".to_string(),
        event: BuildEvent::Log("Building services...".to_string()),
    });

    // Ensure .env file exists for docker-compose
    ensure_env_file(build_dir, config).await?;

    // Execute docker compose build
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("build")
        .current_dir(build_dir)
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("docker compose build failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("✅ docker compose build completed successfully");
    Ok(())
}

/// Ensure .env file exists for docker-compose
async fn ensure_env_file(build_dir: &str, config: Option<&Config>) -> Result<()> {
    let env_path = Path::new(build_dir).join(".env");
    
    if !env_path.exists() {
        if let Some(cfg) = config {
            if let Some(env_config) = &cfg.build.environment {
                if !env_config.is_empty() {
                    info!("Creating .env file from config...");
                    let mut content = String::new();
                    for (key, value) in env_config {
                        content.push_str(&format!("{}={}\n", key, value));
                    }
                    tokio::fs::write(&env_path, content).await?;
                    return Ok(());
                }
            }
        }
        
        // Try .env.example if it exists
        let example_path = Path::new(build_dir).join(".env.example");
        if example_path.exists() {
            info!("Creating .env from .env.example...");
            tokio::fs::copy(&example_path, &env_path).await?;
        } else {
            warn!("No .env file, .env.example, or environment config found. Creating minimal .env");
            tokio::fs::write(&env_path, "# Auto-generated by Shipwright - Please update with your values\n").await?;
            info!("⚠️  Created minimal .env file - review and update values!");
        }
    } else {
        info!("✓ Found existing .env file at {:?}", env_path);
    }
    
    Ok(())
}

/// Build using Dockerfile directly
async fn build_with_docker(
    build_dir: &str,
    project_name: &str,
    tx: broadcast::Sender<AgentMessage>,
) -> Result<()> {
    let docker = Docker::connect_with_socket_defaults()?;
    let image_name = format!("{}:latest", project_name);

    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: project_name.to_string(),
        event: BuildEvent::Log("Building Docker image...".to_string()),
    });

    // Create a tarball of the build directory
    let mut tar = tar::Builder::new(Vec::new());
    tar.append_dir_all(".", build_dir)?;
    let tar_data = tar.into_inner()?;

    let options = BuildImageOptions {
        t: image_name.clone(),
        dockerfile: "Dockerfile".to_string(),
        ..Default::default()
    };

    let mut stream = docker.build_image(options, None, Some(tar_data.into()));

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => {
                if let Some(stream_text) = output.stream {
                    let _ = tx.send(AgentMessage::BuildUpdate {
                        project_name: project_name.to_string(),
                        event: BuildEvent::Log(stream_text.trim().to_string()),
                    });
                }
                if let Some(error) = output.error {
                    anyhow::bail!("Docker build error: {}", error);
                }
            }
            Err(e) => anyhow::bail!("Docker API error: {}", e),
        }
    }

    info!("✅ Docker build completed successfully: {}", image_name);
    Ok(())
}

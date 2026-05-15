use anyhow::{Result, Context};
use bollard::Docker;
use bollard::image::BuildImageOptions;
use futures_util::stream::StreamExt;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{info, error, warn};
use tar::Builder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tokio::sync::broadcast;
use shipwright_common::protocol::{AgentMessage, BuildEvent};
use crate::infrastructure::{detect_infrastructure, detector::recommend_deploy_dir};
use crate::pipeline::deploy::DeploymentContext;

pub async fn run_pipeline(
    project_name: &str,
    repo_url: &str,
    tx: broadcast::Sender<AgentMessage>
) -> Result<()> {
    info!("Starting infrastructure-aware pipeline for project: {}", project_name);

    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: project_name.to_string(),
        event: BuildEvent::Started,
    });

    // 1. Clone (infrastructure-aware)
    let build_dir = match clone_repo(repo_url, project_name).await {
        Ok(dir) => dir,
        Err(e) => {
            let _ = tx.send(AgentMessage::BuildUpdate {
                project_name: project_name.to_string(),
                event: BuildEvent::Failed(format!("Clone failed: {}", e)),
            });
            return Err(e);
        }
    };

    // 2. Load config if exists
    let config = load_config(&build_dir).await.ok();

    // 3. Build (check if we should use compose or standalone)
    if let Err(e) = build_project(project_name, &build_dir, tx.clone(), config.as_ref()).await {
        let _ = tx.send(AgentMessage::BuildUpdate {
            project_name: project_name.to_string(),
            event: BuildEvent::Failed(format!("Build failed: {}", e)),
        });
        return Err(e);
    }

    // 4. Deploy using infrastructure-aware deployment
    let build_dir_str = build_dir.to_string_lossy().to_string();
    let deployment = DeploymentContext::new(project_name, &build_dir_str, config.as_ref()).await?;

    if let Err(e) = deployment.deploy(config.as_ref()).await {
        let _ = tx.send(AgentMessage::BuildUpdate {
            project_name: project_name.to_string(),
            event: BuildEvent::Failed(format!("Deploy failed: {}", e)),
        });
        return Err(e);
    }

    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: project_name.to_string(),
        event: BuildEvent::Success,
    });

    Ok(())
}

/// Convert HTTPS GitHub URL to SSH URL for authentication
fn convert_to_ssh_url(url: &str) -> String {
    // Convert https://github.com/user/repo.git to git@github.com:user/repo.git
    if url.starts_with("https://github.com/") {
        url.replace("https://github.com/", "git@github.com:")
    } else {
        url.to_string()
    }
}

/// Clone repository to appropriate location (infrastructure-aware)
async fn clone_repo(repo_url: &str, project_name: &str) -> Result<PathBuf> {
    // Detect infrastructure to determine best clone location
    let infrastructure = detect_infrastructure().await?;
    let deploy_dir = recommend_deploy_dir(&infrastructure, project_name);
    let build_dir = PathBuf::from(&deploy_dir);

    // Convert HTTPS URLs to SSH for private repo support
    let clone_url = convert_to_ssh_url(repo_url);

    // Check if directory exists (existing project)
    if build_dir.exists() && build_dir.join(".git").exists() {
        info!("📂 Project exists at {}. Pulling latest changes...", deploy_dir);

        let output = Command::new("git")
            .arg("pull")
            .arg("--rebase")
            .current_dir(&build_dir)
            .output()
            .await?;

        if output.status.success() {
            info!("✅ Successfully updated existing project");
            return Ok(build_dir);
        } else {
            warn!("Git pull failed, performing fresh clone");
            let _ = std::fs::remove_dir_all(&build_dir);
        }
    } else if build_dir.exists() {
        // Directory exists but is not a git repo - remove it
        warn!("Directory {} exists but is not a git repository. Removing for fresh clone.", deploy_dir);
        std::fs::remove_dir_all(&build_dir)?;
    }

    // Create parent directory if needed
    if let Some(parent) = build_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&build_dir)?;

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
        anyhow::bail!("Failed to clone repository: {}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(build_dir)
}

async fn build_docker_image(
    project_name: &str, 
    build_dir: &Path, 
    tx: broadcast::Sender<AgentMessage>
) -> Result<()> {
    let docker = Docker::connect_with_socket_defaults()?;
    let image_name = format!("{}:latest", project_name);

    if !build_dir.join("Dockerfile").exists() {
        anyhow::bail!("Dockerfile not found in repository root.");
    }

    info!("Building Docker image: {}", image_name);

    // Create tarball context
    let mut buffer = Vec::new();
    {
        let enc = GzEncoder::new(&mut buffer, Compression::default());
        let mut tar = Builder::new(enc);
        tar.append_dir_all(".", build_dir)?;
        tar.finish()?;
    }

    let build_options = BuildImageOptions {
        dockerfile: "Dockerfile",
        t: &image_name,
        rm: true,
        ..Default::default()
    };

    let mut build_stream = docker.build_image(build_options, None, Some(buffer.into()));

    while let Some(msg) = build_stream.next().await {
        let msg = msg?;
        if let Some(stream) = msg.stream {
            let line = stream.trim().to_string();
            if !line.is_empty() {
                info!("Build [{}]: {}", project_name, line);
                let _ = tx.send(AgentMessage::BuildUpdate {
                    project_name: project_name.to_string(),
                    event: BuildEvent::Log(line),
                });
            }
        }
        if let Some(err) = msg.error {
            anyhow::bail!("Build error: {}", err);
        }
    }

    info!("Successfully built {}", image_name);
    Ok(())
}

/// Load Shipwright config from project directory
async fn load_config(build_dir: &Path) -> Result<shipwright_common::config::Config> {
    let config_path = build_dir.join(".shipwright.yml");

    if !config_path.exists() {
        anyhow::bail!("No .shipwright.yml found in project");
    }

    let config_content = tokio::fs::read_to_string(config_path).await?;
    let config: shipwright_common::config::Config = serde_yaml::from_str(&config_content)?;

    Ok(config)
}

/// Build project using appropriate method (standalone or compose)
async fn build_project(
    project_name: &str,
    build_dir: &Path,
    tx: broadcast::Sender<AgentMessage>,
    config: Option<&shipwright_common::config::Config>,
) -> Result<()> {
    // Check if docker-compose file exists
    let compose_candidates = [
        "docker-compose.deploy.yml",
        "docker-compose.vps.yml",
        "docker-compose.production.yml",
        "docker-compose.yml",
    ];

    let compose_file = compose_candidates.iter()
        .find(|&f| build_dir.join(f).exists());

    if let Some(file) = compose_file {
        info!("📦 Building with docker-compose: {}", file);
        build_with_compose(build_dir, file, tx.clone()).await?;
    } else if build_dir.join("Dockerfile").exists() {
        info!("📦 Building with Dockerfile");
        build_docker_image(project_name, build_dir, tx.clone()).await?;
    } else {
        anyhow::bail!("No Dockerfile or docker-compose.yml found");
    }

    Ok(())
}

/// Build using docker-compose
async fn build_with_compose(
    build_dir: &Path,
    compose_file: &str,
    tx: broadcast::Sender<AgentMessage>,
) -> Result<()> {
    let output = Command::new("docker-compose")
        .arg("-f")
        .arg(compose_file)
        .arg("build")
        .arg("--no-cache")
        .current_dir(build_dir)
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("docker-compose build failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("✅ docker-compose build completed successfully");
    Ok(())
}

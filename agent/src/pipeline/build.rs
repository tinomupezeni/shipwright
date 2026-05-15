use anyhow::{Result, Context};
use bollard::Docker;
use bollard::image::BuildImageOptions;
use futures_util::stream::StreamExt;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{info, error};
use tar::Builder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tokio::sync::broadcast;
use shipwright_common::protocol::{AgentMessage, BuildEvent};

pub async fn run_pipeline(
    project_name: &str, 
    repo_url: &str, 
    tx: broadcast::Sender<AgentMessage>
) -> Result<()> {
    info!("Starting pipeline for project: {}", project_name);

    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: project_name.to_string(),
        event: BuildEvent::Started,
    });

    // 1. Clone
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

    // 2. Build
    if let Err(e) = build_docker_image(project_name, &build_dir, tx.clone()).await {
        let _ = tx.send(AgentMessage::BuildUpdate {
            project_name: project_name.to_string(),
            event: BuildEvent::Failed(format!("Build failed: {}", e)),
        });
        return Err(e);
    }

    // 3. Deploy (Swap containers)
    if let Err(e) = deploy_container(project_name).await {
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

async fn clone_repo(repo_url: &str, project_name: &str) -> Result<PathBuf> {
    let build_dir = std::env::temp_dir().join("shipwright-builds").join(project_name);
    
    if build_dir.exists() {
        let _ = std::fs::remove_dir_all(&build_dir);
    }
    std::fs::create_dir_all(&build_dir)?;

    info!("Cloning {} into {:?}", repo_url, build_dir);
    
    let output = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(repo_url)
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

async fn deploy_container(project_name: &str) -> Result<()> {
    let docker = Docker::connect_with_socket_defaults()?;
    let image_name = format!("{}:latest", project_name);
    let container_name = format!("shipwright-{}", project_name);

    info!("Deploying container: {}", container_name);

    // Stop and remove existing container if it exists
    use bollard::container::{StopContainerOptions, RemoveContainerOptions, Config, CreateContainerOptions, StartContainerOptions};
    
    let _ = docker.stop_container(&container_name, Some(StopContainerOptions { t: 10 })).await;
    let _ = docker.remove_container(&container_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;

    // Create and start new container
    let config = Config {
        image: Some(image_name),
        ..Default::default()
    };

    docker.create_container(Some(CreateContainerOptions { name: container_name.clone(), ..Default::default() }), config).await?;
    docker.start_container(&container_name, None::<StartContainerOptions<String>>).await?;

    info!("Successfully deployed {}", container_name);
    Ok(())
}

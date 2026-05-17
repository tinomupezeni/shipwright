use anyhow::{Result, Context};
use bollard::Docker;
use bollard::image::BuildImageOptions;
use futures_util::stream::StreamExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use rusqlite::Connection;
use tokio::process::Command;
use tracing::{info, error, warn};
use tar::Builder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tokio::sync::broadcast;
use shipwright_common::protocol::{AgentMessage, BuildEvent};
use crate::infrastructure::{detect_infrastructure, detector::recommend_deploy_dir};
use crate::pipeline::deploy::DeploymentContext;
use crate::deployment_tracking::{DeploymentTracker, DeploymentStatus};

/// Get the current commit SHA from a git repository
async fn get_commit_sha(build_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(build_dir)
        .output()
        .await
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get commit SHA: {}", String::from_utf8_lossy(&output.stderr));
    }

    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(sha)
}

pub async fn run_pipeline(
    project_id: &str,
    project_name: &str,
    repo_url_or_dir: &str,
    tx: broadcast::Sender<AgentMessage>,
    db: Arc<Mutex<Connection>>,
    attempt_id: Option<String>,
) -> Result<String> {
    info!("Starting infrastructure-aware pipeline for project: {}", project_name);

    let tracker = DeploymentTracker::new(db.clone());

    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: project_name.to_string(),
        event: BuildEvent::Started,
    });

    // 1. Clone (infrastructure-aware)
    let build_dir = match clone_repo(repo_url_or_dir, project_name).await {
        Ok(dir) => dir,
        Err(e) => {
            let _ = tx.send(AgentMessage::BuildUpdate {
                project_name: project_name.to_string(),
                event: BuildEvent::Failed(format!("Clone failed: {}", e)),
            });
            return Err(e);
        }
    };

    // Get commit SHA
    let commit_sha = get_commit_sha(&build_dir).await
        .context("Failed to get commit SHA")?;

    // Create or use existing deployment attempt
    let attempt = if let Some(id) = attempt_id {
        // Retry case: load existing attempt
        tracker.get_attempt(&id)?
            .context("Retry attempt not found")?
    } else {
        // Webhook case: create new attempt
        let config_path = build_dir.join(".shipwright.yml");
        tracker.create_attempt(
            project_id,
            project_name,
            &commit_sha,
            &build_dir.to_string_lossy(),
            &config_path.to_string_lossy(),
            "webhook",
        )?
    };

    let attempt_id = attempt.id.clone();

    // Update status to Running
    tracker.update_status(&attempt_id, DeploymentStatus::Running)?;

    // 2. Load config if exists
    let config = load_config(&build_dir).await.ok();

    // 3. Build (check if we should use compose or standalone)
    if let Err(e) = build_project(project_name, &build_dir, tx.clone(), config.as_ref()).await {
        let _ = tx.send(AgentMessage::BuildUpdate {
            project_name: project_name.to_string(),
            event: BuildEvent::Failed(format!("Build failed: {}", e)),
        });

        // Mark deployment as failed
        let _ = tracker.complete_attempt(
            &attempt_id,
            DeploymentStatus::Failed,
            Some("Build failed".to_string()),
            Some(format!("{:#}", e)),
        );

        return Err(e);
    }

    // 3.5. Run post-build smoke tests
    use crate::smoke_tests::{SmokeTestRunner, SmokeTestConfig, TestCategory};

    let build_dir_str = build_dir.to_string_lossy().to_string();
    let deployment = DeploymentContext::new(project_name, &build_dir_str, config.as_ref(), Some(tx.clone())).await?;

    let smoke_config = if let Some(cfg) = &config {
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

    if smoke_config.enabled {
        info!("🧪 Running post-build smoke tests...");
        let mut test_runner = SmokeTestRunner::new(deployment.clone(), smoke_config.clone());

        if let Err(e) = test_runner.run_category(TestCategory::PostBuild).await {
            let _ = tx.send(AgentMessage::BuildUpdate {
                project_name: project_name.to_string(),
                event: BuildEvent::Failed(format!("Post-build smoke tests failed: {}", e)),
            });

            // Mark deployment as failed
            let _ = tracker.complete_attempt(
                &attempt_id,
                DeploymentStatus::Failed,
                Some("Post-build smoke tests failed".to_string()),
                Some(format!("{:#}", e)),
            );

            return Err(e);
        }
    }

    // 4. Deploy using infrastructure-aware deployment

    if let Err(e) = deployment.deploy(config.as_ref()).await {
        let _ = tx.send(AgentMessage::BuildUpdate {
            project_name: project_name.to_string(),
            event: BuildEvent::Failed(format!("Deploy failed: {}", e)),
        });

        // Mark deployment as failed
        let _ = tracker.complete_attempt(
            &attempt_id,
            DeploymentStatus::Failed,
            Some("Deployment failed".to_string()),
            Some(format!("{:#}", e)),
        );

        return Err(e);
    }

    // Mark deployment as successful
    tracker.complete_attempt(
        &attempt_id,
        DeploymentStatus::Success,
        None,
        None,
    )?;

    let _ = tx.send(AgentMessage::BuildUpdate {
        project_name: project_name.to_string(),
        event: BuildEvent::Success,
    });

    Ok(attempt_id)
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
    // First check if config specifies a compose file
    if let Some(cfg) = config {
        if let Some(compose_file) = &cfg.build.compose_file {
            let compose_path = build_dir.join(compose_file);
            if compose_path.exists() {
                info!("📦 Building with configured docker-compose: {}", compose_file);
                build_with_compose(build_dir, compose_file, tx.clone(), config).await?;
                return Ok(());
            } else {
                warn!("Configured compose file {} not found, falling back to auto-detection", compose_file);
            }
        }
    }

    // Fallback: auto-detect compose file
    let compose_candidates = [
        "docker-compose.deploy.yml",
        "docker-compose.vps.yml",
        "docker-compose.production.yml",
        "docker-compose.yml",
        "infra/docker-compose.deploy.yml",
        "infra/docker-compose.yml",
    ];

    let compose_file = compose_candidates.iter()
        .find(|&f| build_dir.join(f).exists());

    if let Some(file) = compose_file {
        info!("📦 Building with docker-compose: {}", file);
        build_with_compose(build_dir, file, tx.clone(), config).await?;
    } else if build_dir.join("Dockerfile").exists() {
        info!("📦 Building with Dockerfile");
        build_docker_image(project_name, build_dir, tx.clone()).await?;
    } else {
        anyhow::bail!("No Dockerfile or docker-compose.yml found");
    }

    Ok(())
}

/// Ensure .env file exists for docker-compose
async fn ensure_env_file(build_dir: &Path, compose_file: &str, config: Option<&shipwright_common::config::Config>) -> Result<()> {
    use std::collections::HashMap;

    // Determine .env file location based on compose file location
    let compose_path = build_dir.join(compose_file);
    let compose_dir = compose_path.parent().unwrap_or(build_dir);
    let env_file = compose_dir.join(".env");

    // If config has environment variables, merge them into .env
    if let Some(cfg) = config {
        if let Some(config_vars) = &cfg.build.environment {
            if !config_vars.is_empty() {
                // Read existing .env if it exists
                let mut env_vars = HashMap::new();

                if env_file.exists() {
                    let existing_content = tokio::fs::read_to_string(&env_file).await?;
                    for line in existing_content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        if let Some(eq_pos) = line.find('=') {
                            let key = line[..eq_pos].trim().to_string();
                            let value = line[eq_pos + 1..].trim().to_string();
                            env_vars.insert(key, value);
                        }
                    }
                    info!("✓ Found existing .env file at {}", env_file.display());
                }

                // Merge config variables (config takes precedence)
                for (key, value) in config_vars.iter() {
                    env_vars.insert(key.clone(), value.clone());
                }

                // Write merged env file
                let env_content = env_vars.iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join("\n");

                tokio::fs::write(&env_file, env_content).await?;
                info!("✓ Merged {} environment variables from config into .env", config_vars.len());
                return Ok(());
            }
        }
    }

    // If .env already exists and no config vars, we're good
    if env_file.exists() {
        info!("✓ Found existing .env file at {}", env_file.display());
        return Ok(());
    }

    // Check for .env.example or .env.template
    let env_example = compose_dir.join(".env.example");
    let env_template = compose_dir.join(".env.template");

    if env_example.exists() {
        info!("Creating .env from .env.example");
        tokio::fs::copy(&env_example, &env_file).await?;
        return Ok(());
    }

    if env_template.exists() {
        info!("Creating .env from .env.template");
        tokio::fs::copy(&env_template, &env_file).await?;
        return Ok(());
    }

    // If config has environment variables, create .env from them
    if let Some(cfg) = config {
        if let Some(env_vars) = &cfg.build.environment {
            if !env_vars.is_empty() {
                info!("Creating .env from config environment variables");
                let env_content = env_vars.iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join("\n");
                tokio::fs::write(&env_file, env_content).await?;
                return Ok(());
            }
        }
    }

    // Create minimal .env with common defaults
    warn!("No .env file, .env.example, or environment config found. Creating minimal .env");
    let minimal_env = "# Auto-generated by Shipwright - Please update with your values\n\
                      # Database\n\
                      POSTGRES_USER=postgres\n\
                      POSTGRES_PASSWORD=changeme\n\
                      POSTGRES_DB=app\n\
                      \n\
                      # Django/Application\n\
                      DJANGO_SECRET_KEY=changeme-insecure-key\n\
                      DJANGO_DEBUG=False\n";

    tokio::fs::write(&env_file, minimal_env).await?;
    info!("⚠️  Created minimal .env file - review and update values!");

    Ok(())
}

/// Fix file ownership to match directory owner (when running as root)
async fn fix_ownership(build_dir: &Path) -> Result<()> {
    // Only try to fix ownership if we're running as root
    let whoami_output = Command::new("whoami").output().await?;
    let current_user = String::from_utf8_lossy(&whoami_output.stdout).trim().to_string();

    if current_user != "root" {
        return Ok(()); // Not running as root, no need to fix
    }

    // Get the directory owner
    let stat_output = Command::new("stat")
        .arg("-c")
        .arg("%U:%G")
        .arg(build_dir.parent().unwrap_or(build_dir))
        .output()
        .await?;

    if !stat_output.status.success() {
        return Ok(()); // Can't determine owner, skip
    }

    let owner = String::from_utf8_lossy(&stat_output.stdout).trim().to_string();

    if owner.is_empty() || owner.starts_with("root") {
        return Ok(()); // Already root-owned, no change needed
    }

    info!("Fixing ownership of {} to {}", build_dir.display(), owner);

    let chown_output = Command::new("chown")
        .arg("-R")
        .arg(&owner)
        .arg(build_dir)
        .output()
        .await?;

    if !chown_output.status.success() {
        warn!("Failed to fix ownership: {}", String::from_utf8_lossy(&chown_output.stderr));
    }

    Ok(())
}

/// Build using docker-compose
async fn build_with_compose(
    build_dir: &Path,
    compose_file: &str,
    tx: broadcast::Sender<AgentMessage>,
    config: Option<&shipwright_common::config::Config>,
) -> Result<()> {
    // Ensure .env file exists
    ensure_env_file(build_dir, compose_file, config).await?;

    // Validate environment variables
    let validation_report = crate::env_validator::validate_env_vars(build_dir, compose_file).await?;
    if !validation_report.is_valid() {
        anyhow::bail!("{}", validation_report.error_message());
    }

    // Fix ownership if running as root
    fix_ownership(build_dir).await?;

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

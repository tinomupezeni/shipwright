use axum::{
    routing::{get, post, delete},
    Router,
    Json,
    extract::State,
    http::{StatusCode, HeaderMap},
    response::IntoResponse,
    body::Bytes,
};

use crate::webhooks::secrets_api;
use crate::webhooks::retry_api;
use serde::{Deserialize, Serialize};
use tracing::{info, error, warn};
use std::sync::{Arc, Mutex};
use rusqlite::Connection;
use tower_http::trace::TraceLayer;
use hmac::{Hmac, Mac};
use sha2::Sha256;

#[derive(Debug, Deserialize)]
pub struct GitHubPushEvent {
    #[serde(rename = "ref")]
    pub reference: String,
    pub repository: Repository,
    pub head_commit: Option<Commit>,
}

#[derive(Debug, Deserialize)]
pub struct Repository {
    pub name: String,
    pub full_name: String,
    pub clone_url: String,
}

#[derive(Debug, Deserialize)]
pub struct Commit {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ProjectRegistration {
    pub name: String,
    pub repo_url: String,
    pub webhook_secret: String,
    pub deploy_branch: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub broadcast_tx: tokio::sync::broadcast::Sender<shipwright_common::protocol::AgentMessage>,
}

pub async fn start_server(addr: &str, state: AppState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/webhooks/github", post(handle_github_webhook))
        .route("/webhooks/shipwright", post(handle_self_update_webhook))
        .route("/projects", post(register_project))
        // Secret Management API (SMP/v1)
        .route("/api/v1/secrets", post(secrets_api::set_secret))
        .route("/api/v1/secrets/list", post(secrets_api::list_secrets))
        .route("/api/v1/secrets/all", post(secrets_api::get_all_secrets))
        .route("/api/v1/secrets/:name", post(secrets_api::get_secret))
        .route("/api/v1/secrets/:name/delete", post(secrets_api::delete_secret))
        // Deployment retry API
        .route("/api/v1/deployments/retry", post(retry_api::retry_deployment))
        .route("/api/v1/deployments/status", post(retry_api::get_deployment_status))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("HTTP Webhook server listening on: {}", addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn register_project(
    State(state): State<AppState>,
    Json(payload): Json<ProjectRegistration>
) -> impl IntoResponse {
    info!("Registering or updating project: {}", payload.name);

    let deploy_branch = payload.deploy_branch.unwrap_or_else(|| "main".to_string());

    let db = state.db.lock().unwrap();
    let res = db.execute(
        "INSERT INTO projects (id, name, repo_url, webhook_secret, deploy_branch, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(name) DO UPDATE SET
            repo_url = excluded.repo_url,
            webhook_secret = excluded.webhook_secret,
            deploy_branch = excluded.deploy_branch",
        (
            uuid::Uuid::new_v4().to_string(),
            &payload.name,
            &payload.repo_url,
            &payload.webhook_secret,
            &deploy_branch,
            chrono::Utc::now().timestamp(),
        ),
    );

    match res {
        Ok(_) => (StatusCode::CREATED, "Project registered successfully").into_response(),
        Err(e) => {
            error!("Failed to register project: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to register project: {}", e)).into_response()
        }
    }
}

/// Verify GitHub webhook signature
fn verify_github_signature(secret: &str, signature_header: &str, body: &[u8]) -> bool {
    // GitHub sends signature as: sha256=<hex_digest>
    if !signature_header.starts_with("sha256=") {
        return false;
    }

    let received_signature = &signature_header[7..]; // Skip "sha256="

    // Compute HMAC-SHA256
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(body);

    // Compare signatures (constant-time comparison)
    let expected = mac.finalize().into_bytes();
    let expected_hex = hex::encode(expected);

    expected_hex == received_signature
}

async fn handle_github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Parse the JSON payload
    let payload: GitHubPushEvent = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to parse GitHub webhook payload: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid JSON payload").into_response();
        }
    };

    info!("Received GitHub webhook for repository: {}", payload.repository.full_name);

    let project_name = payload.repository.name.clone();
    let repo_url = payload.repository.clone_url.clone();

    // Get project details from database
    let project_info: Option<(String, String, String)> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT id, webhook_secret, deploy_branch FROM projects WHERE name = ?1",
            [&project_name],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).ok()
    };

    let (project_id, webhook_secret, deploy_branch) = match project_info {
        Some(info) => info,
        None => {
            warn!("Project {} not registered. Skipping.", project_name);
            return (StatusCode::NOT_FOUND, "Project not registered").into_response();
        }
    };

    // Verify GitHub signature
    if let Some(signature) = headers.get("X-Hub-Signature-256") {
        let signature_str = match signature.to_str() {
            Ok(s) => s,
            Err(_) => {
                error!("Invalid signature header");
                return (StatusCode::BAD_REQUEST, "Invalid signature header").into_response();
            }
        };

        if !verify_github_signature(&webhook_secret, signature_str, &body) {
            error!("GitHub webhook signature verification failed for project: {}", project_name);
            return (StatusCode::UNAUTHORIZED, "Signature verification failed").into_response();
        }
    } else {
        error!("Missing X-Hub-Signature-256 header");
        return (StatusCode::UNAUTHORIZED, "Missing signature header").into_response();
    }

    info!("✓ GitHub webhook signature verified for {}", project_name);

    // Extract branch from ref (refs/heads/main -> main)
    let pushed_branch = payload.reference
        .strip_prefix("refs/heads/")
        .unwrap_or(&payload.reference);

    // Check if this is the branch we should deploy
    if pushed_branch != deploy_branch {
        info!("Ignoring push to branch '{}' (configured to deploy: '{}')", pushed_branch, deploy_branch);
        return (StatusCode::OK, format!("Ignoring push to branch '{}'", pushed_branch)).into_response();
    }

    info!("✓ Push to deploy branch '{}' detected", deploy_branch);

    if let Some(commit) = payload.head_commit {
        info!("🚀 Triggering deployment for {} at commit {} ({})", project_name, &commit.id[..7], commit.message);

        let tx = state.broadcast_tx.clone();
        let db = state.db.clone();
        let project_id_clone = project_id.clone();
        // Spawn the pipeline in the background
        tokio::spawn(async move {
            if let Err(e) = crate::pipeline::build::run_pipeline(
                &project_id_clone,
                &project_name,
                &repo_url,
                tx,
                db,
                None,  // New webhook deployment
            ).await {
                error!("Pipeline failed for {}: {}", project_name, e);
            }
        });

        (StatusCode::OK, "Deployment triggered").into_response()
    } else {
        warn!("No head_commit in webhook payload");
        (StatusCode::OK, "No commit to deploy").into_response()
    }
}

/// Health check endpoint for monitoring and load balancers
async fn health_check() -> impl IntoResponse {
    #[derive(Serialize)]
    struct HealthResponse {
        status: String,
        version: String,
        uptime: String,
    }

    let uptime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let response = HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime: format!("{}s", uptime),
    };

    (StatusCode::OK, Json(response))
}

/// Handle self-update webhook for Shipwright repository
/// When the Shipwright repo is updated, the agent pulls the latest image and restarts
async fn handle_self_update_webhook(
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Parse the JSON payload
    let payload: GitHubPushEvent = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to parse self-update webhook payload: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid JSON payload").into_response();
        }
    };

    info!("Received self-update webhook for Shipwright repository");

    // Only update on push to main branch
    let pushed_branch = payload.reference
        .strip_prefix("refs/heads/")
        .unwrap_or(&payload.reference);

    if pushed_branch != "main" {
        info!("Ignoring self-update push to branch '{}'", pushed_branch);
        return (StatusCode::OK, format!("Ignoring push to branch '{}'", pushed_branch)).into_response();
    }

    // Check if running in Docker or systemd
    let is_docker = std::path::Path::new("/.dockerenv").exists();

    if is_docker {
        info!("🔄 Triggering Docker-based self-update...");

        // Spawn update process in background
        tokio::spawn(async move {
            if let Err(e) = perform_docker_self_update().await {
                error!("Self-update failed: {}", e);
            }
        });

        (StatusCode::OK, "Self-update triggered (Docker mode)").into_response()
    } else {
        info!("🔄 Triggering systemd-based self-update...");

        // Spawn update process in background
        tokio::spawn(async move {
            if let Err(e) = perform_systemd_self_update().await {
                error!("Self-update failed: {}", e);
            }
        });

        (StatusCode::OK, "Self-update triggered (systemd mode)").into_response()
    }
}

/// Perform systemd-based self-update
async fn perform_systemd_self_update() -> anyhow::Result<()> {
    use tokio::process::Command;

    // SHIPWRIGHT_REPO_PATH is set by systemd service to user's home directory
    let repo_path = std::env::var("SHIPWRIGHT_REPO_PATH")
        .expect("SHIPWRIGHT_REPO_PATH must be set in systemd service");

    info!("Step 1: Pulling latest code from GitHub...");

    let output = Command::new("git")
        .current_dir(&repo_path)
        .args(["pull", "origin", "main"])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to pull latest code: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("Step 2: Building updated binary...");

    let output = Command::new("cargo")
        .current_dir(&repo_path)
        .args(["build", "--release", "--package", "shipwright-agent"])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to build binary: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("Step 3: Installing updated binary (requires root)...");

    // Copy binary to system location (service runs as root, so this should work)
    let output = Command::new("install")
        .args([
            "-m", "755",
            &format!("{}/target/release/shipwright-agent", repo_path),
            "/usr/local/bin/shipwright-agent"
        ])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to install binary: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("Step 4: Scheduling service restart in 5 seconds...");

    // Give time for response to be sent
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    info!("Step 5: Restarting systemd service...");

    let output = Command::new("systemctl")
        .args(["restart", "shipwright-agent"])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to restart service: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("✅ Self-update completed successfully");
    Ok(())
}

/// Perform Docker-based self-update
async fn perform_docker_self_update() -> anyhow::Result<()> {
    use tokio::process::Command;

    let repo_path = std::env::var("SHIPWRIGHT_REPO_PATH")
        .unwrap_or_else(|_| "/home/shipwright/.shipwright/repo".to_string());

    info!("Step 1: Pulling latest code from GitHub...");

    let output = Command::new("git")
        .current_dir(&repo_path)
        .args(["pull", "origin", "main"])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to pull latest code: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("Step 2: Building updated binary...");

    let output = Command::new("cargo")
        .current_dir(&repo_path)
        .args(["build", "--release", "--package", "shipwright-agent"])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to build binary: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("Step 3: Rebuilding Docker image with new binary...");

    let output = Command::new("docker")
        .current_dir(&repo_path)
        .args(["build", "-f", "agent/Dockerfile.runtime", "-t", "shipwright-agent:latest", "."])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to build Docker image: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("Step 4: Scheduling container restart in 5 seconds...");

    // Give time for response to be sent
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    info!("Step 5: Restarting container with new image...");

    let output = Command::new("docker")
        .args(["restart", "shipwright-agent"])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to restart container: {}", String::from_utf8_lossy(&output.stderr));
    }

    info!("✅ Self-update completed successfully");
    Ok(())
}

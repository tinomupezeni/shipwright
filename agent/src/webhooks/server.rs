use axum::{
    routing::post,
    Router,
    Json,
    extract::State,
    http::{StatusCode, HeaderMap},
    response::IntoResponse,
    body::Bytes,
};
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
        .route("/webhooks/github", post(handle_github_webhook))
        .route("/projects", post(register_project))
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
    let project_info: Option<(String, String)> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT webhook_secret, deploy_branch FROM projects WHERE name = ?1",
            [&project_name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).ok()
    };

    let (webhook_secret, deploy_branch) = match project_info {
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
        // Spawn the pipeline in the background
        tokio::spawn(async move {
            if let Err(e) = crate::pipeline::build::run_pipeline(&project_name, &repo_url, tx).await {
                error!("Pipeline failed for {}: {}", project_name, e);
            }
        });

        (StatusCode::OK, "Deployment triggered").into_response()
    } else {
        warn!("No head_commit in webhook payload");
        (StatusCode::OK, "No commit to deploy").into_response()
    }
}

use axum::{
    routing::post,
    Router,
    Json,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{info, error};
use std::sync::{Arc, Mutex};
use rusqlite::Connection;
use tower_http::trace::TraceLayer;

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
    
    let db = state.db.lock().unwrap();
    let res = db.execute(
        "INSERT INTO projects (id, name, repo_url, webhook_secret, created_at) 
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(name) DO UPDATE SET
            repo_url = excluded.repo_url,
            webhook_secret = excluded.webhook_secret",
        (
            uuid::Uuid::new_v4().to_string(),
            &payload.name,
            &payload.repo_url,
            &payload.webhook_secret,
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

async fn handle_github_webhook(
    State(state): State<AppState>,
    Json(payload): Json<GitHubPushEvent>
) {
    info!("Received GitHub webhook for repository: {}", payload.repository.full_name);
    
    let project_name = payload.repository.name.clone();
    let repo_url = payload.repository.clone_url.clone();

    // Check if we have this project registered
    let project_exists: bool = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE name = ?1)",
            [&project_name],
            |row| row.get(0),
        ).unwrap_or(false)
    };

    if !project_exists {
        info!("Project {} not registered. Skipping.", project_name);
        return;
    }

    if let Some(commit) = payload.head_commit {
        info!("Triggering build for {} at commit {}", project_name, commit.id);
        
        let tx = state.broadcast_tx.clone();
        // Spawn the pipeline in the background
        tokio::spawn(async move {
            if let Err(e) = crate::pipeline::build::run_pipeline(&project_name, &repo_url, tx).await {
                error!("Pipeline failed for {}: {}", project_name, e);
            }
        });
    }
}

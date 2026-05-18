/// Retry API endpoints for deployment retry functionality
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tracing::{info, error};

use super::server::AppState;
use crate::deployment_tracking::{DeploymentTracker, DeploymentStatus};

#[derive(Debug, Deserialize)]
pub struct RetryRequest {
    pub project_id: String,  // Actually project_name from CLI
}

#[derive(Debug, Serialize)]
pub struct RetryResponse {
    pub success: bool,
    pub message: String,
    pub attempt_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub success: bool,
    pub deployment: Option<DeploymentStatusInfo>,
}

#[derive(Debug, Serialize)]
pub struct DeploymentStatusInfo {
    pub id: String,
    pub project_name: String,
    pub commit_sha: String,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub failure_reason: Option<String>,
    pub retry_count: i32,
}

/// Retry the last failed deployment
pub async fn retry_deployment(
    State(state): State<AppState>,
    Json(req): Json<RetryRequest>,
) -> (StatusCode, Json<RetryResponse>) {
    info!("Retry deployment request for project: {}", req.project_id);

    let tracker = DeploymentTracker::new(state.db.clone());

    // Get the last deployment attempt (req.project_id is actually project_name from CLI)
    let last_attempt = match tracker.get_latest_attempt_by_name(&req.project_id) {
        Ok(Some(attempt)) => attempt,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(RetryResponse {
                    success: false,
                    message: "No deployment found for this project".to_string(),
                    attempt_id: None,
                }),
            );
        }
        Err(e) => {
            error!("Failed to get latest attempt: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RetryResponse {
                    success: false,
                    message: format!("Failed to retrieve deployment history: {}", e),
                    attempt_id: None,
                }),
            );
        }
    };

    // Check if the last deployment failed
    if last_attempt.status != DeploymentStatus::Failed {
        return (
            StatusCode::BAD_REQUEST,
            Json(RetryResponse {
                success: false,
                message: format!(
                    "Last deployment did not fail (status: {:?}). Nothing to retry.",
                    last_attempt.status
                ),
                attempt_id: None,
            }),
        );
    }

    info!(
        "Creating retry attempt for failed deployment {} (commit: {})",
        last_attempt.id, last_attempt.commit_sha
    );

    // Create a retry attempt
    let retry_attempt = match tracker.create_retry_attempt(&last_attempt.id) {
        Ok(attempt) => attempt,
        Err(e) => {
            error!("Failed to create retry attempt: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RetryResponse {
                    success: false,
                    message: format!("Failed to create retry attempt: {}", e),
                    attempt_id: None,
                }),
            );
        }
    };

    // Prepare clones for background task
    let attempt_id_clone = retry_attempt.id.clone();
    let project_name_clone = retry_attempt.project_name.clone();
    let deploy_dir_clone = retry_attempt.deploy_dir.clone();
    let tx_clone = state.broadcast_tx.clone();
    let db_clone = state.db.clone();

    // Spawn the deployment in the background
    tokio::spawn(async move {
        info!("Starting retry deployment for {}", project_name_clone);

        // Run the deployment pipeline with existing attempt
        if let Err(e) = crate::pipeline::build::run_pipeline(
            &attempt_id_clone,
            &project_name_clone,
            &deploy_dir_clone,
            tx_clone,
            db_clone,
            None,
        ).await {
            error!("Retry deployment failed for {}: {}", project_name_clone, e);
        }
    });

    (
        StatusCode::OK,
        Json(RetryResponse {
            success: true,
            message: format!(
                "Retry deployment started (attempt #{}, commit: {})",
                retry_attempt.retry_count,
                retry_attempt.commit_sha[..8].to_string()
            ),
            attempt_id: Some(retry_attempt.id),
        }),
    )
}

/// Get deployment status for a project
pub async fn get_deployment_status(
    State(state): State<AppState>,
    Json(req): Json<RetryRequest>,
) -> (StatusCode, Json<StatusResponse>) {
    let tracker = DeploymentTracker::new(state.db.clone());

    // req.project_id is actually project_name from CLI
    match tracker.get_latest_attempt_by_name(&req.project_id) {
        Ok(Some(attempt)) => {
            let status_str = match attempt.status {
                DeploymentStatus::Pending => "pending",
                DeploymentStatus::Running => "running",
                DeploymentStatus::Success => "success",
                DeploymentStatus::Failed => "failed",
            };

            (
                StatusCode::OK,
                Json(StatusResponse {
                    success: true,
                    deployment: Some(DeploymentStatusInfo {
                        id: attempt.id,
                        project_name: attempt.project_name,
                        commit_sha: attempt.commit_sha,
                        status: status_str.to_string(),
                        started_at: attempt.started_at,
                        completed_at: attempt.completed_at,
                        failure_reason: attempt.failure_reason,
                        retry_count: attempt.retry_count,
                    }),
                }),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(StatusResponse {
                success: false,
                deployment: None,
            }),
        ),
        Err(e) => {
            error!("Failed to get deployment status: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(StatusResponse {
                    success: false,
                    deployment: None,
                }),
            )
        }
    }
}

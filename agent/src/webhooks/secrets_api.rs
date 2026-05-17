use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{info, error};

use crate::secrets::{SecretStorage, SecretWithValue, SecretMetadata};
use crate::webhooks::server::AppState;

#[derive(Debug, Deserialize)]
pub struct SetSecretRequest {
    pub project_id: String,
    pub project_name: String,
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default = "default_performed_by")]
    pub performed_by: String,
}

fn default_performed_by() -> String {
    "cli".to_string()
}

#[derive(Debug, Deserialize)]
pub struct GetSecretRequest {
    pub project_id: String,
    pub project_name: String,
    #[serde(default = "default_performed_by")]
    pub performed_by: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteSecretRequest {
    pub project_id: String,
    #[serde(default = "default_performed_by")]
    pub performed_by: String,
}

#[derive(Debug, Deserialize)]
pub struct ListSecretsRequest {
    pub project_id: String,
}

#[derive(Debug, Deserialize)]
pub struct GetAllSecretsRequest {
    pub project_id: String,
    pub project_name: String,
    #[serde(default = "default_performed_by")]
    pub performed_by: String,
}

#[derive(Debug, Serialize)]
pub struct SetSecretResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct GetSecretResponse {
    pub secret: SecretWithValue,
}

#[derive(Debug, Serialize)]
pub struct ListSecretsResponse {
    pub secrets: Vec<SecretMetadata>,
}

#[derive(Debug, Serialize)]
pub struct GetAllSecretsResponse {
    pub secrets: Vec<SecretWithValue>,
}

#[derive(Debug, Serialize)]
pub struct DeleteSecretResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Set a secret value
pub async fn set_secret(
    State(state): State<AppState>,
    Json(payload): Json<SetSecretRequest>,
) -> impl IntoResponse {
    info!("Setting secret '{}' for project '{}'", payload.name, payload.project_name);

    // Get agent ID from environment or generate one
    let agent_id = std::env::var("SHIPWRIGHT_AGENT_ID")
        .unwrap_or_else(|_| {
            // Load from a persistent location or generate
            // For now, use a default - in production, this should be stored
            crate::crypto::generate_agent_id()
        });

    let storage = SecretStorage::new(state.db.clone(), agent_id);

    match storage.set_secret(
        &payload.project_id,
        &payload.project_name,
        &payload.name,
        &payload.value,
        payload.tags,
        &payload.performed_by,
    ) {
        Ok(_) => {
            let response = SetSecretResponse {
                success: true,
                message: format!("Secret '{}' set successfully", payload.name),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            error!("Failed to set secret: {}", e);
            let response = ErrorResponse {
                error: format!("Failed to set secret: {}", e),
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
        }
    }
}

/// Get a specific secret value
pub async fn get_secret(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(payload): Json<GetSecretRequest>,
) -> impl IntoResponse {
    info!("Getting secret '{}' for project '{}'", name, payload.project_name);

    let agent_id = std::env::var("SHIPWRIGHT_AGENT_ID")
        .unwrap_or_else(|_| crate::crypto::generate_agent_id());

    let storage = SecretStorage::new(state.db.clone(), agent_id);

    match storage.get_secret(
        &payload.project_id,
        &payload.project_name,
        &name,
        &payload.performed_by,
    ) {
        Ok(secret) => {
            let response = GetSecretResponse { secret };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            error!("Failed to get secret: {}", e);
            let response = ErrorResponse {
                error: format!("Failed to get secret: {}", e),
            };
            (StatusCode::NOT_FOUND, Json(response)).into_response()
        }
    }
}

/// List all secrets (metadata only)
pub async fn list_secrets(
    State(state): State<AppState>,
    Json(payload): Json<ListSecretsRequest>,
) -> impl IntoResponse {
    info!("Listing secrets for project '{}'", payload.project_id);

    let agent_id = std::env::var("SHIPWRIGHT_AGENT_ID")
        .unwrap_or_else(|_| crate::crypto::generate_agent_id());

    let storage = SecretStorage::new(state.db.clone(), agent_id);

    match storage.list_secrets(&payload.project_id) {
        Ok(secrets) => {
            let response = ListSecretsResponse { secrets };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            error!("Failed to list secrets: {}", e);
            let response = ErrorResponse {
                error: format!("Failed to list secrets: {}", e),
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
        }
    }
}

/// Get all secrets with values
pub async fn get_all_secrets(
    State(state): State<AppState>,
    Json(payload): Json<GetAllSecretsRequest>,
) -> impl IntoResponse {
    info!("Getting all secrets for project '{}'", payload.project_name);

    let agent_id = std::env::var("SHIPWRIGHT_AGENT_ID")
        .unwrap_or_else(|_| crate::crypto::generate_agent_id());

    let storage = SecretStorage::new(state.db.clone(), agent_id);

    match storage.get_all_secrets(
        &payload.project_id,
        &payload.project_name,
        &payload.performed_by,
    ) {
        Ok(secrets) => {
            let response = GetAllSecretsResponse { secrets };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            error!("Failed to get all secrets: {}", e);
            let response = ErrorResponse {
                error: format!("Failed to get all secrets: {}", e),
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
        }
    }
}

/// Delete a secret
pub async fn delete_secret(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(payload): Json<DeleteSecretRequest>,
) -> impl IntoResponse {
    info!("Deleting secret '{}' for project", name);

    let agent_id = std::env::var("SHIPWRIGHT_AGENT_ID")
        .unwrap_or_else(|_| crate::crypto::generate_agent_id());

    let storage = SecretStorage::new(state.db.clone(), agent_id);

    match storage.delete_secret(&payload.project_id, &name, &payload.performed_by) {
        Ok(_) => {
            let response = DeleteSecretResponse {
                success: true,
                message: format!("Secret '{}' deleted successfully", name),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            error!("Failed to delete secret: {}", e);
            let response = ErrorResponse {
                error: format!("Failed to delete secret: {}", e),
            };
            (StatusCode::NOT_FOUND, Json(response)).into_response()
        }
    }
}

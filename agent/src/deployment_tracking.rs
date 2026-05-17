/// Deployment tracking for retry functionality
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentAttempt {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub commit_sha: String,
    pub deploy_dir: String,
    pub config_path: String,
    pub triggered_by: String,
    pub status: DeploymentStatus,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub failure_reason: Option<String>,
    pub failure_details: Option<String>,
    pub retry_count: i32,
    pub original_attempt_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentStatus {
    Pending,
    Running,
    Success,
    Failed,
}

impl DeploymentStatus {
    fn to_string(&self) -> &str {
        match self {
            DeploymentStatus::Pending => "pending",
            DeploymentStatus::Running => "running",
            DeploymentStatus::Success => "success",
            DeploymentStatus::Failed => "failed",
        }
    }

    fn from_string(s: &str) -> Self {
        match s {
            "pending" => DeploymentStatus::Pending,
            "running" => DeploymentStatus::Running,
            "success" => DeploymentStatus::Success,
            "failed" => DeploymentStatus::Failed,
            _ => DeploymentStatus::Failed,
        }
    }
}

pub struct DeploymentTracker {
    conn: Arc<Mutex<Connection>>,
}

impl DeploymentTracker {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Create a new deployment attempt
    pub fn create_attempt(
        &self,
        project_id: &str,
        project_name: &str,
        commit_sha: &str,
        deploy_dir: &str,
        config_path: &str,
        triggered_by: &str,
    ) -> Result<DeploymentAttempt> {
        let conn = self.conn.lock().unwrap();

        let attempt = DeploymentAttempt {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            project_name: project_name.to_string(),
            commit_sha: commit_sha.to_string(),
            deploy_dir: deploy_dir.to_string(),
            config_path: config_path.to_string(),
            triggered_by: triggered_by.to_string(),
            status: DeploymentStatus::Pending,
            started_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            failure_reason: None,
            failure_details: None,
            retry_count: 0,
            original_attempt_id: None,
        };

        conn.execute(
            "INSERT INTO deployment_attempts (
                id, project_id, project_name, commit_sha, deploy_dir, config_path,
                triggered_by, status, started_at, completed_at, failure_reason,
                failure_details, retry_count, original_attempt_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                &attempt.id,
                &attempt.project_id,
                &attempt.project_name,
                &attempt.commit_sha,
                &attempt.deploy_dir,
                &attempt.config_path,
                &attempt.triggered_by,
                attempt.status.to_string(),
                attempt.started_at,
                attempt.completed_at,
                &attempt.failure_reason,
                &attempt.failure_details,
                attempt.retry_count,
                &attempt.original_attempt_id,
            ],
        ).context("Failed to create deployment attempt")?;

        Ok(attempt)
    }

    /// Update deployment status
    pub fn update_status(&self, attempt_id: &str, status: DeploymentStatus) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE deployment_attempts SET status = ?1 WHERE id = ?2",
            params![status.to_string(), attempt_id],
        )?;

        Ok(())
    }

    /// Mark deployment as complete
    pub fn complete_attempt(
        &self,
        attempt_id: &str,
        status: DeploymentStatus,
        failure_reason: Option<String>,
        failure_details: Option<String>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let completed_at = chrono::Utc::now().timestamp();

        conn.execute(
            "UPDATE deployment_attempts
             SET status = ?1, completed_at = ?2, failure_reason = ?3, failure_details = ?4
             WHERE id = ?5",
            params![
                status.to_string(),
                completed_at,
                failure_reason,
                failure_details,
                attempt_id
            ],
        )?;

        Ok(())
    }

    /// Get the latest deployment attempt for a project
    pub fn get_latest_attempt(&self, project_id: &str) -> Result<Option<DeploymentAttempt>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, project_id, project_name, commit_sha, deploy_dir, config_path,
                    triggered_by, status, started_at, completed_at, failure_reason,
                    failure_details, retry_count, original_attempt_id
             FROM deployment_attempts
             WHERE project_id = ?1
             ORDER BY started_at DESC
             LIMIT 1"
        )?;

        match stmt.query_row(params![project_id], |row| {
            Ok(DeploymentAttempt {
                id: row.get(0)?,
                project_id: row.get(1)?,
                project_name: row.get(2)?,
                commit_sha: row.get(3)?,
                deploy_dir: row.get(4)?,
                config_path: row.get(5)?,
                triggered_by: row.get(6)?,
                status: DeploymentStatus::from_string(&row.get::<_, String>(7)?),
                started_at: row.get(8)?,
                completed_at: row.get(9)?,
                failure_reason: row.get(10)?,
                failure_details: row.get(11)?,
                retry_count: row.get(12)?,
                original_attempt_id: row.get(13)?,
            })
        }) {
            Ok(attempt) => Ok(Some(attempt)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get deployment attempt by ID
    pub fn get_attempt(&self, attempt_id: &str) -> Result<Option<DeploymentAttempt>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, project_id, project_name, commit_sha, deploy_dir, config_path,
                    triggered_by, status, started_at, completed_at, failure_reason,
                    failure_details, retry_count, original_attempt_id
             FROM deployment_attempts
             WHERE id = ?1"
        )?;

        match stmt.query_row(params![attempt_id], |row| {
            Ok(DeploymentAttempt {
                id: row.get(0)?,
                project_id: row.get(1)?,
                project_name: row.get(2)?,
                commit_sha: row.get(3)?,
                deploy_dir: row.get(4)?,
                config_path: row.get(5)?,
                triggered_by: row.get(6)?,
                status: DeploymentStatus::from_string(&row.get::<_, String>(7)?),
                started_at: row.get(8)?,
                completed_at: row.get(9)?,
                failure_reason: row.get(10)?,
                failure_details: row.get(11)?,
                retry_count: row.get(12)?,
                original_attempt_id: row.get(13)?,
            })
        }) {
            Ok(attempt) => Ok(Some(attempt)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Create a retry attempt based on a failed deployment
    pub fn create_retry_attempt(&self, original_attempt_id: &str) -> Result<DeploymentAttempt> {
        let original = self.get_attempt(original_attempt_id)?
            .context("Original deployment attempt not found")?;

        let conn = self.conn.lock().unwrap();

        let retry_attempt = DeploymentAttempt {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: original.project_id.clone(),
            project_name: original.project_name.clone(),
            commit_sha: original.commit_sha.clone(),
            deploy_dir: original.deploy_dir.clone(),
            config_path: original.config_path.clone(),
            triggered_by: "retry".to_string(),
            status: DeploymentStatus::Pending,
            started_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            failure_reason: None,
            failure_details: None,
            retry_count: original.retry_count + 1,
            original_attempt_id: Some(original_attempt_id.to_string()),
        };

        conn.execute(
            "INSERT INTO deployment_attempts (
                id, project_id, project_name, commit_sha, deploy_dir, config_path,
                triggered_by, status, started_at, completed_at, failure_reason,
                failure_details, retry_count, original_attempt_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                &retry_attempt.id,
                &retry_attempt.project_id,
                &retry_attempt.project_name,
                &retry_attempt.commit_sha,
                &retry_attempt.deploy_dir,
                &retry_attempt.config_path,
                &retry_attempt.triggered_by,
                retry_attempt.status.to_string(),
                retry_attempt.started_at,
                retry_attempt.completed_at,
                &retry_attempt.failure_reason,
                &retry_attempt.failure_details,
                retry_attempt.retry_count,
                &retry_attempt.original_attempt_id,
            ],
        ).context("Failed to create retry attempt")?;

        Ok(retry_attempt)
    }
}

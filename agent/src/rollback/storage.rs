/// Database storage for rollback snapshots and events

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

use super::{DeploymentSnapshot, RollbackEvent, SnapshotStatus, ServiceDeployment};

pub struct RollbackStorage {
    conn: Arc<Mutex<Connection>>,
}

impl RollbackStorage {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)
            .context("Failed to open database for rollback storage")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Save a deployment snapshot
    pub fn save_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let strategy_str = match snapshot.strategy {
            shipwright_common::config::RollbackStrategy::ImageTagging => "image-tagging",
            shipwright_common::config::RollbackStrategy::GitCommit => "git-commit",
            shipwright_common::config::RollbackStrategy::Snapshot => "snapshot",
            shipwright_common::config::RollbackStrategy::Hybrid => "hybrid",
        };

        let status_str = match snapshot.status {
            SnapshotStatus::Active => "active",
            SnapshotStatus::RolledBack => "rolled_back",
            SnapshotStatus::Failed => "failed",
            SnapshotStatus::Superseded => "superseded",
        };

        let image_tags_json = snapshot.image_tags.as_ref()
            .map(|tags| serde_json::to_string(tags).unwrap());

        conn.execute(
            "INSERT INTO deployment_snapshots (
                id, project_id, commit_sha, deployed_at, status, strategy,
                image_tags, git_branch, git_message, snapshot_path, database_backup_path,
                smoke_test_passed, smoke_test_results, triggered_by, rollback_from_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                &snapshot.id,
                &snapshot.project_id,
                &snapshot.commit_sha,
                snapshot.deployed_at,
                status_str,
                strategy_str,
                image_tags_json,
                &snapshot.git_branch,
                &snapshot.git_message,
                &snapshot.snapshot_path,
                &snapshot.database_backup_path,
                snapshot.smoke_test_passed,
                &snapshot.smoke_test_results,
                &snapshot.triggered_by,
                &snapshot.rollback_from_id,
            ],
        ).context("Failed to save deployment snapshot")?;

        Ok(())
    }

    /// Save a service deployment
    pub fn save_service_deployment(&self, service: &ServiceDeployment) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let health_status_str = match service.health_status {
            super::HealthStatus::Healthy => "healthy",
            super::HealthStatus::Unhealthy => "unhealthy",
            super::HealthStatus::Starting => "starting",
            super::HealthStatus::Failed => "failed",
        };

        let strategy_str = match service.rollback_strategy {
            shipwright_common::config::RollbackStrategy::ImageTagging => "image-tagging",
            shipwright_common::config::RollbackStrategy::GitCommit => "git-commit",
            shipwright_common::config::RollbackStrategy::Snapshot => "snapshot",
            shipwright_common::config::RollbackStrategy::Hybrid => "hybrid",
        };

        conn.execute(
            "INSERT INTO service_deployments (
                id, snapshot_id, service_name, container_id, image_name, image_tag,
                health_status, health_check_output, rollback_strategy, startup_time_ms, memory_usage_mb
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                &service.id,
                &service.snapshot_id,
                &service.service_name,
                &service.container_id,
                &service.image_name,
                &service.image_tag,
                health_status_str,
                &service.health_check_output,
                strategy_str,
                service.startup_time_ms,
                service.memory_usage_mb,
            ],
        ).context("Failed to save service deployment")?;

        Ok(())
    }

    /// Save a rollback event
    pub fn save_rollback_event(&self, event: &RollbackEvent) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let reason_str = match event.reason {
            super::RollbackReason::SmokeTestFailure => "smoke_test_failure",
            super::RollbackReason::Manual => "manual",
            super::RollbackReason::HealthCheckFailure => "health_check_failure",
        };

        conn.execute(
            "INSERT INTO rollback_events (
                id, project_id, from_snapshot_id, to_snapshot_id, reason, failure_details,
                rollback_started_at, rollback_completed_at, rollback_success, performed_by
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &event.id,
                &event.project_id,
                &event.from_snapshot_id,
                &event.to_snapshot_id,
                reason_str,
                &event.failure_details,
                event.rollback_started_at,
                event.rollback_completed_at,
                event.rollback_success,
                &event.performed_by,
            ],
        ).context("Failed to save rollback event")?;

        Ok(())
    }

    /// Get the last successful snapshot for a project
    pub fn get_last_successful_snapshot(&self, project_id: &str) -> Result<Option<DeploymentSnapshot>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, project_id, commit_sha, deployed_at, status, strategy,
                    image_tags, git_branch, git_message, snapshot_path, database_backup_path,
                    smoke_test_passed, smoke_test_results, triggered_by, rollback_from_id
             FROM deployment_snapshots
             WHERE project_id = ?1 AND status = 'active' AND smoke_test_passed = 1
             ORDER BY deployed_at DESC
             LIMIT 1"
        )?;

        match stmt.query_row(params![project_id], |row| {
            match self.row_to_snapshot(row) {
                Ok(snapshot) => Ok(snapshot),
                Err(e) => Err(rusqlite::Error::ExecuteReturnedResults),
            }
        }) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get the currently active snapshot
    pub fn get_active_snapshot(&self, project_id: &str) -> Result<Option<DeploymentSnapshot>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, project_id, commit_sha, deployed_at, status, strategy,
                    image_tags, git_branch, git_message, snapshot_path, database_backup_path,
                    smoke_test_passed, smoke_test_results, triggered_by, rollback_from_id
             FROM deployment_snapshots
             WHERE project_id = ?1 AND status = 'active'
             ORDER BY deployed_at DESC
             LIMIT 1"
        )?;

        match stmt.query_row(params![project_id], |row| {
            match self.row_to_snapshot(row) {
                Ok(snapshot) => Ok(snapshot),
                Err(e) => Err(rusqlite::Error::ExecuteReturnedResults),
            }
        }) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Mark previous snapshots as superseded
    pub fn supersede_previous_snapshots(&self, project_id: &str, except_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE deployment_snapshots
             SET status = 'superseded'
             WHERE project_id = ?1 AND status = 'active' AND id != ?2",
            params![project_id, except_id],
        )?;

        Ok(())
    }

    /// Mark a snapshot as active
    pub fn mark_snapshot_as_active(&self, snapshot_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE deployment_snapshots SET status = 'active' WHERE id = ?1",
            params![snapshot_id],
        )?;

        Ok(())
    }

    /// Mark a snapshot as rolled back
    pub fn mark_snapshot_as_rolled_back(&self, snapshot_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE deployment_snapshots SET status = 'rolled_back' WHERE id = ?1",
            params![snapshot_id],
        )?;

        Ok(())
    }

    /// Complete a rollback event
    pub fn complete_rollback_event(&self, event_id: &str, success: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let completed_at = chrono::Utc::now().timestamp();

        conn.execute(
            "UPDATE rollback_events
             SET rollback_completed_at = ?1, rollback_success = ?2
             WHERE id = ?3",
            params![completed_at, success, event_id],
        )?;

        Ok(())
    }

    /// List all snapshots for a project
    pub fn list_snapshots(&self, project_id: &str) -> Result<Vec<DeploymentSnapshot>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, project_id, commit_sha, deployed_at, status, strategy,
                    image_tags, git_branch, git_message, snapshot_path, database_backup_path,
                    smoke_test_passed, smoke_test_results, triggered_by, rollback_from_id
             FROM deployment_snapshots
             WHERE project_id = ?1
             ORDER BY deployed_at DESC"
        )?;

        let snapshots = stmt.query_map(params![project_id], |row| {
            Ok(self.row_to_snapshot(row))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .collect::<Result<Vec<_>>>()?;

        Ok(snapshots)
    }

    /// Get rollback history for a project
    pub fn get_rollback_history(&self, project_id: &str) -> Result<Vec<RollbackEvent>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, project_id, from_snapshot_id, to_snapshot_id, reason, failure_details,
                    rollback_started_at, rollback_completed_at, rollback_success, performed_by
             FROM rollback_events
             WHERE project_id = ?1
             ORDER BY rollback_started_at DESC"
        )?;

        let events = stmt.query_map(params![project_id], |row| {
            let reason_str: String = row.get(4)?;
            let reason = match reason_str.as_str() {
                "smoke_test_failure" => super::RollbackReason::SmokeTestFailure,
                "manual" => super::RollbackReason::Manual,
                "health_check_failure" => super::RollbackReason::HealthCheckFailure,
                _ => super::RollbackReason::Manual,
            };

            Ok(RollbackEvent {
                id: row.get(0)?,
                project_id: row.get(1)?,
                from_snapshot_id: row.get(2)?,
                to_snapshot_id: row.get(3)?,
                reason,
                failure_details: row.get(5)?,
                rollback_started_at: row.get(6)?,
                rollback_completed_at: row.get(7)?,
                rollback_success: row.get(8)?,
                performed_by: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(events)
    }

    fn row_to_snapshot(&self, row: &rusqlite::Row) -> Result<DeploymentSnapshot> {
        let status_str: String = row.get(4)?;
        let status = match status_str.as_str() {
            "active" => SnapshotStatus::Active,
            "rolled_back" => SnapshotStatus::RolledBack,
            "failed" => SnapshotStatus::Failed,
            "superseded" => SnapshotStatus::Superseded,
            _ => SnapshotStatus::Failed,
        };

        let strategy_str: String = row.get(5)?;
        let strategy = match strategy_str.as_str() {
            "image-tagging" => shipwright_common::config::RollbackStrategy::ImageTagging,
            "git-commit" => shipwright_common::config::RollbackStrategy::GitCommit,
            "snapshot" => shipwright_common::config::RollbackStrategy::Snapshot,
            "hybrid" => shipwright_common::config::RollbackStrategy::Hybrid,
            _ => shipwright_common::config::RollbackStrategy::Hybrid,
        };

        let image_tags_json: Option<String> = row.get(6)?;
        let image_tags = image_tags_json.and_then(|json| serde_json::from_str(&json).ok());

        Ok(DeploymentSnapshot {
            id: row.get(0)?,
            project_id: row.get(1)?,
            commit_sha: row.get(2)?,
            deployed_at: row.get(3)?,
            status,
            strategy,
            image_tags,
            git_branch: row.get(7)?,
            git_message: row.get(8)?,
            snapshot_path: row.get(9)?,
            database_backup_path: row.get(10)?,
            smoke_test_passed: row.get(11)?,
            smoke_test_results: row.get(12)?,
            triggered_by: row.get(13)?,
            rollback_from_id: row.get(14)?,
        })
    }
}

/// Rollback system for deployment failure recovery
///
/// Implements hybrid rollback strategy:
/// - Image tagging for stateless services (5-10s)
/// - Git commit for frontends (2-5min)
/// - Snapshot for stateful services (30-60s)

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn, error};

pub mod image_tagging;
pub mod git_commit;
pub mod snapshot;
pub mod storage;

use crate::pipeline::deploy::DeploymentContext;
use shipwright_common::config::RollbackStrategy;

/// Deployment snapshot metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentSnapshot {
    pub id: String,
    pub project_id: String,
    pub commit_sha: String,
    pub deployed_at: i64,
    pub status: SnapshotStatus,
    pub strategy: RollbackStrategy,

    // Image information for image-tagging strategy
    pub image_tags: Option<HashMap<String, String>>,

    // Git information for git-commit strategy
    pub git_branch: Option<String>,
    pub git_message: Option<String>,

    // Snapshot paths for snapshot strategy
    pub snapshot_path: Option<String>,
    pub database_backup_path: Option<String>,

    // Test results
    pub smoke_test_passed: Option<bool>,
    pub smoke_test_results: Option<String>,

    // Metadata
    pub triggered_by: String,
    pub rollback_from_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotStatus {
    Active,
    RolledBack,
    Failed,
    Superseded,
}

/// Service-specific deployment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDeployment {
    pub id: String,
    pub snapshot_id: String,
    pub service_name: String,

    // Container state
    pub container_id: Option<String>,
    pub image_name: Option<String>,
    pub image_tag: Option<String>,

    // Health status
    pub health_status: HealthStatus,
    pub health_check_output: Option<String>,

    // Rollback strategy for this specific service
    pub rollback_strategy: RollbackStrategy,

    // Performance metrics
    pub startup_time_ms: Option<i64>,
    pub memory_usage_mb: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Starting,
    Failed,
}

/// Rollback event for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackEvent {
    pub id: String,
    pub project_id: String,
    pub from_snapshot_id: String,
    pub to_snapshot_id: String,

    pub reason: RollbackReason,
    pub failure_details: Option<String>,

    pub rollback_started_at: i64,
    pub rollback_completed_at: Option<i64>,
    pub rollback_success: Option<bool>,

    pub performed_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackReason {
    SmokeTestFailure,
    Manual,
    HealthCheckFailure,
}

/// Trait for rollback strategy implementations
#[async_trait]
pub trait RollbackStrategyImpl: Send + Sync {
    /// Create a snapshot of the current deployment
    async fn create_snapshot(&self, ctx: &DeploymentContext) -> Result<DeploymentSnapshot>;

    /// Rollback to a previous snapshot
    async fn rollback_to_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<()>;

    /// Verify snapshot integrity
    async fn verify_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<bool>;

    /// Estimate rollback time in seconds
    fn estimated_rollback_time(&self) -> u64;
}

/// Rollback manager that coordinates different strategies
pub struct RollbackManager {
    storage: storage::RollbackStorage,
    strategies: HashMap<RollbackStrategy, Box<dyn RollbackStrategyImpl>>,
}

impl RollbackManager {
    pub fn new(db_path: &str) -> Result<Self> {
        let storage = storage::RollbackStorage::new(db_path)?;

        let mut strategies: HashMap<RollbackStrategy, Box<dyn RollbackStrategyImpl>> = HashMap::new();
        strategies.insert(
            RollbackStrategy::ImageTagging,
            Box::new(image_tagging::ImageTaggingStrategy),
        );
        strategies.insert(
            RollbackStrategy::GitCommit,
            Box::new(git_commit::GitCommitStrategy),
        );
        strategies.insert(
            RollbackStrategy::Snapshot,
            Box::new(snapshot::SnapshotStrategy),
        );

        Ok(Self { storage, strategies })
    }

    /// Create a deployment snapshot using the appropriate strategy
    pub async fn create_snapshot(
        &self,
        ctx: &DeploymentContext,
        strategy: RollbackStrategy,
    ) -> Result<DeploymentSnapshot> {
        let actual_strategy = match strategy {
            RollbackStrategy::Hybrid => self.determine_hybrid_strategy(ctx),
            other => other,
        };

        info!("Creating deployment snapshot with strategy: {:?}", actual_strategy);

        let strategy_impl = self.strategies.get(&actual_strategy)
            .context("Strategy not implemented")?;

        let mut snapshot = strategy_impl.create_snapshot(ctx).await?;
        snapshot.strategy = actual_strategy;

        // Store snapshot in database
        self.storage.save_snapshot(&snapshot)?;

        // Mark previous active snapshots as superseded
        self.storage.supersede_previous_snapshots(&snapshot.project_id, &snapshot.id)?;

        Ok(snapshot)
    }

    /// Rollback to the last successful deployment
    pub async fn rollback_to_previous(
        &self,
        project_id: &str,
        reason: RollbackReason,
        performed_by: &str,
    ) -> Result<()> {
        // Get the last successful snapshot
        let target_snapshot = self.storage.get_last_successful_snapshot(project_id)?
            .context("No previous successful deployment found")?;

        info!(
            "Rolling back project {} to snapshot {} (deployed at {})",
            project_id,
            target_snapshot.id,
            target_snapshot.deployed_at
        );

        // Get current snapshot to record the rollback event
        let current_snapshot = self.storage.get_active_snapshot(project_id)?
            .context("No active deployment found")?;

        // Create rollback event
        let event = RollbackEvent {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            from_snapshot_id: current_snapshot.id.clone(),
            to_snapshot_id: target_snapshot.id.clone(),
            reason,
            failure_details: None,
            rollback_started_at: chrono::Utc::now().timestamp(),
            rollback_completed_at: None,
            rollback_success: None,
            performed_by: performed_by.to_string(),
        };

        self.storage.save_rollback_event(&event)?;

        // Execute rollback
        let strategy = self.strategies.get(&target_snapshot.strategy)
            .context("Strategy not implemented")?;

        match strategy.rollback_to_snapshot(&target_snapshot).await {
            Ok(_) => {
                info!("Rollback completed successfully");

                // Update statuses
                self.storage.mark_snapshot_as_active(&target_snapshot.id)?;
                self.storage.mark_snapshot_as_rolled_back(&current_snapshot.id)?;
                self.storage.complete_rollback_event(&event.id, true)?;

                Ok(())
            }
            Err(e) => {
                error!("Rollback failed: {:#}", e);
                self.storage.complete_rollback_event(&event.id, false)?;
                Err(e)
            }
        }
    }

    /// Determine which strategy to use in hybrid mode
    fn determine_hybrid_strategy(&self, ctx: &DeploymentContext) -> RollbackStrategy {
        // Check if we have database migrations
        let has_migrations = self.has_database_migrations(ctx);

        // Check if we have a database service
        let has_database = self.has_database_service(ctx);

        // Auto-detect based on service characteristics
        if has_migrations || has_database {
            info!("Using Snapshot strategy (detected database/migrations)");
            RollbackStrategy::Snapshot
        } else if ctx.project_name.contains("frontend") ||
                  ctx.project_name.contains("web") ||
                  ctx.project_name.contains("ui") {
            info!("Using GitCommit strategy (detected frontend service)");
            RollbackStrategy::GitCommit
        } else {
            info!("Using ImageTagging strategy (stateless service)");
            RollbackStrategy::ImageTagging
        }
    }

    fn has_database_migrations(&self, ctx: &DeploymentContext) -> bool {
        // Check for common migration directories
        let migration_paths = [
            "migrations",
            "db/migrations",
            "database/migrations",
            "alembic/versions",
            "prisma/migrations",
        ];

        migration_paths.iter().any(|path| {
            std::path::Path::new(&ctx.deploy_dir)
                .join(path)
                .exists()
        })
    }

    fn has_database_service(&self, ctx: &DeploymentContext) -> bool {
        // This would check the compose file for database services
        // For now, return false - can be enhanced
        false
    }

    /// List all snapshots for a project
    pub fn list_snapshots(&self, project_id: &str) -> Result<Vec<DeploymentSnapshot>> {
        self.storage.list_snapshots(project_id)
    }

    /// Get rollback history for a project
    pub fn get_rollback_history(&self, project_id: &str) -> Result<Vec<RollbackEvent>> {
        self.storage.get_rollback_history(project_id)
    }
}

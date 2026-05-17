/// Git commit rollback strategy
///
/// Medium-speed rollback (2-5min) for frontend services by checking out previous commit and rebuilding
///
/// How it works:
/// 1. Before deployment: Record current git commit SHA and branch
/// 2. Deploy new version
/// 3. On failure: Checkout previous commit, rebuild, and redeploy

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::{info, warn};

use super::{DeploymentSnapshot, RollbackStrategyImpl, SnapshotStatus};
use crate::pipeline::deploy::DeploymentContext;

pub struct GitCommitStrategy;

#[async_trait]
impl RollbackStrategyImpl for GitCommitStrategy {
    async fn create_snapshot(&self, ctx: &DeploymentContext) -> Result<DeploymentSnapshot> {
        info!("Creating git-commit snapshot for project {}", ctx.project_name);

        // Get current git commit and branch
        let deploy_dir = std::path::Path::new(&ctx.deploy_dir);

        let commit_sha = self.get_current_commit(deploy_dir)?;
        let branch = self.get_current_branch(deploy_dir)?;
        let commit_message = self.get_commit_message(deploy_dir, &commit_sha)?;

        info!(
            "Snapshot git state: {} on branch {} ({})",
            &commit_sha[..8],
            branch,
            commit_message.lines().next().unwrap_or("no message")
        );

        Ok(DeploymentSnapshot {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: ctx.project_name.clone(),
            commit_sha: commit_sha.clone(),
            deployed_at: chrono::Utc::now().timestamp(),
            status: SnapshotStatus::Active,
            strategy: shipwright_common::config::RollbackStrategy::GitCommit,
            image_tags: None,
            git_branch: Some(branch),
            git_message: Some(commit_message),
            snapshot_path: None,
            database_backup_path: None,
            smoke_test_passed: None,
            smoke_test_results: None,
            triggered_by: "auto".to_string(),
            rollback_from_id: None,
        })
    }

    async fn rollback_to_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<()> {
        info!("Rolling back using git-commit strategy to snapshot {}", snapshot.id);

        let deploy_dir = std::path::Path::new("/home")
            .join("user")
            .join("apps")
            .join(&snapshot.project_id);

        // Checkout the previous commit
        info!("Checking out commit {}", &snapshot.commit_sha[..8]);

        let output = tokio::process::Command::new("git")
            .args(&["checkout", &snapshot.commit_sha])
            .current_dir(&deploy_dir)
            .output()
            .await
            .context("Failed to checkout git commit")?;

        if !output.status.success() {
            anyhow::bail!(
                "Git checkout failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Rebuild the project
        info!("Rebuilding project...");

        let output = tokio::process::Command::new("docker")
            .args(&["compose", "build"])
            .current_dir(&deploy_dir)
            .output()
            .await
            .context("Failed to rebuild project")?;

        if !output.status.success() {
            anyhow::bail!(
                "Docker compose build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Restart services
        info!("Restarting services...");

        let output = tokio::process::Command::new("docker")
            .args(&["compose", "up", "-d", "--force-recreate"])
            .current_dir(&deploy_dir)
            .output()
            .await
            .context("Failed to restart services")?;

        if !output.status.success() {
            anyhow::bail!(
                "Docker compose up failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        info!("Git-commit rollback completed");
        Ok(())
    }

    async fn verify_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<bool> {
        // Verify that the commit exists in the repository
        let deploy_dir = std::path::Path::new("/home")
            .join("user")
            .join("apps")
            .join(&snapshot.project_id);

        if !deploy_dir.exists() {
            warn!("Deploy directory does not exist: {:?}", deploy_dir);
            return Ok(false);
        }

        let output = tokio::process::Command::new("git")
            .args(&["cat-file", "-e", &snapshot.commit_sha])
            .current_dir(&deploy_dir)
            .output()
            .await
            .context("Failed to verify commit")?;

        Ok(output.status.success())
    }

    fn estimated_rollback_time(&self) -> u64 {
        // Git checkout + rebuild typically takes 2-5 minutes
        180
    }
}

impl GitCommitStrategy {
    fn get_current_commit(&self, deploy_dir: &std::path::Path) -> Result<String> {
        let output = std::process::Command::new("git")
            .args(&["rev-parse", "HEAD"])
            .current_dir(deploy_dir)
            .output()
            .context("Failed to get current commit")?;

        if !output.status.success() {
            anyhow::bail!("Git rev-parse failed");
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn get_current_branch(&self, deploy_dir: &std::path::Path) -> Result<String> {
        let output = std::process::Command::new("git")
            .args(&["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(deploy_dir)
            .output()
            .context("Failed to get current branch")?;

        if !output.status.success() {
            anyhow::bail!("Git rev-parse failed");
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn get_commit_message(&self, deploy_dir: &std::path::Path, commit_sha: &str) -> Result<String> {
        let output = std::process::Command::new("git")
            .args(&["log", "-1", "--pretty=%B", commit_sha])
            .current_dir(deploy_dir)
            .output()
            .context("Failed to get commit message")?;

        if !output.status.success() {
            return Ok("No commit message".to_string());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

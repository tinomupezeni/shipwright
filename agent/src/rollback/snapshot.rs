/// Snapshot rollback strategy
///
/// Full snapshot rollback (30-60s) for stateful services with database backups
///
/// How it works:
/// 1. Before deployment: Create volume snapshots and database backups
/// 2. Deploy new version
/// 3. On failure: Restore volume snapshots and database backups, restart containers

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::{info, warn};

use super::{DeploymentSnapshot, RollbackStrategyImpl, SnapshotStatus};
use crate::pipeline::deploy::DeploymentContext;

pub struct SnapshotStrategy;

#[async_trait]
impl RollbackStrategyImpl for SnapshotStrategy {
    async fn create_snapshot(&self, ctx: &DeploymentContext) -> Result<DeploymentSnapshot> {
        info!("Creating full snapshot for project {}", ctx.project_name);

        let timestamp = chrono::Utc::now().timestamp();
        let snapshot_id = uuid::Uuid::new_v4().to_string();

        // Create snapshot directory
        let snapshot_dir = std::path::Path::new("/var/lib/shipwright/snapshots")
            .join(&snapshot_id);

        tokio::fs::create_dir_all(&snapshot_dir)
            .await
            .context("Failed to create snapshot directory")?;

        // Snapshot Docker volumes
        let volume_snapshot_path = self.snapshot_volumes(ctx, &snapshot_dir).await?;

        // Backup database if exists
        let db_backup_path = self.backup_database(ctx, &snapshot_dir).await.ok();

        info!(
            "Created snapshot at {:?} (volumes: {:?}, db: {:?})",
            snapshot_dir,
            volume_snapshot_path,
            db_backup_path
        );

        Ok(DeploymentSnapshot {
            id: snapshot_id,
            project_id: ctx.project_name.clone(),
            commit_sha: ctx.commit_sha.clone().unwrap_or_else(|| "unknown".to_string()),
            deployed_at: timestamp,
            status: SnapshotStatus::Active,
            strategy: shipwright_common::config::RollbackStrategy::Snapshot,
            image_tags: None,
            git_branch: None,
            git_message: None,
            snapshot_path: Some(snapshot_dir.to_string_lossy().to_string()),
            database_backup_path: db_backup_path,
            smoke_test_passed: None,
            smoke_test_results: None,
            triggered_by: "auto".to_string(),
            rollback_from_id: None,
        })
    }

    async fn rollback_to_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<()> {
        info!("Rolling back using snapshot strategy to snapshot {}", snapshot.id);

        let snapshot_path = snapshot.snapshot_path.as_ref()
            .context("No snapshot path found")?;

        // Detect infrastructure to find where the project is currently located
        let infrastructure = crate::infrastructure::detect_infrastructure().await?;
        let deploy_dir = crate::infrastructure::detector::recommend_deploy_dir(&infrastructure, &snapshot.project_id);

        info!("Using deployment directory: {}", deploy_dir);

        // Stop containers
        info!("Stopping containers for project {}", snapshot.project_id);
        self.stop_project_containers(&deploy_dir).await?;

        // Restore volumes
        info!("Restoring volumes from snapshot");
        self.restore_volumes(&snapshot.project_id, snapshot_path).await?;

        // Restore database if backup exists
        if let Some(db_backup_path) = &snapshot.database_backup_path {
            info!("Restoring database from backup");
            self.restore_database(&snapshot.project_id, db_backup_path).await?;
        }

        // Restart containers
        info!("Restarting containers");
        self.restart_project_containers(&deploy_dir).await?;

        info!("Snapshot rollback completed");
        Ok(())
    }

    async fn verify_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<bool> {
        // Verify snapshot files exist
        let snapshot_path = match &snapshot.snapshot_path {
            Some(path) => path,
            None => return Ok(false),
        };

        let path = std::path::Path::new(snapshot_path);
        if !path.exists() {
            warn!("Snapshot path does not exist: {:?}", path);
            return Ok(false);
        }

        // Verify database backup if it should exist
        if let Some(db_backup_path) = &snapshot.database_backup_path {
            let db_path = std::path::Path::new(db_backup_path);
            if !db_path.exists() {
                warn!("Database backup does not exist: {:?}", db_path);
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn estimated_rollback_time(&self) -> u64 {
        // Snapshot restoration typically takes 30-60 seconds
        45
    }
}

impl SnapshotStrategy {
    async fn snapshot_volumes(
        &self,
        ctx: &DeploymentContext,
        snapshot_dir: &std::path::Path,
    ) -> Result<String> {
        let docker = bollard::Docker::connect_with_socket_defaults()
            .context("Failed to connect to Docker")?;

        // Get volumes used by this project
        let volumes = docker
            .list_volumes::<String>(None)
            .await
            .context("Failed to list volumes")?;

        let volume_snapshot_dir = snapshot_dir.join("volumes");
        tokio::fs::create_dir_all(&volume_snapshot_dir).await?;

        for volume in volumes.volumes.unwrap_or_default() {
            if volume.name.contains(&ctx.project_name) {
                info!("Snapshotting volume: {}", volume.name);

                // Copy volume data using a temporary container
                let volume_backup = volume_snapshot_dir.join(&volume.name);
                tokio::fs::create_dir_all(&volume_backup).await?;

                // Use tar to backup volume data
                let output = tokio::process::Command::new("docker")
                    .args(&[
                        "run",
                        "--rm",
                        "-v",
                        &format!("{}:/source", volume.name),
                        "-v",
                        &format!("{}:/backup", volume_backup.to_string_lossy()),
                        "alpine",
                        "sh",
                        "-c",
                        "cd /source && tar czf /backup/data.tar.gz .",
                    ])
                    .output()
                    .await
                    .context("Failed to backup volume")?;

                if !output.status.success() {
                    warn!(
                        "Failed to backup volume {}: {}",
                        volume.name,
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }

        Ok(volume_snapshot_dir.to_string_lossy().to_string())
    }

    async fn backup_database(
        &self,
        ctx: &DeploymentContext,
        snapshot_dir: &std::path::Path,
    ) -> Result<String> {
        // Try to find a database container
        let docker = bollard::Docker::connect_with_socket_defaults()
            .context("Failed to connect to Docker")?;

        let containers = docker
            .list_containers::<String>(None)
            .await
            .context("Failed to list containers")?;

        // Look for common database containers (postgres, mysql, mongodb)
        for container in containers {
            if let Some(names) = container.names {
                for name in names {
                    if name.contains(&ctx.project_name) {
                        if let Some(image) = &container.image {
                            let db_backup_path = if image.contains("postgres") {
                                self.backup_postgres(&name, snapshot_dir).await.ok()
                            } else if image.contains("mysql") || image.contains("mariadb") {
                                self.backup_mysql(&name, snapshot_dir).await.ok()
                            } else if image.contains("mongo") {
                                self.backup_mongodb(&name, snapshot_dir).await.ok()
                            } else {
                                None
                            };

                            if let Some(path) = db_backup_path {
                                return Ok(path);
                            }
                        }
                    }
                }
            }
        }

        anyhow::bail!("No database container found")
    }

    async fn backup_postgres(
        &self,
        container_name: &str,
        snapshot_dir: &std::path::Path,
    ) -> Result<String> {
        let backup_file = snapshot_dir.join("postgres_backup.sql");

        info!("Backing up PostgreSQL database from {}", container_name);

        let output = tokio::process::Command::new("docker")
            .args(&[
                "exec",
                container_name,
                "pg_dumpall",
                "-U",
                "postgres",
            ])
            .output()
            .await
            .context("Failed to run pg_dumpall")?;

        if !output.status.success() {
            anyhow::bail!(
                "pg_dumpall failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        tokio::fs::write(&backup_file, output.stdout).await?;

        Ok(backup_file.to_string_lossy().to_string())
    }

    async fn backup_mysql(
        &self,
        container_name: &str,
        snapshot_dir: &std::path::Path,
    ) -> Result<String> {
        let backup_file = snapshot_dir.join("mysql_backup.sql");

        info!("Backing up MySQL database from {}", container_name);

        let output = tokio::process::Command::new("docker")
            .args(&[
                "exec",
                container_name,
                "mysqldump",
                "--all-databases",
                "-u",
                "root",
            ])
            .output()
            .await
            .context("Failed to run mysqldump")?;

        if !output.status.success() {
            anyhow::bail!(
                "mysqldump failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        tokio::fs::write(&backup_file, output.stdout).await?;

        Ok(backup_file.to_string_lossy().to_string())
    }

    async fn backup_mongodb(
        &self,
        container_name: &str,
        snapshot_dir: &std::path::Path,
    ) -> Result<String> {
        let backup_dir = snapshot_dir.join("mongodb_backup");
        tokio::fs::create_dir_all(&backup_dir).await?;

        info!("Backing up MongoDB database from {}", container_name);

        let output = tokio::process::Command::new("docker")
            .args(&[
                "exec",
                container_name,
                "mongodump",
                "--out",
                "/backup",
            ])
            .output()
            .await
            .context("Failed to run mongodump")?;

        if !output.status.success() {
            anyhow::bail!(
                "mongodump failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(backup_dir.to_string_lossy().to_string())
    }

    async fn restore_volumes(
        &self,
        project_name: &str,
        snapshot_path: &str,
    ) -> Result<()> {
        let volume_snapshot_dir = std::path::Path::new(snapshot_path).join("volumes");

        if !volume_snapshot_dir.exists() {
            warn!("No volume snapshots found");
            return Ok(());
        }

        // Restore each volume
        let mut entries = tokio::fs::read_dir(&volume_snapshot_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let volume_name = entry.file_name().to_string_lossy().to_string();
            let backup_file = entry.path().join("data.tar.gz");

            if backup_file.exists() {
                info!("Restoring volume: {}", volume_name);

                let output = tokio::process::Command::new("docker")
                    .args(&[
                        "run",
                        "--rm",
                        "-v",
                        &format!("{}:/target", volume_name),
                        "-v",
                        &format!("{}:/backup", entry.path().to_string_lossy()),
                        "alpine",
                        "sh",
                        "-c",
                        "cd /target && tar xzf /backup/data.tar.gz",
                    ])
                    .output()
                    .await
                    .context("Failed to restore volume")?;

                if !output.status.success() {
                    warn!(
                        "Failed to restore volume {}: {}",
                        volume_name,
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }

        Ok(())
    }

    async fn restore_database(
        &self,
        project_name: &str,
        db_backup_path: &str,
    ) -> Result<()> {
        let backup_path = std::path::Path::new(db_backup_path);
        
        // Find the appropriate database container for this project
        let db_container = self.find_db_container(project_name).await.ok();
        
        if let Some(container_name) = db_container {
            if backup_path.join("postgres_backup.sql").exists() {
                self.restore_postgres(&container_name, &backup_path.join("postgres_backup.sql")).await?;
            } else if backup_path.join("mysql_backup.sql").exists() {
                self.restore_mysql(&container_name, &backup_path.join("mysql_backup.sql")).await?;
            } else if backup_path.join("mongodb_backup").exists() {
                self.restore_mongodb(&container_name, &backup_path.join("mongodb_backup")).await?;
            }
        } else {
            warn!("No database container found for restoration of project {}", project_name);
            
            // Fallback: try default names if discovery failed but backup exists
            if backup_path.join("postgres_backup.sql").exists() {
                let fallback = format!("{}-db", project_name);
                self.restore_postgres(&fallback, &backup_path.join("postgres_backup.sql")).await?;
            }
        }

        Ok(())
    }

    async fn find_db_container(&self, project_name: &str) -> Result<String> {
        let docker = bollard::Docker::connect_with_socket_defaults()?;
        let containers = docker.list_containers::<String>(None).await?;

        for container in containers {
            if let Some(names) = container.names {
                for name in names {
                    let name = name.trim_start_matches('/');
                    if name.contains(project_name) {
                        if let Some(image) = &container.image {
                            let image = image.to_lowercase();
                            if image.contains("postgres") || image.contains("mysql") || 
                               image.contains("mariadb") || image.contains("mongo") {
                                return Ok(name.to_string());
                            }
                        }
                        
                        // Also check name for database keywords
                        let name_lower = name.to_lowercase();
                        if name_lower.contains("db") || name_lower.contains("postgres") || 
                           name_lower.contains("mysql") || name_lower.contains("mongo") {
                            return Ok(name.to_string());
                        }
                    }
                }
            }
        }

        anyhow::bail!("No database container found")
    }

    async fn restore_postgres(&self, container_name: &str, backup_file: &std::path::Path) -> Result<()> {
        info!("Restoring PostgreSQL database to {}", container_name);

        let backup_content = tokio::fs::read_to_string(backup_file).await?;

        let mut child = tokio::process::Command::new("docker")
            .args(&["exec", "-i", container_name, "psql", "-U", "postgres"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn psql")?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(backup_content.as_bytes()).await?;
        }

        let output = child.wait_with_output().await?;

        if !output.status.success() {
            anyhow::bail!(
                "psql restore failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    async fn restore_mysql(&self, container_name: &str, backup_file: &std::path::Path) -> Result<()> {
        info!("Restoring MySQL database to {}", container_name);

        let backup_content = tokio::fs::read_to_string(backup_file).await?;

        let mut child = tokio::process::Command::new("docker")
            .args(&["exec", "-i", container_name, "mysql", "-u", "root"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn mysql")?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(backup_content.as_bytes()).await?;
        }

        let output = child.wait_with_output().await?;

        if !output.status.success() {
            anyhow::bail!(
                "mysql restore failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    async fn restore_mongodb(&self, container_name: &str, backup_dir: &std::path::Path) -> Result<()> {
        info!("Restoring MongoDB database to {}", container_name);

        let output = tokio::process::Command::new("docker")
            .args(&[
                "exec",
                container_name,
                "mongorestore",
                &backup_dir.to_string_lossy(),
            ])
            .output()
            .await
            .context("Failed to run mongorestore")?;

        if !output.status.success() {
            anyhow::bail!(
                "mongorestore failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    async fn stop_project_containers(&self, deploy_dir: &str) -> Result<()> {
        let output = tokio::process::Command::new("docker")
            .args(&["compose", "down"])
            .current_dir(deploy_dir)
            .output()
            .await
            .context("Failed to stop containers")?;

        if !output.status.success() {
            anyhow::bail!(
                "docker compose down failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    async fn restart_project_containers(&self, deploy_dir: &str) -> Result<()> {
        let output = tokio::process::Command::new("docker")
            .args(&["compose", "up", "-d"])
            .current_dir(deploy_dir)
            .output()
            .await
            .context("Failed to restart containers")?;

        if !output.status.success() {
            anyhow::bail!(
                "docker compose up failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }
}

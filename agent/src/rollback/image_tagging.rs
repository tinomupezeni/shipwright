/// Image tagging rollback strategy
///
/// Fast rollback (5-10s) for stateless services by re-tagging Docker images
///
/// How it works:
/// 1. Before deployment: Tag current images as "rollback-<timestamp>"
/// 2. Deploy new version with "latest" tag
/// 3. On failure: Re-tag "rollback-<timestamp>" as "latest" and restart containers

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use tracing::{info, warn};

use super::{DeploymentSnapshot, RollbackStrategyImpl, SnapshotStatus};
use crate::pipeline::deploy::DeploymentContext;

pub struct ImageTaggingStrategy;

#[async_trait]
impl RollbackStrategyImpl for ImageTaggingStrategy {
    async fn create_snapshot(&self, ctx: &DeploymentContext) -> Result<DeploymentSnapshot> {
        info!("Creating image-tagging snapshot for project {}", ctx.project_name);

        // Get current running containers and their images
        let docker = bollard::Docker::connect_with_socket_defaults()
            .context("Failed to connect to Docker")?;

        let containers = docker
            .list_containers::<String>(None)
            .await
            .context("Failed to list containers")?;

        let mut image_tags = HashMap::new();

        // Find containers for this project
        for container in containers {
            if let Some(names) = container.names {
                for name in names {
                    if name.contains(&ctx.project_name) {
                        if let Some(ref image) = container.image {
                            // Extract service name from container name
                            let service_name = name
                                .trim_start_matches('/')
                                .trim_start_matches(&format!("{}-", ctx.project_name))
                                .to_string();

                            // Tag current image for rollback
                            let rollback_tag = format!("rollback-{}", chrono::Utc::now().timestamp());

                            // Parse image name and tag
                            let (image_name, current_tag) = if let Some(pos) = image.rfind(':') {
                                (&image[..pos], &image[pos + 1..])
                            } else {
                                (image.as_str(), "latest")
                            };

                            // Create rollback tag
                            let rollback_image = format!("{}:{}", image_name, rollback_tag);

                            info!("Tagging {} as {}", image, rollback_image);

                            // Tag the image
                            if let Err(e) = docker
                                .tag_image(
                                    &image,
                                    Some(bollard::image::TagImageOptions {
                                        repo: image_name.to_string(),
                                        tag: rollback_tag.clone(),
                                    }),
                                )
                                .await
                            {
                                warn!("Failed to tag image {}: {}", image, e);
                            } else {
                                image_tags.insert(service_name, rollback_tag);
                            }
                        }
                    }
                }
            }
        }

        Ok(DeploymentSnapshot {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: ctx.project_name.clone(),
            commit_sha: ctx.commit_sha.clone().unwrap_or_else(|| "unknown".to_string()),
            deployed_at: chrono::Utc::now().timestamp(),
            status: SnapshotStatus::Active,
            strategy: shipwright_common::config::RollbackStrategy::ImageTagging,
            image_tags: Some(image_tags),
            git_branch: None,
            git_message: None,
            snapshot_path: None,
            database_backup_path: None,
            smoke_test_passed: None,
            smoke_test_results: None,
            triggered_by: "auto".to_string(),
            rollback_from_id: None,
        })
    }

    async fn rollback_to_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<()> {
        info!("Rolling back using image-tagging strategy to snapshot {}", snapshot.id);

        let image_tags = snapshot.image_tags.as_ref()
            .context("No image tags found in snapshot")?;

        let docker = bollard::Docker::connect_with_socket_defaults()
            .context("Failed to connect to Docker")?;

        // For each service, re-tag the rollback image and restart container
        for (service_name, rollback_tag) in image_tags {
            info!("Rolling back service {} to tag {}", service_name, rollback_tag);

            // Find the container
            let container_name = format!("{}-{}", snapshot.project_id, service_name);

            // Get container info to find the image name
            if let Ok(container_info) = docker.inspect_container(&container_name, None).await {
                if let Some(config) = container_info.config {
                    if let Some(image) = config.image {
                        // Parse image name
                        let image_name = if let Some(pos) = image.rfind(':') {
                            &image[..pos]
                        } else {
                            image.as_str()
                        };

                        let rollback_image = format!("{}:{}", image_name, rollback_tag);

                        // Tag the rollback image as latest
                        info!("Tagging {} as latest", rollback_image);
                        docker
                            .tag_image(
                                &rollback_image,
                                Some(bollard::image::TagImageOptions {
                                    repo: image_name.to_string(),
                                    tag: "latest".to_string(),
                                }),
                            )
                            .await
                            .context("Failed to re-tag image")?;

                        // Restart the container
                        info!("Restarting container {}", container_name);
                        docker
                            .restart_container(&container_name, None)
                            .await
                            .context("Failed to restart container")?;
                    }
                }
            }
        }

        info!("Image-tagging rollback completed");
        Ok(())
    }

    async fn verify_snapshot(&self, snapshot: &DeploymentSnapshot) -> Result<bool> {
        // Verify that the tagged images still exist
        let image_tags = match &snapshot.image_tags {
            Some(tags) => tags,
            None => return Ok(false),
        };

        let docker = bollard::Docker::connect_with_socket_defaults()
            .context("Failed to connect to Docker")?;

        for (_service_name, tag) in image_tags {
            // List images to see if the tag exists
            let images = docker
                .list_images::<String>(None)
                .await
                .context("Failed to list images")?;

            let tag_exists = images.iter().any(|img| {
                img.repo_tags.iter().any(|repo_tag| {
                    repo_tag.contains(tag)
                })
            });

            if !tag_exists {
                warn!("Rollback image tag {} not found", tag);
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn estimated_rollback_time(&self) -> u64 {
        // Image tagging is fast - typically 5-10 seconds
        10
    }
}

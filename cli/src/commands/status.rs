use anyhow::{Result, Context};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shipwright_common::config::Config;
use std::fs;
use std::path::Path;
use chrono::{DateTime, Utc, TimeZone};

#[derive(Serialize)]
struct StatusRequest {
    project_id: String,
}

#[derive(Deserialize)]
struct StatusResponse {
    success: bool,
    deployment: Option<DeploymentInfo>,
}

#[derive(Deserialize)]
struct DeploymentInfo {
    id: String,
    project_name: String,
    commit_sha: String,
    status: String,
    started_at: i64,
    completed_at: Option<i64>,
    failure_reason: Option<String>,
    retry_count: i32,
}

fn get_api_url(host: &str) -> String {
    format!("http://{}:17670", host)
}

fn format_timestamp(timestamp: i64) -> String {
    let dt: DateTime<Utc> = Utc.timestamp_opt(timestamp, 0).unwrap();
    let now = Utc::now();
    let duration = now.signed_duration_since(dt);

    if duration.num_seconds() < 60 {
        format!("{} seconds ago", duration.num_seconds())
    } else if duration.num_minutes() < 60 {
        format!("{} minutes ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{} hours ago", duration.num_hours())
    } else {
        format!("{} days ago", duration.num_days())
    }
}

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");
    if !config_path.exists() {
        println!(".shipwright.yml not found. Run 'shipwright init' first.");
        return Ok(());
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;

    let vps = config.deploy.vps.as_ref()
        .context("No VPS configuration found in .shipwright.yml")?;

    println!("📊 Deployment Status for {}\n", config.project.name);
    println!("═══════════════════════════════════════════════════════");

    // Get deployment status from agent
    let client = Client::new();
    let url = format!("{}/api/v1/deployments/status", get_api_url(&vps.host));

    let request = StatusRequest {
        project_id: config.project.name.clone(),
    };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => {
            let status_response: StatusResponse = resp.json().await
                .context("Failed to parse response from agent")?;

            if let Some(deployment) = status_response.deployment {
                // Status emoji and color
                let (status_icon, status_text) = match deployment.status.as_str() {
                    "success" => ("✅", "SUCCESS"),
                    "failed" => ("❌", "FAILED"),
                    "running" => ("🔄", "RUNNING"),
                    "pending" => ("⏳", "PENDING"),
                    _ => ("❓", "UNKNOWN"),
                };

                println!("Status:        {} {}", status_icon, status_text);
                println!("Project:       {}", deployment.project_name);
                println!("Commit:        {}", &deployment.commit_sha[..8]);
                println!("Started:       {}", format_timestamp(deployment.started_at));

                if let Some(completed_at) = deployment.completed_at {
                    let started_dt: DateTime<Utc> = Utc.timestamp_opt(deployment.started_at, 0).unwrap();
                    let completed_dt: DateTime<Utc> = Utc.timestamp_opt(completed_at, 0).unwrap();
                    let duration = completed_dt.signed_duration_since(started_dt);

                    println!("Completed:     {}", format_timestamp(completed_at));
                    println!("Duration:      {}s", duration.num_seconds());
                }

                if deployment.retry_count > 0 {
                    println!("Retry Count:   {}", deployment.retry_count);
                }

                if let Some(reason) = deployment.failure_reason {
                    println!("\n❌ Failure Reason:");
                    println!("   {}", reason);

                    println!("\n🔧 To fix and retry:");
                    println!("   1. Fix the issue (update .env, set secrets, or fix code)");
                    println!("   2. Run: shipwright retry");
                }

                println!("═══════════════════════════════════════════════════════");

                // Show actionable next steps
                if deployment.status == "failed" {
                    println!("\n💡 Next Steps:");
                    println!("   • Review error details above");
                    println!("   • Fix configuration or code issues");
                    println!("   • Run 'shipwright retry' to retry deployment");
                } else if deployment.status == "success" {
                    println!("\n💡 Deployment is healthy!");
                    println!("   • View logs: shipwright logs");
                    println!("   • Watch live: shipwright watch");
                }
            } else {
                println!("Status:        ❓ NO DEPLOYMENT HISTORY");
                println!("\n💡 No deployments found for this project.");
                println!("   Run 'shipwright up' to create your first deployment.");
            }
        }
        Ok(resp) => {
            let status = resp.status();
            println!("Status:        ❌ ERROR");
            println!("Error:         API returned status {}", status);

            if status == 404 {
                println!("\n💡 No deployment history found.");
                println!("   Run 'shipwright up' to create your first deployment.");
            }
        }
        Err(e) => {
            println!("Status:        ❌ DISCONNECTED");
            println!("Error:         {}", e);
            println!("\n💡 Cannot connect to agent at {}", vps.host);
            println!("   • Check if agent is running");
            println!("   • Verify network connectivity");
        }
    }

    Ok(())
}

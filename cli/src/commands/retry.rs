use anyhow::{Result, Context};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shipwright_common::config::Config;
use std::fs;

#[derive(Serialize)]
struct RetryRequest {
    project_id: String,
}

#[derive(Deserialize)]
struct RetryResponse {
    success: bool,
    message: String,
    attempt_id: Option<String>,
}

fn get_api_url(host: &str) -> String {
    format!("http://{}:17670", host)
}

pub async fn run() -> Result<()> {
    // Load config
    let config_content = fs::read_to_string(".shipwright.yml")
        .context(".shipwright.yml not found. Are you in a Shipwright project directory?")?;
    let config: Config = serde_yaml::from_str(&config_content)?;

    let vps = config.deploy.vps.as_ref()
        .context("No VPS configuration found in .shipwright.yml")?;

    println!("🔄 Retrying last failed deployment for {}...", config.project.name);

    // Call retry API
    let client = Client::new();
    let url = format!("{}/api/v1/deployments/retry", get_api_url(&vps.host));

    let request = RetryRequest {
        project_id: config.project.name.clone(),
    };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await
        .context("Failed to connect to agent. Is the agent running?")?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());

        if status == 404 {
            println!("❌ No deployment history found for this project.");
            println!("   Run 'shipwright up' to create an initial deployment.");
            return Ok(());
        } else if status == 400 {
            println!("❌ Last deployment did not fail - nothing to retry.");
            println!("   Run 'shipwright status' to see the current deployment status.");
            return Ok(());
        } else {
            anyhow::bail!("API error ({}): {}", status, error_text);
        }
    }

    let retry_response: RetryResponse = response.json().await
        .context("Failed to parse response from agent")?;

    if retry_response.success {
        println!("✅ {}", retry_response.message);
        println!("\n💡 Tip: Run 'shipwright watch' in another terminal to see live deployment progress");

        if let Some(attempt_id) = retry_response.attempt_id {
            println!("   Attempt ID: {}", attempt_id);
        }
    } else {
        println!("❌ Failed to retry deployment: {}", retry_response.message);
    }

    Ok(())
}

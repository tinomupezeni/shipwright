use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use shipwright_common::config::Config;
use dialoguer::{Input, Confirm};

#[derive(Debug, Serialize)]
struct SetSecretRequest {
    project_id: String,
    project_name: String,
    name: String,
    value: String,
    tags: Option<Vec<String>>,
    performed_by: String,
}

#[derive(Debug, Serialize)]
struct GetSecretRequest {
    project_id: String,
    project_name: String,
    performed_by: String,
}

#[derive(Debug, Serialize)]
struct ListSecretsRequest {
    project_id: String,
}

#[derive(Debug, Serialize)]
struct GetAllSecretsRequest {
    project_id: String,
    project_name: String,
    performed_by: String,
}

#[derive(Debug, Serialize)]
struct DeleteSecretRequest {
    project_id: String,
    performed_by: String,
}

#[derive(Debug, Deserialize)]
struct SecretWithValue {
    name: String,
    value: String,
    created_at: i64,
    updated_at: i64,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SecretMetadata {
    name: String,
    created_at: i64,
    updated_at: i64,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SetSecretResponse {
    success: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
struct GetSecretResponse {
    secret: SecretWithValue,
}

#[derive(Debug, Deserialize)]
struct ListSecretsResponse {
    secrets: Vec<SecretMetadata>,
}

#[derive(Debug, Deserialize)]
struct GetAllSecretsResponse {
    secrets: Vec<SecretWithValue>,
}

#[derive(Debug, Deserialize)]
struct DeleteSecretResponse {
    success: bool,
    message: String,
}

fn load_config() -> Result<Config> {
    let config_content = fs::read_to_string(".shipwright.yml")
        .context("Failed to read .shipwright.yml. Run 'shipwright init' first.")?;
    serde_yaml::from_str(&config_content).context("Failed to parse .shipwright.yml")
}

fn get_api_url(host: &str) -> String {
    format!("http://{}:17670", host)
}

pub async fn run_set(name: Option<String>, value: Option<String>, tags: Vec<String>) -> Result<()> {
    let config = load_config()?;
    let vps = config.deploy.vps.context("VPS configuration not found in .shipwright.yml")?;

    // Get secret name
    let secret_name = match name {
        Some(n) => n,
        None => Input::<String>::new()
            .with_prompt("Secret name")
            .interact_text()?,
    };

    // Get secret value
    let secret_value = match value {
        Some(v) => v,
        None => dialoguer::Password::new()
            .with_prompt("Secret value")
            .interact()?,
    };

    let client = Client::new();
    let url = format!("{}/api/v1/secrets", get_api_url(&vps.host));

    let request = SetSecretRequest {
        project_id: config.project.name.clone(),
        project_name: config.project.name.clone(),
        name: secret_name.clone(),
        value: secret_value,
        tags: if tags.is_empty() { None } else { Some(tags) },
        performed_by: "cli".to_string(),
    };

    let response = client.post(&url)
        .json(&request)
        .send()
        .await
        .context("Failed to connect to agent")?;

    if response.status().is_success() {
        let result: SetSecretResponse = response.json().await?;
        println!("✅ {}", result.message);
    } else {
        let error_text = response.text().await?;
        anyhow::bail!("Failed to set secret: {}", error_text);
    }

    Ok(())
}

pub async fn run_get(name: String, show_value: bool) -> Result<()> {
    let config = load_config()?;
    let vps = config.deploy.vps.context("VPS configuration not found in .shipwright.yml")?;

    let client = Client::new();
    let url = format!("{}/api/v1/secrets/{}", get_api_url(&vps.host), name);

    let request = GetSecretRequest {
        project_id: config.project.name.clone(),
        project_name: config.project.name.clone(),
        performed_by: "cli".to_string(),
    };

    let response = client.post(&url)
        .json(&request)
        .send()
        .await
        .context("Failed to connect to agent")?;

    if response.status().is_success() {
        let result: GetSecretResponse = response.json().await?;
        let secret = result.secret;

        println!("\n🔐 Secret: {}", secret.name);
        if show_value {
            println!("   Value: {}", secret.value);
        } else {
            println!("   Value: ********** (use --show to reveal)");
        }
        println!("   Created: {}", chrono::DateTime::from_timestamp(secret.created_at, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown".to_string()));
        println!("   Updated: {}", chrono::DateTime::from_timestamp(secret.updated_at, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown".to_string()));
        if let Some(tags) = secret.tags {
            println!("   Tags: {}", tags.join(", "));
        }
    } else {
        let error_text = response.text().await?;
        anyhow::bail!("Failed to get secret: {}", error_text);
    }

    Ok(())
}

pub async fn run_list(with_values: bool) -> Result<()> {
    let config = load_config()?;
    let vps = config.deploy.vps.context("VPS configuration not found in .shipwright.yml")?;

    let client = Client::new();

    if with_values {
        // Get all secrets with values
        let url = format!("{}/api/v1/secrets/all", get_api_url(&vps.host));

        let request = GetAllSecretsRequest {
            project_id: config.project.name.clone(),
            project_name: config.project.name.clone(),
            performed_by: "cli".to_string(),
        };

        let response = client.post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to connect to agent")?;

        if response.status().is_success() {
            let result: GetAllSecretsResponse = response.json().await?;

            if result.secrets.is_empty() {
                println!("📭 No secrets configured for this project");
            } else {
                println!("\n🔐 Secrets for project '{}':\n", config.project.name);
                for secret in result.secrets {
                    println!("   {}={}", secret.name, secret.value);
                }
                println!();
            }
        } else {
            let error_text = response.text().await?;
            anyhow::bail!("Failed to list secrets: {}", error_text);
        }
    } else {
        // List metadata only
        let url = format!("{}/api/v1/secrets/list", get_api_url(&vps.host));

        let request = ListSecretsRequest {
            project_id: config.project.name.clone(),
        };

        let response = client.post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to connect to agent")?;

        if response.status().is_success() {
            let result: ListSecretsResponse = response.json().await?;

            if result.secrets.is_empty() {
                println!("📭 No secrets configured for this project");
            } else {
                println!("\n🔐 Secrets for project '{}':\n", config.project.name);
                println!("   {:<30} {:<20} {:<20}", "NAME", "CREATED", "UPDATED");
                println!("   {}", "-".repeat(70));

                for secret in result.secrets {
                    let created = chrono::DateTime::from_timestamp(secret.created_at, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "Unknown".to_string());
                    let updated = chrono::DateTime::from_timestamp(secret.updated_at, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "Unknown".to_string());

                    println!("   {:<30} {:<20} {:<20}", secret.name, created, updated);

                    if let Some(tags) = secret.tags {
                        if !tags.is_empty() {
                            println!("   └─ Tags: {}", tags.join(", "));
                        }
                    }
                }
                println!();
            }
        } else {
            let error_text = response.text().await?;
            anyhow::bail!("Failed to list secrets: {}", error_text);
        }
    }

    Ok(())
}

pub async fn run_delete(name: String, force: bool) -> Result<()> {
    let config = load_config()?;
    let vps = config.deploy.vps.context("VPS configuration not found in .shipwright.yml")?;

    // Confirm deletion unless --force is used
    if !force {
        let confirmed = Confirm::new()
            .with_prompt(format!("Are you sure you want to delete secret '{}'?", name))
            .default(false)
            .interact()?;

        if !confirmed {
            println!("❌ Deletion cancelled");
            return Ok(());
        }
    }

    let client = Client::new();
    let url = format!("{}/api/v1/secrets/{}/delete", get_api_url(&vps.host), name);

    let request = DeleteSecretRequest {
        project_id: config.project.name.clone(),
        performed_by: "cli".to_string(),
    };

    let response = client.post(&url)
        .json(&request)
        .send()
        .await
        .context("Failed to connect to agent")?;

    if response.status().is_success() {
        let result: DeleteSecretResponse = response.json().await?;
        println!("✅ {}", result.message);
    } else {
        let error_text = response.text().await?;
        anyhow::bail!("Failed to delete secret: {}", error_text);
    }

    Ok(())
}

pub async fn run_export() -> Result<()> {
    let config = load_config()?;
    let vps = config.deploy.vps.context("VPS configuration not found in .shipwright.yml")?;

    let client = Client::new();
    let url = format!("{}/api/v1/secrets/all", get_api_url(&vps.host));

    let request = GetAllSecretsRequest {
        project_id: config.project.name.clone(),
        project_name: config.project.name.clone(),
        performed_by: "cli".to_string(),
    };

    let response = client.post(&url)
        .json(&request)
        .send()
        .await
        .context("Failed to connect to agent")?;

    if response.status().is_success() {
        let result: GetAllSecretsResponse = response.json().await?;

        if result.secrets.is_empty() {
            println!("# No secrets configured");
        } else {
            println!("# Secrets for project '{}'", config.project.name);
            println!("# Exported at: {}", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"));
            println!();
            for secret in result.secrets {
                println!("{}={}", secret.name, secret.value);
            }
        }
    } else {
        let error_text = response.text().await?;
        anyhow::bail!("Failed to export secrets: {}", error_text);
    }

    Ok(())
}

use anyhow::{Result, Context};
use std::process::Command;
use shipwright_common::config::Config;
use std::fs;
use dialoguer::{Input, theme::ColorfulTheme};
use octocrab::Octocrab;
use serde_json::json;

pub async fn run() -> Result<()> {
    // 1. Read config
    let config_path = std::path::Path::new(".shipwright.yml");
    if !config_path.exists() {
        anyhow::bail!(".shipwright.yml not found. Run 'shipwright init' first.");
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;
    
    let vps = config.deploy.vps.as_ref().context("No VPS configured in .shipwright.yml")?;

    // 2. Get GitHub info
    let git_remote = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()?;
    let repo_url = String::from_utf8(git_remote.stdout)?.trim().to_string();
    
    // Parse owner/repo from URL
    // e.g., https://github.com/owner/repo.git or git@github.com:owner/repo.git
    let re = regex::Regex::new(r"github\.com[:/]([^/]+)/([^/.]+)(\.git)?$")?;
    let caps = re.captures(&repo_url).context("Failed to parse GitHub repository from remote URL. Ensure your origin is a GitHub URL.")?;
    let owner = &caps[1];
    let repo = &caps[2];

    println!("Registering {}/{} with Shipwright Agent on {}...", owner, repo, vps.host);

    // 3. Register with Agent
    let client = reqwest::Client::new();
    let webhook_secret = uuid::Uuid::new_v4().to_string();
    
    // Use the registration endpoint on the agent
    let agent_url = format!("http://{}:8083/projects", vps.host);
    let res = client.post(&agent_url)
        .json(&json!({
            "name": repo,
            "repo_url": repo_url,
            "webhook_secret": webhook_secret
        }))
        .send()
        .await?;

    if !res.status().is_success() {
        anyhow::bail!("Failed to register project with agent: {}", res.text().await?);
    }

    println!("✅ Project registered with Shipwright Agent.");

    // 4. Setup GitHub Webhook
    let github_token: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter your GitHub Personal Access Token (PAT) with 'repo' scope")
        .interact_text()?;

    let octocrab = Octocrab::builder()
        .personal_token(github_token)
        .build()?;

    let webhook_url = format!("http://{}:8083/webhooks/github", vps.host);
    
    let route = format!("/repos/{owner}/{repo}/hooks");
    let payload = json!({
        "name": "web",
        "active": true,
        "events": ["push"],
        "config": {
            "url": webhook_url,
            "content_type": "json",
            "secret": webhook_secret,
            "insecure_ssl": "1"
        }
    });

    match octocrab.post::<_, serde_json::Value>(route, Some(&payload)).await {
        Ok(_) => println!("✅ GitHub Webhook created successfully!"),
        Err(e) => {
            if e.to_string().contains("already exists") {
                println!("ℹ️  GitHub Webhook already exists. Skipping creation.");
            } else {
                return Err(e.into());
            }
        }
    }

    println!("\n🚀 Done! Shipwright will now automatically build and deploy every time you push to GitHub.");
    println!("💡 You can view logs by running 'shipwright logs' or the upcoming 'shipwright watch' command.");

    Ok(())
}

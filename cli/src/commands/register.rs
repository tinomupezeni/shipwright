use anyhow::{Result, Context};
use std::process::Command;
use shipwright_common::config::Config;
use std::fs;
use dialoguer::{Input, theme::ColorfulTheme};
use octocrab::Octocrab;
use serde_json::json;

use crate::docker::deploy::execute_remote_command;

async fn discover_agent_ports(vps: &shipwright_common::config::VpsConfig) -> Result<(u16, u16)> {
    let output = execute_remote_command(vps, "cat /etc/shipwright/agent.env || cat agent.env")?;
    
    let mut ws_port = 8081;
    let mut http_port = 8083;

    for line in output.lines() {
        if line.starts_with("SHIPWRIGHT_WS_PORT=") {
            ws_port = line.replace("SHIPWRIGHT_WS_PORT=", "").parse()?;
        } else if line.starts_with("SHIPWRIGHT_HTTP_PORT=") {
            http_port = line.replace("SHIPWRIGHT_HTTP_PORT=", "").parse()?;
        }
    }

    Ok((ws_port, http_port))
}

pub async fn run() -> Result<()> {
    // 1. Read config
    let config_path = std::path::Path::new(".shipwright.yml");
    if !config_path.exists() {
        anyhow::bail!(".shipwright.yml not found. Run 'shipwright init' first.");
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;
    
    let vps = config.deploy.vps.as_ref().context("No VPS configured in .shipwright.yml")?;

    // 2. Discover Agent Ports
    println!("🔍 Discovering Agent ports on {}...", vps.host);
    let (_ws_port, http_port) = discover_agent_ports(vps).await?;
    println!("📡 Agent found listening on port {}", http_port);

    // 3. Get GitHub info
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

    println!("Registering {} ({}/{}) with Shipwright Agent on {}...", repo, owner, repo, vps.host);

    // 3. Register with Agent
    let client = reqwest::Client::new();
    let webhook_secret = uuid::Uuid::new_v4().to_string();
    
    // Use the registration endpoint on the discovered port
    let agent_url = format!("http://{}:{}/projects", vps.host, http_port);
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
    let token_path = shellexpand::tilde("~/.shipwright/github_token").to_string();
    let mut github_token = fs::read_to_string(&token_path).ok().map(|t| t.trim().to_string());

    if github_token.is_none() {
        println!("\n🔑 GitHub Authentication Required");
        println!("To automate your deployments, Shipwright needs a Personal Access Token (PAT) to create webhooks.");
        println!("\nHow to get one:");
        println!("  1. Go to: https://github.com/settings/tokens");
        println!("  2. Click 'Generate new token' (Classic is easiest).");
        println!("  3. Select the 'repo' scope.");
        println!("  4. Copy the token and paste it here.\n");

        let input: String = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter your GitHub PAT")
            .interact_text()?;
        
        let _ = fs::create_dir_all(shellexpand::tilde("~/.ssh").to_string()); // Ensure base exists
        let ship_dir = shellexpand::tilde("~/.shipwright").to_string();
        let _ = fs::create_dir_all(&ship_dir);
        fs::write(&token_path, &input)?;
        println!("💾 Token saved securely for future use.");
        github_token = Some(input);
    }

    let octocrab = Octocrab::builder()
        .personal_token(github_token.unwrap())
        .build()?;

    let webhook_url = format!("http://{}:{}/webhooks/github", vps.host, http_port);
    
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

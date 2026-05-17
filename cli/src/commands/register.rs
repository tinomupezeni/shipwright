use anyhow::{Result, Context};
use std::process::Command;
use shipwright_common::config::Config;
use std::fs;
use dialoguer::{Input, theme::ColorfulTheme};
use octocrab::Octocrab;
use serde_json::json;

// Fixed ports for Shipwright Agent
// Must match agent/src/main.rs constants
const SHIPWRIGHT_WS_PORT: u16 = 17671;
const SHIPWRIGHT_HTTP_PORT: u16 = 17670;

pub async fn run() -> Result<()> {
    // 1. Read config
    let config_path = std::path::Path::new(".shipwright.yml");
    if !config_path.exists() {
        anyhow::bail!(".shipwright.yml not found. Run 'shipwright init' first.");
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;
    
    let vps = config.deploy.vps.as_ref().context("No VPS configured in .shipwright.yml")?;

    // 2. Use fixed Agent ports
    println!("📡 Connecting to Shipwright Agent on {}:{}...", vps.host, SHIPWRIGHT_HTTP_PORT);
    let http_port = SHIPWRIGHT_HTTP_PORT;

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

    // Get current branch to use as deploy branch
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()?;
    let current_branch = String::from_utf8(branch_output.stdout)?.trim().to_string();
    let deploy_branch = if current_branch.is_empty() { "main".to_string() } else { current_branch };

    println!("📌 Deploy branch: {}", deploy_branch);

    // 3. Register with Agent
    let client = reqwest::Client::new();
    let webhook_secret = uuid::Uuid::new_v4().to_string();

    // Use the registration endpoint on the discovered port
    let agent_url = format!("http://{}:{}/projects", vps.host, http_port);
    let res = client.post(&agent_url)
        .json(&json!({
            "name": repo,
            "repo_url": repo_url,
            "webhook_secret": webhook_secret,
            "deploy_branch": deploy_branch
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

    println!("\n🚀 Done! Shipwright will automatically deploy when you push to the '{}' branch.", deploy_branch);
    println!("💡 View deployment logs with 'shipwright watch'");
    println!("🔒 Webhook secured with signature verification");

    Ok(())
}

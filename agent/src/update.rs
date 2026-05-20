use anyhow::{Result, Context};
use shipwright_common::version::VERSION;
use serde::Deserialize;
use std::fs;
use std::env;
use tracing::{info, warn};
use tokio::time::{sleep, Duration};

#[derive(Deserialize, Debug)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize, Debug)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub async fn start_update_checker() {
    info!("🚀 Starting background update checker for agent...");
    
    loop {
        if let Err(e) = check_and_notify_update().await {
            warn!("⚠️  Failed to check for agent updates: {}", e);
        }
        
        // Check once every 12 hours
        sleep(Duration::from_secs(12 * 3600)).await;
    }
}

async fn check_and_notify_update() -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("shipwright-agent")
        .timeout(Duration::from_secs(5))
        .build()?;
        
    let response = client
        .get("https://api.github.com/repos/tinomupezeni/shipwright/releases/latest")
        .send()
        .await?;
        
    if !response.status().is_success() {
        return Ok(());
    }

    let release: GithubRelease = response.json().await?;
    let latest_version = release.tag_name.trim_start_matches('v');
    
    if latest_version != VERSION {
        info!("✨ A new version of Shipwright Agent is available: v{} (current: v{})", latest_version, VERSION);
        info!("👉 Run 'shipwright update-agent' or 'shipwright update --agent' to upgrade.");
    }
    
    Ok(())
}

/// Perform an in-place self-update of the agent binary
pub async fn perform_self_update() -> Result<()> {
    info!("🔄 Initiating agent self-update...");
    
    let client = reqwest::Client::builder()
        .user_agent("shipwright-agent")
        .build()?;
        
    let release: GithubRelease = client
        .get("https://api.github.com/repos/tinomupezeni/shipwright/releases/latest")
        .send()
        .await?
        .json()
        .await?;
        
    let target_os = env::consts::OS;
    let target_arch = env::consts::ARCH;
    
    let asset_name = match (target_os, target_arch) {
        ("linux", "x86_64") => "shipwright-agent-linux-x86_64",
        ("macos", "x86_64") => "shipwright-agent-macos-x86_64",
        ("macos", "aarch64") => "shipwright-agent-macos-aarch64",
        ("windows", "x86_64") => "shipwright-agent-windows-x86_64.exe",
        _ => anyhow::bail!("Unsupported platform: {}-{}", target_os, target_arch),
    };
    
    let asset = release.assets.iter()
        .find(|a| a.name == asset_name)
        .context(format!("Could not find asset '{}' in the latest release", asset_name))?;
        
    info!("Downloading new agent binary from {}...", asset.browser_download_url);
    
    let response = client.get(&asset.browser_download_url).send().await?;
    let content = response.bytes().await?;
    
    let current_exe = env::current_exe()?;
    let mut temp_exe = current_exe.clone();
    temp_exe.set_extension("tmp");
    
    fs::write(&temp_exe, content)?;
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_exe)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&temp_exe, perms)?;
    }
    
    let mut old_exe = current_exe.clone();
    old_exe.set_extension("old");
    
    info!("Swapping agent binaries...");
    fs::rename(&current_exe, &old_exe)?;
    if let Err(e) = fs::rename(&temp_exe, &current_exe) {
        fs::rename(&old_exe, &current_exe)?;
        return Err(e).context("Failed to replace agent binary");
    }
    
    info!("✅ Agent binary updated to v{}. Restarting service...", release.tag_name);
    
    // Clean up old binary
    let _ = fs::remove_file(old_exe);
    
    // Self-restart: The systemd service has Restart=always, so exiting will trigger a restart with the new binary.
    std::process::exit(0);
}

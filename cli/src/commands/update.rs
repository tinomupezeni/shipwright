use anyhow::{Result, Context};
use shipwright_common::version::VERSION;
use serde::Deserialize;
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::env;
use std::io::Write;
use std::path::PathBuf;

#[derive(Deserialize, Debug)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
    body: String,
}

#[derive(Deserialize, Debug)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub async fn run() -> Result<()> {
    println!("Checking for updates...");
    
    let client = reqwest::Client::builder()
        .user_agent("shipwright-cli")
        .build()?;
        
    let release: GithubRelease = client
        .get("https://api.github.com/repos/tinomupezeni/shipwright/releases/latest")
        .send()
        .await?
        .json()
        .await?;
        
    let latest_version = release.tag_name.trim_start_matches('v');
    
    if latest_version == VERSION {
        println!("✅ Shipwright is already up to date (v{})", VERSION);
        return Ok(());
    }
    
    println!("🚀 New version available: v{} (current: v{})", latest_version, VERSION);
    println!("\nChangelog:\n{}", release.body);
    
    if !Confirm::new()
        .with_prompt("Do you want to update now?")
        .default(true)
        .interact()? 
    {
        return Ok(());
    }
    
    // Determine the asset to download based on OS and Arch
    let target_os = env::consts::OS;
    let target_arch = env::consts::ARCH;
    
    let asset_name = match (target_os, target_arch) {
        ("linux", "x86_64") => "shipwright-linux-x86_64",
        ("macos", "x86_64") => "shipwright-macos-x86_64",
        ("macos", "aarch64") => "shipwright-macos-aarch64",
        ("windows", "x86_64") => "shipwright-windows-x86_64.exe",
        _ => anyhow::bail!("Unsupported platform: {}-{}", target_os, target_arch),
    };
    
    let asset = release.assets.iter()
        .find(|a| a.name == asset_name)
        .context(format!("Could not find asset '{}' in the latest release", asset_name))?;
        
    println!("Downloading {}...", asset_name);
    
    let response = client.get(&asset.browser_download_url).send().await?;
    let total_size = response.content_length().unwrap_or(0);
    
    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")?
        .progress_chars("#>-"));
        
    let mut downloaded = 0;
    let mut stream = response.bytes_stream();
    let mut content = Vec::new();
    
    use futures_util::StreamExt;
    while let Some(item) = stream.next().await {
        let chunk = item?;
        content.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }
    
    pb.finish_with_message("Download complete");
    
    // Replace the current binary
    let current_exe = env::current_exe()?;
    let mut temp_exe = current_exe.clone();
    temp_exe.set_extension("tmp");
    
    fs::write(&temp_exe, content)?;
    
    // On Unix, we need to set execution permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_exe)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&temp_exe, perms)?;
    }
    
    // Rename current to old and temp to current (Atomic-ish)
    let mut old_exe = current_exe.clone();
    old_exe.set_extension("old");
    
    fs::rename(&current_exe, &old_exe)?;
    if let Err(e) = fs::rename(&temp_exe, &current_exe) {
        // Rollback
        fs::rename(&old_exe, &current_exe)?;
        return Err(e).context("Failed to replace binary");
    }
    
    // Clean up
    let _ = fs::remove_file(old_exe);
    
    println!("\n✅ Successfully updated to v{}!", latest_version);
    println!("Please restart Shipwright to use the new version.");
    
    Ok(())
}

pub async fn check_for_updates_silently() -> Result<()> {
    let shipwright_dir = dirs::home_dir().map(|h| h.join(".shipwright")).unwrap_or_else(|| PathBuf::from(".shipwright"));
    let last_check_file = shipwright_dir.join("last_update_check");
    
    // Only check once every 24 hours
    if let Ok(metadata) = fs::metadata(&last_check_file) {
        if let Ok(modified) = metadata.modified() {
            if let Ok(elapsed) = modified.elapsed() {
                if elapsed.as_secs() < 86400 {
                    return Ok(());
                }
            }
        }
    }
    
    // Create dir if it doesn't exist
    let _ = fs::create_dir_all(&shipwright_dir);
    // Update the timestamp file
    let _ = fs::write(&last_check_file, "");

    let client = reqwest::Client::builder()
        .user_agent("shipwright-cli")
        .timeout(std::time::Duration::from_secs(2)) // Fast timeout
        .build()?;
        
    let release_res = client
        .get("https://api.github.com/repos/tinomupezeni/shipwright/releases/latest")
        .send()
        .await;
        
    if let Ok(response) = release_res {
        if let Ok(release) = response.json::<GithubRelease>().await {
            let latest_version = release.tag_name.trim_start_matches('v');
            if latest_version != VERSION {
                println!("\n✨ A new version of Shipwright is available: v{} (current: v{})", latest_version, VERSION);
                println!("👉 Run 'shipwright update' to install it.\n");
            }
        }
    }
    
    Ok(())
}

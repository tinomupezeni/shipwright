use anyhow::{Result, Context};
use shipwright_common::config::Config;
use std::fs;
use std::path::Path;
use std::process::Command;
use dialoguer::{Confirm, theme::ColorfulTheme};
use crate::docker::deploy::execute_remote_command;

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");
    if !config_path.exists() {
        println!("❌ .shipwright.yml not found. Run 'shipwright init' first.");
        return Ok(());
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;

    let vps = config.deploy.vps.as_ref().context("No VPS configured in .shipwright.yml")?;

    println!("🚀 Updating Shipwright Agent on {}...", vps.host);

    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("This will rebuild and upload the agent binary. Continue?")
        .default(true)
        .interact()?
    {
        println!("Update cancelled.");
        return Ok(());
    }

    // 1. Build agent locally
    println!("🔨 Building Shipwright Agent binary locally...");
    let build_status = Command::new("cargo")
        .args(["build", "--release", "-p", "shipwright-agent"])
        .status()?;

    if !build_status.success() {
        anyhow::bail!("Failed to build shipwright-agent locally.");
    }

    // 2. Check if binary exists
    let binary_path = Path::new("target/release/shipwright-agent");
    if !binary_path.exists() {
        anyhow::bail!("Agent binary not found at target/release/shipwright-agent");
    }

    println!("✅ Agent binary built successfully");

    // 3. Stop the agent service
    println!("⏸️  Stopping agent service...");
    execute_remote_command(vps, "sudo systemctl stop shipwright-agent")?;

    // 4. Upload the new binary
    println!("📤 Uploading new agent binary to VPS...");
    let scp_status = Command::new("scp")
        .arg("-i")
        .arg(&vps.ssh_key)
        .arg("target/release/shipwright-agent")
        .arg(format!("{}@{}:/tmp/shipwright-agent", vps.user, vps.host))
        .status()?;

    if !scp_status.success() {
        // Try to restart the old agent if upload failed
        let _ = execute_remote_command(vps, "sudo systemctl start shipwright-agent");
        anyhow::bail!("Failed to upload agent binary. Old agent has been restarted.");
    }

    // 5. Install the new binary
    println!("⚙️  Installing new agent binary...");
    execute_remote_command(vps, "sudo mv /tmp/shipwright-agent /usr/local/bin/shipwright-agent")?;
    execute_remote_command(vps, "sudo chmod +x /usr/local/bin/shipwright-agent")?;

    // 6. Restart the agent service
    println!("🔄 Restarting agent service...");
    execute_remote_command(vps, "sudo systemctl daemon-reload")?;
    execute_remote_command(vps, "sudo systemctl start shipwright-agent")?;

    // 7. Check status
    println!("🔍 Checking agent status...");
    match execute_remote_command(vps, "sudo systemctl is-active shipwright-agent") {
        Ok(_) => {
            println!("\n✅ Shipwright Agent updated successfully!");
            println!("✨ The agent is now running with the latest changes.");
        }
        Err(_) => {
            println!("\n⚠️  Agent updated but may not be running properly.");
            println!("Check logs with: ssh {}@{} 'sudo journalctl -u shipwright-agent -f'",
                vps.user, vps.host);
        }
    }

    Ok(())
}

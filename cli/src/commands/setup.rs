use anyhow::{Result, Context};
use shipwright_common::config::Config;
use std::fs;
use std::path::Path;
use dialoguer::{Confirm, theme::ColorfulTheme};
use crate::docker::deploy::execute_remote_command;

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");
    if !config_path.exists() {
        println!(".shipwright.yml not found. Run 'shipwright init' first.");
        return Ok(());
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;

    let vps = config.deploy.vps.as_ref().context("No VPS configured in .shipwright.yml")?;

    println!("🛠️  Preparing to setup VPS at {}...", vps.host);
    
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("This will install Docker and the Shipwright Agent on your VPS. Continue?")
        .interact()? 
    {
        println!("Setup cancelled.");
        return Ok(());
    }

    // 1. Check for Docker first
    println!("🐳 Checking for Docker...");
    let docker_check = execute_remote_command(vps, "docker --version");
    if docker_check.is_err() {
        if Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Docker is missing. Install it now?")
            .default(true)
            .interact()?
        {
            println!("📦 Updating system and installing Docker...");
            execute_remote_command(vps, "sudo apt-get update -y")?;
            execute_remote_command(vps, "curl -fsSL https://get.docker.com -o get-docker.sh && sudo sh get-docker.sh")?;
            execute_remote_command(vps, "sudo usermod -aG docker $USER")?;
        }
    } else {
        println!("✅ Docker is already installed.");
    }

    // 2. Setup Firewall (UFW)
    if Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Would you like to enable a basic firewall (UFW) allowing SSH, HTTP, and HTTPS?")
        .default(true)
        .interact()?
    {
        println!("🛡️  Configuring Firewall...");
        execute_remote_command(vps, "sudo ufw allow 22/tcp")?;
        execute_remote_command(vps, "sudo ufw allow 80/tcp")?;
        execute_remote_command(vps, "sudo ufw allow 443/tcp")?;
        execute_remote_command(vps, "sudo ufw allow 8081/tcp")?; // Agent WebSocket
        execute_remote_command(vps, "sudo ufw --force enable")?;
    }

    // 3. Install Caddy
    if vps.domain.is_some() && Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("A domain is configured. Would you like to install Caddy for automatic HTTPS?")
        .default(true)
        .interact()?
    {
        println!("🛡️  Installing Caddy...");
        execute_remote_command(vps, "sudo apt-get install -y debian-keyring debian-archive-keyring apt-transport-https")?;
        execute_remote_command(vps, "curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg")?;
        execute_remote_command(vps, "curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-stable.list")?;
        execute_remote_command(vps, "sudo apt-get update")?;
        execute_remote_command(vps, "sudo apt-get install caddy -y")?;
    }

    // 4. Deploy Agent
    println!("🤖 Preparing Shipwright Agent (Mini-PaaS Daemon)...");
    
    // Build agent locally
    println!("🔨 Building Shipwright Agent binary...");
    let build_status = Command::new("cargo")
        .args(["build", "--release", "-p", "shipwright-agent"])
        .status()?;

    if !build_status.success() {
        anyhow::bail!("Failed to build shipwright-agent locally.");
    }

    // SCP the agent to the VPS
    println!("📤 Uploading agent binary to VPS...");
    let scp_status = Command::new("scp")
        .arg("-i")
        .arg(&vps.ssh_key)
        .arg("target/release/shipwright-agent")
        .arg(format!("{}@{}:/tmp/shipwright-agent", vps.user, vps.host))
        .status()?;

    if !scp_status.success() {
        anyhow::bail!("Failed to upload agent binary.");
    }

    // Move to /usr/local/bin and setup systemd
    println!("⚙️  Installing agent service on VPS...");
    execute_remote_command(vps, "sudo mv /tmp/shipwright-agent /usr/local/bin/shipwright-agent")?;
    execute_remote_command(vps, "sudo chmod +x /usr/local/bin/shipwright-agent")?;

    let service_file = r#"[Unit]
Description=Shipwright Agent Daemon
After=network.target docker.service

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/shipwright-agent
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
"#;

    execute_remote_command(vps, &format!("echo '{}' | sudo tee /etc/systemd/system/shipwright-agent.service", service_file))?;
    execute_remote_command(vps, "sudo systemctl daemon-reload")?;
    execute_remote_command(vps, "sudo systemctl enable shipwright-agent")?;
    execute_remote_command(vps, "sudo systemctl restart shipwright-agent")?;

    // Open range of ports for Agent (WS and Webhooks)
    execute_remote_command(vps, "sudo ufw allow 8081:8090/tcp")?;

    // 5. Hardware Handshake (Target-Aware Build Orchestration)
    println!("🔍 Performing Hardware Handshake...");
    execute_remote_command(vps, "sudo mkdir -p /etc/shipwright")?;
    // We get CPU flags to inject as build arguments
    let capabilities = execute_remote_command(vps, "grep '^flags' /proc/cpuinfo | head -n 1 | cut -d':' -f2")?;
    let flags = capabilities.trim().replace(" ", ",");
    execute_remote_command(vps, &format!("echo 'SHIPWRIGHT_CPU_FLAGS={}' | sudo tee /etc/shipwright/hardware.env", flags))?;
    
    println!("\n✅ Shipwright Agent is now running as a global daemon.");
    println!("✨ VPS Setup Complete! Your server is now ready for 'shipwright register'.");
    Ok(())
}

use std::process::Command;

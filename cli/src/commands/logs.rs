use anyhow::{Result, Context};
use shipwright_common::config::Config;
use std::fs;
use std::path::Path;
use std::process::Command;

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");
    if !config_path.exists() {
        println!(".shipwright.yml not found. Run 'shipwright init' first.");
        return Ok(());
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;

    if let Some(vps) = &config.deploy.vps {
        println!("Streaming logs from {}...", vps.host);
        
        let mut ssh_cmd = Command::new("ssh");
        ssh_cmd.arg("-i").arg(shellexpand::tilde(&vps.ssh_key).to_string().replace("\"", ""));
        ssh_cmd.arg("-o").arg("StrictHostKeyChecking=no");
        ssh_cmd.arg(format!("{}@{}", vps.user, vps.host));
        ssh_cmd.arg(format!("docker logs -f {}", config.project.name));

        let mut child = ssh_cmd.spawn().context("Failed to spawn ssh for logs")?;
        let _ = child.wait().context("Failed to wait for ssh logs")?;
    } else {
        println!("No VPS configured.");
    }

    Ok(())
}

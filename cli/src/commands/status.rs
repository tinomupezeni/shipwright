use anyhow::Result;
use shipwright_common::config::Config;
use std::fs;
use std::path::Path;
use crate::docker::deploy::execute_remote_command;

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");
    if !config_path.exists() {
        println!(".shipwright.yml not found. Run 'shipwright init' first.");
        return Ok(());
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;

    if let Some(vps) = &config.deploy.vps {
        println!("Checking status on {}...", vps.host);
        
        let output = execute_remote_command(vps, &format!("docker ps --filter name={} --format '{{{{.Status}}}}'", config.project.name))?;
        
        if output.trim().is_empty() {
            println!("Status: ● NOT RUNNING");
        } else {
            println!("Status: ● RUNNING ({})", output.trim());
        }
    } else {
        println!("No VPS configured.");
    }

    Ok(())
}

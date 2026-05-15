use anyhow::{Result, Context};
use std::fs;
use std::path::Path;
use std::process::Command;
use dialoguer::{Input, Select, theme::ColorfulTheme, Confirm};

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");

    if config_path.exists() {
        println!(".shipwright.yml already exists. Skipping initialization.");
        return Ok(());
    }

    println!("🚀 Welcome to Shipwright! Let's set up your project.");

    // 1. Auto-detect Project Name from Git or Folder
    let current_dir = std::env::current_dir()?;
    let folder_name = current_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("myapp");

    let git_remote = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output();

    let mut detected_name = folder_name.to_string();
    if let Ok(output) = git_remote {
        let url = String::from_utf8_lossy(&output.stdout);
        if let Some(repo_name) = url.trim().split('/').last() {
            detected_name = repo_name.replace(".git", "");
        }
    }

    let project_name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Project name")
        .default(detected_name)
        .interact_text()?;
    
    let project_name = project_name.to_lowercase().replace(" ", "-");

    // 2. Deployment Strategy (Mini-PaaS is now the default)
    let strategy_options = vec!["Mini-PaaS (Build & Deploy on VPS - Easiest)", "Traditional (Push image to Registry)"];
    let strategy_selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("How would you like to deploy?")
        .items(&strategy_options)
        .default(0)
        .interact()?;

    let (registry_url, registry_user, registry_token) = if strategy_selection == 1 {
        // Traditional Flow
        let registry_options = vec!["GitHub Container Registry (GHCR)", "Docker Hub", "Custom Registry"];
        let registry_type = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Choose your container registry")
            .items(&registry_options)
            .default(0)
            .interact()?;

        let registry_user: String = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Registry Username")
            .interact_text()?;

        let registry_url = match registry_type {
            0 => format!("ghcr.io/{}", registry_user),
            1 => format!("docker.io/{}", registry_user),
            _ => Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Registry URL")
                .interact_text()?,
        };

        let registry_token: String = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter your Registry Token/Password")
            .interact_text()?;

        (Some(registry_url), Some(registry_user), Some(registry_token))
    } else {
        // Mini-PaaS Flow: No registry needed
        (None, None, None)
    };

    // 3. VPS Configuration
    let vps_host: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("VPS IP Address")
        .interact_text()?;

    let vps_user: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("VPS SSH User")
        .default("winstontino".into())
        .interact_text()?;

    // SSH Key Detection
    let ssh_dir = shellexpand::tilde("~/.ssh").to_string();
    let mut ssh_keys = Vec::new();
    if let Ok(entries) = fs::read_dir(Path::new(&ssh_dir)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("id_") && !name.ends_with(".pub") {
                    ssh_keys.push(name.to_string());
                }
            }
        }
    }

    let vps_key: String = if !ssh_keys.is_empty() {
        let mut options = ssh_keys;
        options.push("Enter path manually...".to_string());
        
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Choose your SSH Private Key")
            .items(&options)
            .default(0)
            .interact()?;

        if selection == options.len() - 1 {
            Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Path to SSH Private Key")
                .interact_text()?
        } else {
            format!("~/.ssh/{}", options[selection])
        }
    } else {
        Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Path to SSH Private Key")
            .default("~/.ssh/id_rsa".into())
            .interact_text()?
    };

    let domain: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Domain name (optional)")
        .allow_empty(true)
        .interact_text()?;

    // 4. Persistence
    fs::create_dir_all(".shipwright")?;
    if let Some(token) = registry_token {
        fs::write(".shipwright/token", &token)?;
    }

    // Automated .gitignore update
    if Path::new(".gitignore").exists() {
        let mut gitignore = fs::read_to_string(".gitignore")?;
        if !gitignore.contains(".shipwright") {
            gitignore.push_str("\n# Shipwright credentials\n.shipwright/\n");
            fs::write(".gitignore", gitignore)?;
            println!("🔒 Added .shipwright to .gitignore");
        }
    }

    let has_dockerfile = Path::new("Dockerfile").exists();
    let build_config = if has_dockerfile {
        r#"build:
  image: auto-detected
  steps:
    - docker build -t ${PROJECT_NAME} ."#
    } else {
        r#"build:
  image: alpine:latest
  steps:
    - echo "Building...""#
    };

    let registry_section = if let (Some(url), Some(user)) = (registry_url, registry_user) {
        format!(r#"  registry:
    url: {url}
    auth:
      username: {user}
      token_file: .shipwright/token"#)
    } else {
        "  # Built locally on VPS (Mini-PaaS mode)".to_string()
    };

    let config = format!(r#"version: 1

project:
  name: {project_name}
  framework: auto

{build_config}

deploy:
  type: docker
{registry_section}
  vps:
    host: {vps_host}
    user: {vps_user}
    ssh_key: {vps_key}
    domain: {domain}
  replicas: 1
  
  health:
    http:
      path: /
      expect: 200
      timeout: 30s
"#);

    fs::write(config_path, config)?;
    println!("\n✅ Created .shipwright.yml with your settings.");
    
    println!("\n🚀 Next steps:");
    println!("  1. 🛠️  Run 'shipwright setup' to prepare your VPS (if not done).");
    println!("  2. 🔗 Run 'shipwright register' to link this project to your VPS & GitHub.");
    println!("  3. 🚢 Push your code and run 'shipwright watch' to see it go live!");

    Ok(())
}

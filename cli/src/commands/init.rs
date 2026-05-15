use anyhow::Result;
use std::fs;
use std::path::Path;
use dialoguer::{Input, Select, theme::ColorfulTheme};

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");

    if config_path.exists() {
        println!(".shipwright.yml already exists. Skipping initialization.");
        return Ok(());
    }

    println!("🚀 Welcome to Shipwright! Let's set up your project.");

    let current_dir = std::env::current_dir()?;
    let default_name = current_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("myapp");

    let project_name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Project name")
        .default(default_name.into())
        .interact_text()?;
    
    // Normalize for Docker
    let project_name = project_name.to_lowercase().replace(" ", "-");

    let registry_options = vec!["GitHub Container Registry (GHCR)", "Docker Hub", "Custom Registry"];
    let registry_type = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose your container registry")
        .items(&registry_options)
        .default(0)
        .interact()?;

    let registry_user: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Registry Username (e.g., your Docker Hub or GitHub user)")
        .interact_text()?;

    let registry_url = match registry_type {
        0 => format!("ghcr.io/{}", registry_user),
        1 => format!("docker.io/{}", registry_user),
        _ => Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Registry URL")
            .interact_text()?,
    };

    let vps_host: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("VPS IP Address or Hostname")
        .interact_text()?;

    let vps_user: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("VPS SSH User")
        .default("root".into())
        .interact_text()?;

    // SSH Key Detection
    let ssh_dir = shellexpand::tilde("~/.ssh").to_string();
    let mut ssh_keys = Vec::new();
    if let Ok(entries) = fs::read_dir(Path::new(&ssh_dir)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // Common private key patterns
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
        .with_prompt("Domain name (optional, e.g., example.com)")
        .allow_empty(true)
        .interact_text()?;

    let registry_token: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter your Registry Token/Password (for pushing images)")
        .interact_text()?;

    // Automated folder and token creation
    fs::create_dir_all(".shipwright")?;
    fs::write(".shipwright/token", &registry_token)?;

    // Automated .gitignore update
    if Path::new(".gitignore").exists() {
        let mut gitignore = fs::read_to_string(".gitignore")?;
        if !gitignore.contains(".shipwright") {
            gitignore.push_str("\n# Shipwright credentials\n.shipwright/\n");
            fs::write(".gitignore", gitignore)?;
            println!("🔒 Added .shipwright to .gitignore");
        }
    }

    let has_compose = Path::new("docker-compose.yml").exists() || Path::new("docker-compose.yaml").exists();
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

    let deploy_config = if has_compose {
        format!(r#"deploy:
  type: docker-compose
  registry:
    url: {registry_url}
    auth:
      username: {registry_user}
      token_file: .shipwright/token
  vps:
    host: {vps_host}
    user: {vps_user}
    ssh_key: {vps_key}
    domain: {domain}
  replicas: 1"#)
    } else {
        format!(r#"deploy:
  type: docker
  registry:
    url: {registry_url}
    auth:
      username: {registry_user}
      token_file: .shipwright/token
  vps:
    host: {vps_host}
    user: {vps_user}
    ssh_key: {vps_key}
    domain: {domain}
  replicas: 1"#)
    };

    let config = format!(r#"version: 1

project:
  name: {project_name}
  framework: auto

{build_config}

{deploy_config}
  
  health:
    http:
      path: /
      expect: 200
      timeout: 30s
"#);

    fs::write(config_path, config)?;
    println!("\n✅ Created .shipwright.yml with your settings.");
    
    println!("\n🚀 Next steps:");
    println!("  1. 🛠️  Run 'shipwright-cli setup' to prepare your VPS (installs Docker/Caddy).");
    println!("  2. 🚢 Run 'shipwright-cli up' to build and deploy your project!");
    println!("\n💡 Tip: Check .shipwright.yml to refine your build steps or health checks.");

    Ok(())
}

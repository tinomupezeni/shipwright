use anyhow::{Result, Context};
use bollard::Docker;
use bollard::image::{PushImageOptions, TagImageOptions};
use bollard::auth::DockerCredentials;
use dialoguer::{Input, Password, Confirm};
use futures_util::stream::StreamExt;
use regex::Regex;
use tracing::info;
use shipwright_common::config::Config;
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::io::Write;
use std::sync::Mutex;
use once_cell::sync::Lazy;

/// Global storage for sudo password during session
static SUDO_PASSWORD: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Prompt for sudo password if not already stored
fn get_sudo_password() -> Result<String> {
    let mut stored = SUDO_PASSWORD.lock().unwrap();

    if let Some(ref pw) = *stored {
        return Ok(pw.clone());
    }

    println!("\n🔐 Sudo password required for VPS operations");
    let password: String = Password::new()
        .with_prompt("Enter sudo password for VPS")
        .interact()?;

    *stored = Some(password.clone());
    Ok(password)
}

/// Execute a remote command that requires sudo
pub fn execute_sudo_command(vps: &shipwright_common::config::VpsConfig, command: &str) -> Result<String> {
    let sudo_pw = get_sudo_password()?;

    // Wrap command with sudo -S and pipe the password
    let sudo_command = format!("echo '{}' | sudo -S bash -c '{}'",
        sudo_pw.replace("'", "'\\''"),
        command.replace("'", "'\\''")
    );

    execute_remote_command(vps, &sudo_command)
}

/// Represents a custom image that may need to be built
#[derive(Debug, Clone)]
struct CustomImage {
    service_name: String,
    image_name: String,      // e.g., "hbec-student-backend"
    full_image: String,      // e.g., "tinotenda762/hbec-student-backend:latest"
    build_context: Option<String>,
    dockerfile: Option<String>,
}

pub async fn deploy_image(config: &Config) -> Result<()> {
    // For docker-compose deployments, skip the local build/push - go straight to deploy
    if config.deploy.deploy_type == "docker-compose" {
        println!("📦 Docker Compose deployment - using pre-built images from registry");

        if let Some(vps) = &config.deploy.vps {
            deploy_with_compose(vps, config).context("Failed to deploy with docker-compose")?;

            if let Some(domain) = &vps.domain {
                if !domain.is_empty() {
                    setup_caddy_proxy(vps, domain, 8080).context("Failed to setup Caddy proxy")?;
                }
            }
        } else {
            anyhow::bail!("VPS configuration required for docker-compose deployment");
        }

        return Ok(());
    }

    // Single container deployment - build and push local image
    let docker = Docker::connect_with_socket_defaults()?;

    let local_image = format!("{}:latest", config.project.name);
    
    if let Some(registry) = &config.deploy.registry {
        let remote_image = format!("{}/{}:latest", registry.url, config.project.name);

        info!("Tagging image {} as {}", local_image, remote_image);

        docker.tag_image(&local_image, Some(TagImageOptions {
            repo: remote_image.as_str(),
            tag: "latest",
        })).await.context("Failed to tag image")?;

        info!("Pushing image to {}", remote_image);

        let auth_config = if let Some(auth) = &registry.auth {
            let token = fs::read_to_string(&auth.token_file)
                .context(format!("Failed to read token from {}", auth.token_file))?
                .trim()
                .to_string();

            Some(DockerCredentials {
                username: Some(auth.username.clone()),
                password: Some(token),
                ..Default::default()
            })
        } else {
            None
        };

        let mut push_stream = docker.push_image(
            &remote_image,
            Some(PushImageOptions::<String> {
                ..Default::default()
            }),
            auth_config,
        );

        while let Some(msg) = push_stream.next().await {
            let msg = msg?;
            if let Some(status) = msg.status {
                println!("{}", status);
            }
            if let Some(error) = msg.error {
                anyhow::bail!("Push error: {}", error);
            }
        }

        info!("Successfully pushed {}", remote_image);

        if let Some(vps) = &config.deploy.vps {
            deploy_to_vps(vps, &remote_image, &config.project.name).context("Failed to deploy to VPS")?;

            if let Some(domain) = &vps.domain {
                if !domain.is_empty() {
                    setup_caddy_proxy(vps, domain, 8080).context("Failed to setup Caddy proxy")?;
                }
            }
        }
    } else {
        // Mini-PaaS mode - if no registry, we assume the agent handles the build from git
        println!("🚀 Mini-PaaS mode: Agent will handle building and deployment from GitHub pushes.");
    }

    Ok(())
}

fn deploy_with_compose(vps: &shipwright_common::config::VpsConfig, config: &Config) -> Result<()> {
    info!("Deploying with docker-compose to {}...", vps.host);

    // Determine remote directory
    let remote_dir = format!("/home/{}/{}", vps.user, config.project.name);

    // 1. Find the compose file (check multiple naming conventions)
    let compose_file = find_compose_file()?;
    println!("📄 Found compose file: {}", compose_file);

    // 2. Check and prompt for missing environment variables
    let env_file = ensure_env_file(&compose_file)?;
    println!("📄 Using env file: {}", env_file);

    // 3. Check and prompt for missing volume files
    ensure_volume_files(&compose_file)?;

    // Create remote directory structure
    println!("\n📁 Creating remote directory structure...");
    execute_remote_command(vps, &format!("mkdir -p {}/docker/keys {}/monitoring/grafana/provisioning/datasources", remote_dir, remote_dir))?;

    // 4. Upload docker-compose file
    upload_file(vps, &compose_file, &format!("{}/docker-compose.yml", remote_dir))?;

    // 5. Upload .env file
    println!("📄 Uploading .env file...");
    upload_file(vps, &env_file, &format!("{}/.env", remote_dir))?;

    // 6. Upload volume-mounted files (detected from compose file)
    upload_volume_files(vps, &compose_file, &remote_dir)?;

    // 7. Check for custom images and build/push if missing
    let env_vars = parse_env_file(&env_file);
    let compose_content = fs::read_to_string(&compose_file)?;
    let custom_images = extract_custom_images(&compose_content, &env_vars);

    if !custom_images.is_empty() {
        println!("\n🔍 Found {} custom image(s) to check:", custom_images.len());
        for img in &custom_images {
            println!("   • {} -> {}", img.service_name, img.full_image);
        }
        build_and_push_missing_images(&custom_images, &env_vars)?;
    }

    // 8. Authenticate with container registries if needed (for any remaining private registries)
    authenticate_registries(vps, &compose_file)?;

    // 9. Pull and start containers
    println!("\n🐳 Pulling images...");
    execute_remote_command(vps, &format!("cd {} && docker compose pull", remote_dir))?;

    println!("🚀 Starting containers...");
    let start_result = execute_remote_command(vps, &format!("cd {} && docker compose up -d", remote_dir));

    // 10. Verify deployment and run diagnostics if needed
    println!("\n📊 Verifying deployment...");
    std::thread::sleep(std::time::Duration::from_secs(10)); // Give containers more time to start

    // Check container health
    let all_healthy = super::diagnostics::print_health_summary(vps, &remote_dir)
        .unwrap_or(false);

    if !all_healthy || start_result.is_err() {
        println!("\n⚠️  Some containers are not healthy. Running diagnostics...");
        std::thread::sleep(std::time::Duration::from_secs(5)); // Wait for logs to populate

        let diagnostics = super::diagnostics::diagnose_failures(vps, &remote_dir)?;
        super::diagnostics::print_diagnostics(&diagnostics);

        // Ask if user wants to attempt automatic fixes
        if !diagnostics.is_empty() {
            // Check for password authentication issues specifically
            let has_password_issue = diagnostics.iter().any(|d|
                d.logs.to_lowercase().contains("password authentication failed")
            );

            if has_password_issue {
                println!("\n🔑 Database password mismatch detected!");
                println!("   Your .env file has different passwords than the existing databases.");

                let fix_passwords = dialoguer::Confirm::new()
                    .with_prompt("Would you like to automatically update database passwords to match .env?")
                    .default(true)
                    .interact()?;

                if fix_passwords {
                    let fixed = super::diagnostics::fix_database_passwords(vps, &remote_dir)?;

                    if fixed {
                        println!("\n   Waiting for services to restart...");
                        std::thread::sleep(std::time::Duration::from_secs(20));

                        let recovered = super::diagnostics::print_health_summary(vps, &remote_dir)
                            .unwrap_or(false);

                        if recovered {
                            println!("\n✅ Password fix successful! All services healthy.");
                        } else {
                            println!("\n⚠️  Some issues may persist. Waiting a bit more...");
                            std::thread::sleep(std::time::Duration::from_secs(15));
                            super::diagnostics::print_health_summary(vps, &remote_dir)?;
                        }
                    }
                }
            } else {
                let try_fixes = dialoguer::Confirm::new()
                    .with_prompt("Would you like to attempt automatic recovery (restart unhealthy containers)?")
                    .default(true)
                    .interact()?;

                if try_fixes {
                    println!("\n🔧 Attempting recovery...");
                    let fixes = super::diagnostics::attempt_common_fixes(vps, &remote_dir, &diagnostics)?;

                    if !fixes.is_empty() {
                        println!("   Applied fixes:");
                        for fix in &fixes {
                            println!("   • {}", fix);
                        }

                        // Wait and check again
                        println!("\n   Waiting for containers to stabilize...");
                        std::thread::sleep(std::time::Duration::from_secs(15));

                        let recovered = super::diagnostics::print_health_summary(vps, &remote_dir)
                            .unwrap_or(false);

                        if !recovered {
                            println!("\n⚠️  Some issues persist. Please check manually using the commands above.");
                        }
                    }
                }
            }
        }

        // Don't fail completely - containers might recover
        println!("\n📝 Note: Deployment completed with warnings. Monitor your containers.");
    } else {
        verify_containers(vps, &remote_dir)?;
        println!("\n✅ All containers started successfully!");
    }

    // 11. Setup Caddy reverse proxy for exposed services
    setup_caddy_for_project(vps, config, &compose_content)?;

    Ok(())
}

/// Setup Caddy reverse proxy for the project's services
fn setup_caddy_for_project(
    vps: &shipwright_common::config::VpsConfig,
    config: &Config,
    compose_content: &str,
) -> Result<()> {
    use super::caddy;

    // Detect services that might need reverse proxy
    let detected_services = caddy::detect_exposed_services(compose_content);

    if detected_services.is_empty() {
        return Ok(());
    }

    // Get existing service configs from VPS config
    let existing_configs = &vps.services;

    // Check if we need to prompt for domain configuration
    let unconfigured: Vec<_> = detected_services.iter()
        .filter(|s| s.is_frontend || s.name.contains("backend"))
        .filter(|s| !existing_configs.iter().any(|c| c.name == s.name))
        .collect();

    let service_configs = if !unconfigured.is_empty() {
        println!("\n🌐 Domain Configuration");
        println!("   {} service(s) can be exposed via reverse proxy", detected_services.len());

        let should_configure = dialoguer::Confirm::new()
            .with_prompt("Would you like to configure domains for your services?")
            .default(true)
            .interact()?;

        if should_configure {
            // Setup Caddy infrastructure first
            caddy::setup_caddy_infrastructure(vps)?;
            caddy::ensure_caddy_network(vps)?;

            // Prompt for domain configuration
            caddy::prompt_for_domains(&detected_services, &vps.host, existing_configs)?
        } else {
            println!("   Skipping domain configuration. You can run 'shipwright domains' later.");
            return Ok(());
        }
    } else if !existing_configs.is_empty() {
        // Use existing configuration
        existing_configs.clone()
    } else {
        return Ok(());
    };

    if service_configs.is_empty() {
        return Ok(());
    }

    // Check for domain conflicts with other projects
    let conflicts = caddy::check_domain_conflicts(vps, &service_configs, &config.project.name)?;
    if !conflicts.is_empty() {
        println!("\n⚠️  Domain conflicts detected:");
        for conflict in &conflicts {
            println!("   • {}", conflict);
        }
        anyhow::bail!("Please resolve domain conflicts before continuing");
    }

    // Generate and deploy Caddyfile
    let container_prefix = &config.project.name;
    let caddyfile = caddy::generate_caddyfile(&config.project.name, &service_configs, container_prefix);

    caddy::deploy_caddy_config(
        vps,
        &config.project.name,
        &caddyfile,
        vps.acme_email.as_deref(),
    )?;

    // Save service configs back to shipwright.yml (if we prompted for new ones)
    if !unconfigured.is_empty() {
        save_service_configs_hint(&service_configs);
    }

    Ok(())
}

/// Print hint about saving service configs
fn save_service_configs_hint(configs: &[shipwright_common::config::ServiceConfig]) {
    println!("\n💡 Tip: Add these to your .shipwright.yml to skip prompts next time:");
    println!("   deploy:");
    println!("     vps:");
    println!("       services:");
    for config in configs {
        if config.expose {
            println!("         - name: {}", config.name);
            if let Some(domain) = &config.domain {
                println!("           domain: {}", domain);
            }
            println!("           port: {}", config.port);
        }
    }
}

/// Parse docker-compose file and extract required environment variables
fn parse_required_env_vars(compose_content: &str) -> Vec<(String, String)> {
    let mut required_vars = Vec::new();

    // Match ${VAR:?error message} or ${VAR:?} patterns
    let re = Regex::new(r"\$\{([A-Z_][A-Z0-9_]*):(\?[^}]*)\}").unwrap();

    for cap in re.captures_iter(compose_content) {
        let var_name = cap[1].to_string();
        let error_hint = cap[2].trim_start_matches('?').to_string();

        // Avoid duplicates
        if !required_vars.iter().any(|(name, _)| name == &var_name) {
            required_vars.push((var_name, error_hint));
        }
    }

    required_vars
}

/// Parse existing .env file into a HashMap
fn parse_env_file(path: &str) -> HashMap<String, String> {
    let mut env_vars = HashMap::new();

    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
                env_vars.insert(key, value);
            }
        }
    }

    env_vars
}

/// Ensure .env file exists with all required variables, prompting user for missing ones
fn ensure_env_file(compose_file: &str) -> Result<String> {
    let compose_content = fs::read_to_string(compose_file)?;
    let required_vars = parse_required_env_vars(&compose_content);

    if required_vars.is_empty() {
        // No required vars, just return existing .env or create empty one
        if fs::metadata(".env").is_ok() {
            return Ok(".env".to_string());
        } else if fs::metadata(".env.production").is_ok() {
            return Ok(".env.production".to_string());
        }
        return Ok(".env".to_string());
    }

    // Determine which .env file to use
    let env_file = if fs::metadata(".env").is_ok() {
        ".env"
    } else if fs::metadata(".env.production").is_ok() {
        ".env.production"
    } else {
        ".env"
    };

    let mut existing_vars = parse_env_file(env_file);
    let mut missing_vars: Vec<(String, String)> = Vec::new();

    // Find missing variables
    for (var_name, hint) in &required_vars {
        if !existing_vars.contains_key(var_name) || existing_vars.get(var_name).map(|v| v.is_empty()).unwrap_or(true) {
            missing_vars.push((var_name.clone(), hint.clone()));
        }
    }

    if missing_vars.is_empty() {
        println!("✅ All required environment variables are set");
        return Ok(env_file.to_string());
    }

    // Prompt user for missing variables
    println!("\n⚠️  Missing {} required environment variable(s):", missing_vars.len());
    for (var, hint) in &missing_vars {
        if hint.is_empty() {
            println!("   • {}", var);
        } else {
            println!("   • {} ({})", var, hint);
        }
    }

    let should_continue = Confirm::new()
        .with_prompt("Would you like to enter the missing values now?")
        .default(true)
        .interact()?;

    if !should_continue {
        anyhow::bail!("Deployment cancelled. Please set the missing environment variables in {}", env_file);
    }

    println!("\n📝 Enter values for missing environment variables:");
    println!("   (Values will be saved to {})\n", env_file);

    for (var_name, hint) in &missing_vars {
        let prompt_text = if hint.is_empty() {
            var_name.clone()
        } else {
            format!("{} ({})", var_name, hint)
        };

        // Use password input for sensitive variables
        let is_sensitive = var_name.to_lowercase().contains("password")
            || var_name.to_lowercase().contains("secret")
            || var_name.to_lowercase().contains("key")
            || var_name.to_lowercase().contains("token");

        let value: String = if is_sensitive {
            Password::new()
                .with_prompt(&prompt_text)
                .interact()?
        } else {
            Input::new()
                .with_prompt(&prompt_text)
                .interact_text()?
        };

        existing_vars.insert(var_name.clone(), value);
    }

    // Write updated .env file
    let mut env_content = String::new();

    // Preserve existing content and comments
    if let Ok(original) = fs::read_to_string(env_file) {
        for line in original.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                env_content.push_str(line);
                env_content.push('\n');
            } else if let Some((key, _)) = trimmed.split_once('=') {
                let key = key.trim();
                if let Some(value) = existing_vars.get(key) {
                    // Check if value needs quoting
                    if value.contains(' ') || value.contains('$') || value.contains('#') {
                        env_content.push_str(&format!("{}=\"{}\"\n", key, value));
                    } else {
                        env_content.push_str(&format!("{}={}\n", key, value));
                    }
                    existing_vars.remove(key);
                } else {
                    env_content.push_str(line);
                    env_content.push('\n');
                }
            }
        }
    }

    // Add any new variables that weren't in the original file
    if !existing_vars.is_empty() {
        env_content.push_str("\n# Added by Shipwright\n");
        for (key, value) in &existing_vars {
            if value.contains(' ') || value.contains('$') || value.contains('#') {
                env_content.push_str(&format!("{}=\"{}\"\n", key, value));
            } else {
                env_content.push_str(&format!("{}={}\n", key, value));
            }
        }
    }

    fs::write(env_file, env_content)?;
    println!("\n✅ Updated {} with missing variables", env_file);

    Ok(env_file.to_string())
}

/// Ensure required volume-mounted files exist, prompting user if missing
fn ensure_volume_files(compose_file: &str) -> Result<()> {
    let content = fs::read_to_string(compose_file)?;

    // Critical files that are required for the deployment to work
    let critical_files: &[(&str, &[&str], &str)] = &[
        ("docker/keys/jwt_private.pem", &["docker/keys/jwt_private.pem"], "JWT private key for authentication"),
        ("docker/keys/jwt_public.pem", &["docker/keys/jwt_public.pem"], "JWT public key for authentication"),
        ("docker/init-db.sql", &["docker/init-db.sql"], "Database initialization script"),
    ];

    let mut missing_critical: Vec<(&str, &str)> = Vec::new();

    for (remote_name, local_paths, description) in critical_files {
        let is_referenced = local_paths.iter().any(|p| {
            content.contains(*p) || content.contains(&format!("./{}", p))
        }) || content.contains(&format!("./{}", remote_name));

        if !is_referenced {
            continue;
        }

        let exists = local_paths.iter().any(|p| fs::metadata(p).is_ok());
        if !exists {
            missing_critical.push((*remote_name, *description));
        }
    }

    if missing_critical.is_empty() {
        return Ok(());
    }

    println!("\n⚠️  Missing {} critical file(s):", missing_critical.len());
    for (file, desc) in &missing_critical {
        println!("   • {} - {}", file, desc);
    }

    // For JWT keys, offer to generate them
    let needs_jwt_keys = missing_critical.iter().any(|(f, _)| f.contains("jwt_"));

    if needs_jwt_keys {
        let generate = Confirm::new()
            .with_prompt("Would you like to generate JWT key pair?")
            .default(true)
            .interact()?;

        if generate {
            generate_jwt_keys()?;
            // Remove JWT files from missing list
            missing_critical.retain(|(f, _)| !f.contains("jwt_"));
        }
    }

    // For init-db.sql, offer to create a basic one
    let needs_init_db = missing_critical.iter().any(|(f, _)| f.contains("init-db"));

    if needs_init_db {
        let create = Confirm::new()
            .with_prompt("Would you like to create a basic init-db.sql?")
            .default(true)
            .interact()?;

        if create {
            create_basic_init_db()?;
            missing_critical.retain(|(f, _)| !f.contains("init-db"));
        }
    }

    if !missing_critical.is_empty() {
        println!("\n❌ The following files are still missing:");
        for (file, _) in &missing_critical {
            println!("   • {}", file);
        }
        anyhow::bail!("Please create the missing files and try again");
    }

    Ok(())
}

/// Detect registries used in compose file and authenticate on VPS
fn authenticate_registries(vps: &shipwright_common::config::VpsConfig, compose_file: &str) -> Result<()> {
    let content = fs::read_to_string(compose_file)?;

    // Find all image references
    let image_re = Regex::new(r#"image:\s*["']?([^"'\s\n]+)["']?"#).unwrap();
    let mut registries: HashMap<String, bool> = HashMap::new();

    for cap in image_re.captures_iter(&content) {
        let image = &cap[1];

        // Detect registry from image name
        if image.starts_with("ghcr.io/") {
            registries.insert("ghcr.io".to_string(), true);
        } else if image.contains(".azurecr.io") {
            let parts: Vec<&str> = image.split('/').collect();
            if !parts.is_empty() {
                registries.insert(parts[0].to_string(), true);
            }
        } else if image.contains(".dkr.ecr.") {
            let parts: Vec<&str> = image.split('/').collect();
            if !parts.is_empty() {
                registries.insert(parts[0].to_string(), true);
            }
        }
        // Docker Hub public images don't need explicit registry
    }

    if registries.is_empty() {
        return Ok(());
    }

    println!("\n🔐 Registry authentication required:");
    for registry in registries.keys() {
        println!("   • {}", registry);
    }

    // Check if already logged in on VPS
    for registry in registries.keys() {
        let check_result = execute_remote_command(
            vps,
            &format!("cat ~/.docker/config.json 2>/dev/null | grep -q '{}' && echo 'logged_in' || echo 'not_logged_in'", registry)
        );

        let is_logged_in = check_result.map(|s| s.trim() == "logged_in").unwrap_or(false);

        if is_logged_in {
            println!("   ✓ Already authenticated with {}", registry);
            continue;
        }

        // Prompt for credentials
        println!("\n📝 Enter credentials for {}:", registry);

        let username: String = if registry == "ghcr.io" {
            Input::new()
                .with_prompt("GitHub username")
                .interact_text()?
        } else {
            Input::new()
                .with_prompt("Username")
                .interact_text()?
        };

        let token: String = if registry == "ghcr.io" {
            println!("   (Use a GitHub Personal Access Token with 'read:packages' scope)");
            Password::new()
                .with_prompt("GitHub PAT")
                .interact()?
        } else {
            Password::new()
                .with_prompt("Password/Token")
                .interact()?
        };

        // Login on VPS
        println!("   Authenticating with {}...", registry);
        let login_cmd = format!(
            "echo '{}' | docker login {} -u {} --password-stdin",
            token, registry, username
        );

        match execute_remote_command(vps, &login_cmd) {
            Ok(_) => println!("   ✓ Logged in to {}", registry),
            Err(e) => {
                println!("   ✗ Failed to login to {}: {}", registry, e);
                anyhow::bail!("Registry authentication failed");
            }
        }
    }

    Ok(())
}

fn generate_jwt_keys() -> Result<()> {
    println!("🔐 Generating JWT key pair...");

    fs::create_dir_all("docker/keys")?;

    // Try using openssl if available
    let private_key_result = Command::new("openssl")
        .args(["genrsa", "-out", "docker/keys/jwt_private.pem", "2048"])
        .output();

    match private_key_result {
        Ok(output) if output.status.success() => {
            // Generate public key from private
            Command::new("openssl")
                .args(["rsa", "-in", "docker/keys/jwt_private.pem", "-pubout", "-out", "docker/keys/jwt_public.pem"])
                .output()?;
            println!("✅ Generated JWT keys in docker/keys/");
            Ok(())
        }
        _ => {
            println!("⚠️  OpenSSL not found. Please generate JWT keys manually:");
            println!("   openssl genrsa -out docker/keys/jwt_private.pem 2048");
            println!("   openssl rsa -in docker/keys/jwt_private.pem -pubout -out docker/keys/jwt_public.pem");
            anyhow::bail!("Could not generate JWT keys automatically");
        }
    }
}

fn create_basic_init_db() -> Result<()> {
    println!("📄 Creating basic init-db.sql...");

    fs::create_dir_all("docker")?;

    let init_sql = r#"-- Database initialization script
-- Created by Shipwright

-- Create databases if they don't exist
SELECT 'CREATE DATABASE hbec_student' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'hbec_student')\gexec
SELECT 'CREATE DATABASE hbec_admin' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'hbec_admin')\gexec

-- Grant privileges
GRANT ALL PRIVILEGES ON DATABASE hbec_student TO hbec;
GRANT ALL PRIVILEGES ON DATABASE hbec_admin TO hbec;
"#;

    fs::write("docker/init-db.sql", init_sql)?;
    println!("✅ Created docker/init-db.sql");
    Ok(())
}

fn find_compose_file() -> Result<String> {
    let candidates = [
        "docker-compose.production.yml",
        "docker-compose.prod.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ];

    for candidate in candidates {
        if fs::metadata(candidate).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    anyhow::bail!("No docker-compose file found. Expected one of: {:?}", candidates)
}

fn upload_file(vps: &shipwright_common::config::VpsConfig, local_path: &str, remote_path: &str) -> Result<()> {
    let remote_dest = format!("{}@{}:{}", vps.user, vps.host, remote_path);

    let mut scp_cmd = Command::new("scp");
    scp_cmd.arg("-i").arg(shellexpand::tilde(&vps.ssh_key).to_string().replace("\"", ""));
    scp_cmd.arg("-o").arg("StrictHostKeyChecking=no");
    scp_cmd.arg(local_path);
    scp_cmd.arg(&remote_dest);

    let status = scp_cmd.status().context(format!("Failed to upload {}", local_path))?;
    if !status.success() {
        anyhow::bail!("Failed to upload {}", local_path);
    }
    println!("  ✓ Uploaded {}", local_path);
    Ok(())
}

fn upload_volume_files(vps: &shipwright_common::config::VpsConfig, compose_file: &str, remote_dir: &str) -> Result<()> {
    let content = fs::read_to_string(compose_file)?;

    // Files to upload with fallback locations: (remote_name, [local_paths_to_try])
    let volume_files: &[(&str, &[&str])] = &[
        ("docker/init-db.sql", &["docker/init-db.sql"]),
        ("docker/keys/jwt_private.pem", &["docker/keys/jwt_private.pem"]),
        ("docker/keys/jwt_public.pem", &["docker/keys/jwt_public.pem"]),
        ("litellm_config.yaml", &["litellm_config.yaml", "AGENTIC_HARNESS/litellm_config.yaml"]),
        ("monitoring/prometheus.yml", &["monitoring/prometheus.yml"]),
        ("monitoring/alerts.yml", &["monitoring/alerts.yml"]),
        ("monitoring/grafana/provisioning/datasources/datasources.yml", &["monitoring/grafana/provisioning/datasources/datasources.yml"]),
    ];

    println!("📂 Uploading volume-mounted files...");

    for (remote_name, local_paths) in volume_files {
        // Check if this file is referenced in the compose file
        let is_referenced = local_paths.iter().any(|p| {
            content.contains(*p) || content.contains(&format!("./{}", p))
        }) || content.contains(&format!("./{}", remote_name));

        if !is_referenced {
            continue;
        }

        // Try each local path until we find one that exists
        let mut found = false;
        for local_path in *local_paths {
            if fs::metadata(local_path).is_ok() {
                let remote_path = format!("{}/{}", remote_dir, remote_name);

                // Create parent directory on remote
                if let Some(parent) = std::path::Path::new(remote_name).parent() {
                    if !parent.as_os_str().is_empty() {
                        execute_remote_command(vps, &format!("mkdir -p {}/{}", remote_dir, parent.display()))?;
                    }
                }

                upload_file(vps, local_path, &remote_path)?;
                found = true;
                break;
            }
        }

        if !found && is_referenced {
            println!("  ⚠ Warning: {} referenced in compose but not found locally", remote_name);
        }
    }

    Ok(())
}

fn verify_containers(vps: &shipwright_common::config::VpsConfig, remote_dir: &str) -> Result<()> {
    let output = execute_remote_command(vps, &format!(
        "cd {} && docker compose ps --format 'table {{{{.Name}}}}\\t{{{{.Status}}}}\\t{{{{.Health}}}}'",
        remote_dir
    ))?;

    println!("{}", output);

    // Check for any containers that are not running
    let output_lower = output.to_lowercase();
    if output_lower.contains("exit") || output_lower.contains("restarting") {
        println!("\n⚠️  Warning: Some containers may not be healthy");
        println!("   Run 'shipwright logs' to investigate");
    } else if output_lower.contains("up") || output_lower.contains("running") {
        println!("\n✅ Deployment successful! Containers are running.");
    }

    Ok(())
}

fn setup_caddy_proxy(vps: &shipwright_common::config::VpsConfig, domain: &str, port: u16) -> Result<()> {
    use super::caddy;

    info!("Configuring Caddy proxy for {} -> :{}...", domain, port);

    // Setup Caddy infrastructure (creates /etc/caddy/sites/ structure)
    caddy::setup_caddy_infrastructure(vps)?;

    // Create a simple service config for legacy single-domain setup
    let service_config = vec![shipwright_common::config::ServiceConfig {
        name: "app".to_string(),
        domain: Some(domain.to_string()),
        port,
        path: None,
        expose: true,
    }];

    let caddyfile = caddy::generate_caddyfile("default", &service_config, "");
    caddy::deploy_caddy_config(vps, "default", &caddyfile, None)?;

    Ok(())
}

fn deploy_to_vps(vps: &shipwright_common::config::VpsConfig, image: &str, container_name: &str) -> Result<()> {
    info!("Deploying {} to VPS at {}...", container_name, vps.host);
    
    execute_remote_command(vps, &format!("docker pull {}", image))?;
    execute_remote_command(vps, &format!("docker stop {} || true", container_name))?;
    execute_remote_command(vps, &format!("docker rm {} || true", container_name))?;
    execute_remote_command(vps, &format!("docker run -d --name {} {} ", container_name, image))?;

    info!("Successfully deployed to VPS!");
    Ok(())
}

/// Extract custom images from compose file that use DOCKER_REGISTRY variable
fn extract_custom_images(compose_content: &str, env_vars: &HashMap<String, String>) -> Vec<CustomImage> {
    let mut images = Vec::new();

    // Get the registry from env vars, default to empty (Docker Hub)
    let registry = env_vars.get("DOCKER_REGISTRY").cloned().unwrap_or_default();
    let tag = env_vars.get("TAG").cloned().unwrap_or_else(|| "latest".to_string());

    // Parse YAML to find services with DOCKER_REGISTRY in image
    let mut current_service = String::new();
    let mut in_services = false;

    for line in compose_content.lines() {
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();

        // Track when we enter services section
        if trimmed == "services:" {
            in_services = true;
            continue;
        }

        if !in_services {
            continue;
        }

        // Detect service name (line ending with : at specific indent level)
        if trimmed.ends_with(':') && !trimmed.contains(' ') && indent == 2 {
            current_service = trimmed.trim_end_matches(':').to_string();
            continue;
        }

        // Look for image line with DOCKER_REGISTRY
        if trimmed.starts_with("image:") && trimmed.contains("DOCKER_REGISTRY") {
            // Extract the image name pattern
            // e.g., image: ${DOCKER_REGISTRY:-ghcr.io/rest-creator}/hbec-student-backend:${TAG:-latest}
            if let Some(image_part) = trimmed.strip_prefix("image:") {
                let image_part = image_part.trim();

                // Extract the image name after the registry part
                // Pattern: ${DOCKER_REGISTRY:-...}/IMAGE_NAME:${TAG:-...}
                if let Some(slash_pos) = image_part.find("}/") {
                    let after_registry = &image_part[slash_pos + 2..];
                    // Extract image name (before :${TAG or :latest)
                    let image_name = if let Some(colon_pos) = after_registry.find(':') {
                        after_registry[..colon_pos].to_string()
                    } else {
                        after_registry.to_string()
                    };

                    // Build the full image name
                    let full_image = if registry.is_empty() {
                        format!("{}:{}", image_name, tag)
                    } else {
                        format!("{}/{}:{}", registry, image_name, tag)
                    };

                    images.push(CustomImage {
                        service_name: current_service.clone(),
                        image_name: image_name.clone(),
                        full_image,
                        build_context: None,
                        dockerfile: None,
                    });
                }
            }
        }
    }

    // Now try to find build contexts from the dev compose file
    if let Ok(dev_compose) = fs::read_to_string("docker-compose.yml") {
        for image in &mut images {
            if let Some((ctx, df)) = find_build_context_for_service(&dev_compose, &image.service_name) {
                image.build_context = Some(ctx);
                image.dockerfile = df;
            }
        }
    }

    // Deduplicate by image_name - keep first occurrence
    let mut seen: Vec<String> = Vec::new();
    images.retain(|img| {
        if seen.contains(&img.image_name) {
            false
        } else {
            seen.push(img.image_name.clone());
            true
        }
    });

    images
}

/// Find build context for a service in compose file
fn find_build_context_for_service(compose_content: &str, service_name: &str) -> Option<(String, Option<String>)> {
    let lines: Vec<&str> = compose_content.lines().collect();
    let mut in_target_service = false;
    let mut in_build = false;
    let mut context: Option<String> = None;
    let mut dockerfile: Option<String> = None;
    let mut service_indent = 0;

    for line in lines {
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();

        // Find the service
        if trimmed == format!("{}:", service_name) && indent == 2 {
            in_target_service = true;
            service_indent = indent;
            continue;
        }

        // Exit service if we hit another service at same level
        if in_target_service && indent == service_indent && trimmed.ends_with(':') && !trimmed.contains(' ') {
            break;
        }

        if !in_target_service {
            continue;
        }

        // Look for build section
        if trimmed == "build:" || trimmed.starts_with("build:") {
            in_build = true;
            // Handle inline build: ./context format
            if let Some(ctx) = trimmed.strip_prefix("build:") {
                let ctx = ctx.trim();
                if !ctx.is_empty() {
                    context = Some(ctx.to_string());
                }
            }
            continue;
        }

        if in_build {
            if trimmed.starts_with("context:") {
                if let Some(ctx) = trimmed.strip_prefix("context:") {
                    context = Some(ctx.trim().to_string());
                }
            }
            if trimmed.starts_with("dockerfile:") {
                if let Some(df) = trimmed.strip_prefix("dockerfile:") {
                    dockerfile = Some(df.trim().to_string());
                }
            }
        }
    }

    context.map(|c| (c, dockerfile))
}

/// Check if an image exists on Docker Hub
fn check_image_exists_on_hub(image: &str) -> bool {
    println!("   Checking if {} exists on Docker Hub...", image);

    // Use docker manifest inspect to check if image exists
    let output = Command::new("docker")
        .args(["manifest", "inspect", image])
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                println!("   ✓ {} exists", image);
                true
            } else {
                println!("   ✗ {} not found", image);
                false
            }
        }
        Err(_) => {
            // Fallback: try docker pull with --dry-run or just assume it doesn't exist
            println!("   ✗ Could not check {}, assuming not found", image);
            false
        }
    }
}

/// Build and push missing images to Docker Hub
fn build_and_push_missing_images(images: &[CustomImage], env_vars: &HashMap<String, String>) -> Result<()> {
    let registry = env_vars.get("DOCKER_REGISTRY").cloned().unwrap_or_default();

    if registry.is_empty() {
        anyhow::bail!("DOCKER_REGISTRY not set in .env file. Please set it to your Docker Hub username.");
    }

    // First, authenticate with Docker Hub if needed
    println!("\n🔐 Checking Docker Hub authentication...");
    let auth_check = Command::new("docker")
        .args(["info"])
        .output();

    if auth_check.is_err() {
        println!("⚠️  Docker not available. Please ensure Docker is running.");
        anyhow::bail!("Docker is not available");
    }

    // Check which images need to be built
    let mut images_to_build: Vec<&CustomImage> = Vec::new();

    println!("\n🔍 Checking images on Docker Hub...");
    for image in images {
        if !check_image_exists_on_hub(&image.full_image) {
            if image.build_context.is_some() {
                images_to_build.push(image);
            } else {
                println!("   ⚠️  {} not found and no build context available", image.image_name);
            }
        }
    }

    if images_to_build.is_empty() {
        println!("\n✅ All images exist on Docker Hub");
        return Ok(());
    }

    println!("\n📦 {} image(s) need to be built and pushed:", images_to_build.len());
    for img in &images_to_build {
        println!("   • {} -> {}", img.service_name, img.full_image);
    }

    let should_build = Confirm::new()
        .with_prompt("Build and push these images now?")
        .default(true)
        .interact()?;

    if !should_build {
        anyhow::bail!("Deployment cancelled. Please build and push images manually.");
    }

    // Build and push each image
    for image in images_to_build {
        println!("\n🔨 Building {}...", image.service_name);

        let build_context = image.build_context.as_ref().unwrap();
        let dockerfile = image.dockerfile.as_ref().map(|d| d.as_str()).unwrap_or("Dockerfile");

        // Build the image
        let mut build_cmd = Command::new("docker");
        build_cmd.args(["build", "-t", &image.full_image]);
        build_cmd.args(["-f", &format!("{}/{}", build_context, dockerfile)]);
        build_cmd.arg(build_context);

        let build_status = build_cmd.status().context(format!("Failed to build {}", image.service_name))?;

        if !build_status.success() {
            anyhow::bail!("Failed to build {}", image.service_name);
        }

        println!("   ✓ Built {}", image.full_image);

        // Push the image
        println!("📤 Pushing {}...", image.full_image);

        let push_status = Command::new("docker")
            .args(["push", &image.full_image])
            .status()
            .context(format!("Failed to push {}", image.full_image))?;

        if !push_status.success() {
            anyhow::bail!("Failed to push {}. Make sure you're logged into Docker Hub with 'docker login'", image.full_image);
        }

        println!("   ✓ Pushed {}", image.full_image);
    }

    println!("\n✅ All images built and pushed successfully!");
    Ok(())
}

pub fn execute_remote_command(vps: &shipwright_common::config::VpsConfig, command: &str) -> Result<String> {
    let mut ssh_cmd = Command::new("ssh");
    ssh_cmd.arg("-i").arg(shellexpand::tilde(&vps.ssh_key).to_string().replace("\"", ""));
    ssh_cmd.arg("-o").arg("StrictHostKeyChecking=no");
    ssh_cmd.arg(format!("{}@{}", vps.user, vps.host));
    ssh_cmd.arg(command);

    let output = ssh_cmd.output().context("Failed to execute ssh command")?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        anyhow::bail!("Remote command failed: {}\nStderr: {}", command, stderr);
    }

    print!("{}", stdout);
    let _ = std::io::stdout().flush();
    
    Ok(stdout)
}

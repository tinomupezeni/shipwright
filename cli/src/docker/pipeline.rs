use anyhow::{Result, Context};
use bollard::Docker;
use bollard::image::{BuildImageOptions, PushImageOptions, TagImageOptions};
use bollard::auth::DockerCredentials;
use dialoguer::{Confirm, Input, Password};
use futures_util::stream::StreamExt;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use tar::Builder;
use flate2::write::GzEncoder;
use flate2::Compression;

use shipwright_common::config::Config;

/// Represents a buildable service from docker-compose
#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
    pub context: String,
    pub dockerfile: String,
    pub image_name: String,
}

/// Discover services with build contexts from docker-compose file
pub fn discover_services(compose_file: &str) -> Result<Vec<Service>> {
    let content = fs::read_to_string(compose_file)?;

    // Parse YAML to get services
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)?;

    let mut services = Vec::new();

    if let Some(svc_map) = yaml.get("services").and_then(|s| s.as_mapping()) {
        for (name, config) in svc_map {
            let service_name = name.as_str().unwrap_or("").to_string();

            // Check if service has a build context
            if let Some(build) = config.get("build") {
                let (context, dockerfile) = if build.is_string() {
                    (build.as_str().unwrap_or(".").to_string(), "Dockerfile".to_string())
                } else if build.is_mapping() {
                    let ctx = build.get("context")
                        .and_then(|c| c.as_str())
                        .unwrap_or(".")
                        .to_string();
                    let df = build.get("dockerfile")
                        .and_then(|d| d.as_str())
                        .unwrap_or("Dockerfile")
                        .to_string();
                    (ctx, df)
                } else {
                    continue;
                };

                // Get image name or derive from service name
                let image_name = config.get("image")
                    .and_then(|i| i.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| service_name.clone());

                services.push(Service {
                    name: service_name,
                    context,
                    dockerfile,
                    image_name,
                });
            }
        }
    }

    // Deduplicate by context+dockerfile (some services share build contexts)
    let mut unique_services: Vec<Service> = Vec::new();
    let mut seen: HashMap<String, bool> = HashMap::new();

    for svc in services {
        let key = format!("{}:{}", svc.context, svc.dockerfile);
        if !seen.contains_key(&key) {
            seen.insert(key, true);
            unique_services.push(svc);
        }
    }

    Ok(unique_services)
}

/// Build all services and push to registry
pub async fn build_and_push_services(
    config: &Config,
    compose_file: &str,
    registry_url: &str,
) -> Result<Vec<String>> {
    let services = discover_services(compose_file)?;

    if services.is_empty() {
        println!("ℹ️  No buildable services found in {}", compose_file);
        return Ok(Vec::new());
    }

    println!("\n🔍 Found {} buildable service(s):", services.len());
    for svc in &services {
        println!("   • {} ({})", svc.name, svc.context);
    }

    let should_build = Confirm::new()
        .with_prompt("Build and push these services?")
        .default(true)
        .interact()?;

    if !should_build {
        anyhow::bail!("Build cancelled by user");
    }

    let docker = Docker::connect_with_socket_defaults()
        .context("Failed to connect to Docker. Is Docker running?")?;

    // Get registry credentials
    let credentials = get_registry_credentials(config, registry_url)?;

    let mut pushed_images = Vec::new();

    for (i, service) in services.iter().enumerate() {
        println!("\n[{}/{}] Building {}...", i + 1, services.len(), service.name);

        // Derive the registry image name
        let local_tag = format!("{}:latest", service.name);
        let remote_tag = format!("{}/hbec-{}:latest", registry_url, service.name);

        // Build the image
        build_service_image(&docker, service, &local_tag).await?;

        // Tag for registry
        println!("🏷️  Tagging as {}", remote_tag);
        docker.tag_image(&local_tag, Some(TagImageOptions {
            repo: remote_tag.as_str(),
            tag: "latest",
        })).await.context("Failed to tag image")?;

        // Push to registry
        println!("📤 Pushing to registry...");
        push_image(&docker, &remote_tag, &credentials).await?;

        pushed_images.push(remote_tag);
        println!("✅ {} complete", service.name);
    }

    println!("\n✅ All {} services built and pushed!", services.len());

    Ok(pushed_images)
}

async fn build_service_image(docker: &Docker, service: &Service, tag: &str) -> Result<()> {
    let context_path = Path::new(&service.context);

    if !context_path.exists() {
        anyhow::bail!("Build context not found: {}", service.context);
    }

    let dockerfile_path = context_path.join(&service.dockerfile);
    if !dockerfile_path.exists() {
        anyhow::bail!("Dockerfile not found: {}", dockerfile_path.display());
    }

    println!("📦 Packing build context from {}...", service.context);

    // Create tarball of build context
    let tar_path = format!(".shipwright/{}_context.tar.gz", service.name);
    fs::create_dir_all(".shipwright")?;

    {
        let tar_file = fs::File::create(&tar_path)?;
        let enc = GzEncoder::new(tar_file, Compression::default());
        let mut tar = Builder::new(enc);

        // Add files from context, respecting .dockerignore
        let ignore_patterns = read_dockerignore(context_path);
        add_directory_to_tar(&mut tar, context_path, "", &ignore_patterns)?;

        tar.finish()?;
    }

    let tar_size = fs::metadata(&tar_path)?.len();
    println!("📤 Sending build context ({:.2} MB)...", tar_size as f64 / 1024.0 / 1024.0);

    let mut tar_file = fs::File::open(&tar_path)?;
    let mut buffer = Vec::new();
    std::io::Read::read_to_end(&mut tar_file, &mut buffer)?;

    let build_options = BuildImageOptions::<String> {
        dockerfile: service.dockerfile.clone(),
        t: tag.to_string(),
        rm: true,
        ..Default::default()
    };

    let mut build_stream = docker.build_image(build_options, None, Some(buffer.into()));

    while let Some(msg) = build_stream.next().await {
        let msg = msg?;
        if let Some(stream) = msg.stream {
            print!("{}", stream);
            std::io::stdout().flush()?;
        }
        if let Some(error) = msg.error {
            anyhow::bail!("Build error: {}", error);
        }
    }

    // Cleanup
    let _ = fs::remove_file(&tar_path);

    Ok(())
}

fn read_dockerignore(context_path: &Path) -> Vec<String> {
    let dockerignore_path = context_path.join(".dockerignore");

    if let Ok(content) = fs::read_to_string(dockerignore_path) {
        content
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|s| s.trim().to_string())
            .collect()
    } else {
        // Default ignores
        vec![
            "node_modules".to_string(),
            ".git".to_string(),
            "__pycache__".to_string(),
            "*.pyc".to_string(),
            ".env".to_string(),
            "venv".to_string(),
            ".venv".to_string(),
            "target".to_string(),
        ]
    }
}

fn add_directory_to_tar<W: Write>(
    tar: &mut Builder<W>,
    base_path: &Path,
    prefix: &str,
    ignore_patterns: &[String],
) -> Result<()> {
    for entry in fs::read_dir(base_path)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Check if should be ignored
        let should_ignore = ignore_patterns.iter().any(|pattern| {
            if pattern.ends_with('/') {
                name == pattern.trim_end_matches('/')
            } else if pattern.contains('*') {
                // Simple glob matching
                let regex_pattern = pattern.replace(".", "\\.").replace("*", ".*");
                Regex::new(&format!("^{}$", regex_pattern))
                    .map(|re| re.is_match(&name))
                    .unwrap_or(false)
            } else {
                name == *pattern
            }
        });

        if should_ignore {
            continue;
        }

        let tar_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };

        if path.is_dir() {
            add_directory_to_tar(tar, &path, &tar_path, ignore_patterns)?;
        } else {
            tar.append_path_with_name(&path, &tar_path)?;
        }
    }

    Ok(())
}

fn get_registry_credentials(config: &Config, registry_url: &str) -> Result<DockerCredentials> {
    // Try to get from config first
    if let Some(auth) = &config.deploy.registry.auth {
        if let Ok(token) = fs::read_to_string(&auth.token_file) {
            return Ok(DockerCredentials {
                username: Some(auth.username.clone()),
                password: Some(token.trim().to_string()),
                ..Default::default()
            });
        }
    }

    // Prompt for credentials
    println!("\n🔐 Registry credentials required for {}", registry_url);

    let username: String = Input::new()
        .with_prompt("Username")
        .interact_text()?;

    let password: String = Password::new()
        .with_prompt("Password/Token")
        .interact()?;

    Ok(DockerCredentials {
        username: Some(username),
        password: Some(password),
        ..Default::default()
    })
}

async fn push_image(docker: &Docker, image: &str, credentials: &DockerCredentials) -> Result<()> {
    let mut push_stream = docker.push_image(
        image,
        Some(PushImageOptions::<String>::default()),
        Some(credentials.clone()),
    );

    while let Some(msg) = push_stream.next().await {
        let msg = msg?;
        if let Some(status) = msg.status {
            if status.contains("Pushing") || status.contains("Pushed") || status.contains("Layer") {
                print!("\r   {}", status);
                std::io::stdout().flush()?;
            }
        }
        if let Some(error) = msg.error {
            println!();
            anyhow::bail!("Push error: {}", error);
        }
    }
    println!();

    Ok(())
}

/// Check if images exist in the registry
pub async fn check_images_exist(registry_url: &str, services: &[Service]) -> Result<Vec<Service>> {
    let mut missing = Vec::new();

    println!("🔍 Checking for existing images in registry...");

    for service in services {
        let image = format!("{}/hbec-{}:latest", registry_url, service.name);

        // Try to get image manifest (this is a simplified check)
        // In production, you'd use registry API to check
        let docker = Docker::connect_with_socket_defaults()?;

        match docker.inspect_image(&image).await {
            Ok(_) => {
                println!("   ✓ {} exists locally", service.name);
            }
            Err(_) => {
                println!("   ✗ {} not found", service.name);
                missing.push(service.clone());
            }
        }
    }

    Ok(missing)
}

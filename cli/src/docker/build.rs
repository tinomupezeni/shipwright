use anyhow::{Result, Context};
use bollard::Docker;
use bollard::image::BuildImageOptions;
use futures_util::stream::StreamExt;
use std::io::Write;
use tracing::info;
use shipwright_common::config::Config;
use tar::Builder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::File;

use std::path::Path;

pub async fn build_image(config: &Config) -> Result<()> {
    let docker = Docker::connect_with_socket_defaults()?;
    
    let image_name = format!("{}:latest", config.project.name);

    if !Path::new("Dockerfile").exists() {
        if config.deploy.deploy_type == "docker-compose" {
            println!("ℹ️  No Dockerfile found at root. Skipping build phase and assuming images are defined in docker-compose.");
            return Ok(());
        }
        anyhow::bail!("Dockerfile not found. Run 'shipwright init' or create a Dockerfile.");
    }
    
    info!("Building Docker image: {}", image_name);
    println!("📦 Packing project context...");
    
    // Create a temporary tarball of the current directory
    let tar_gz_path = ".shipwright/context.tar.gz";
    {
        let tar_gz = File::create(tar_gz_path).context("Failed to create tar.gz file")?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = Builder::new(enc);
        
        // Add current directory to tarball, ignoring some common folders
        let entries = std::fs::read_dir(".")?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            
            // Skip heavy or sensitive folders
            if name == "node_modules" || name == "target" || name == ".git" || name == ".shipwright" || name == "venv" || name == "context.tar.gz" {
                continue;
            }
            
            if path.is_dir() {
                tar.append_dir_all(name, &path)?;
            } else {
                println!("  + {}", name);
                tar.append_path_with_name(&path, name)?;
            }
        }
        tar.finish().context("Failed to finish tarball")?;
        let mut enc = tar.into_inner().context("Failed to get encoder")?;
        enc.finish().context("Failed to finish gzip compression")?;
    }

    println!("📤 Sending build context to Docker ({:.2} MB)...", 
        File::open(tar_gz_path)?.metadata()?.len() as f64 / 1024.0 / 1024.0);

    let mut file = File::open(tar_gz_path).context("Failed to open context tarball")?;
    let mut buffer = Vec::new();
    use std::io::Read;
    file.read_to_end(&mut buffer).context("Failed to read context tarball")?;

    let build_options = BuildImageOptions {
        dockerfile: "Dockerfile",
        t: &image_name,
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

    info!("Successfully built {}", image_name);
    
    Ok(())
}

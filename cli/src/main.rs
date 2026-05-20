mod commands;
mod docker;
mod db;
mod websocket;

use clap::{Parser, Subcommand};
use anyhow::Result;
use std::fs;
use std::sync::{Arc, Mutex};
use shipwright_common::config::Config;

#[derive(Parser)]
#[command(name = "shipwright")]
#[command(about = "A deployment tool that makes CI/CD trustworthy", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new Shipwright project
    Init,
    /// Setup the remote VPS (install Docker, Agent, etc.)
    Setup,
    /// Build and deploy the project
    Up {
        #[arg(short, long)]
        dry_run: bool,
    },
    /// View live logs
    Logs,
    /// Check the status of the deployment
    Status,
    /// Install git hooks
    Hooks {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Register the project for automatic deployments (Mini-PaaS)
    Register,
    /// Watch live build logs from the VPS
    Watch,
    /// Update the Shipwright Agent on the VPS
    UpdateAgent,
    /// Manage secrets for deployments
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
    /// Retry the last failed deployment
    Retry,
    /// Show version information
    Version,
    /// Check for and install updates
    Update {
        /// Update the remote agent on the VPS
        #[arg(long)]
        agent: bool,
    },
}

#[derive(Subcommand)]
enum SecretsAction {
    /// Set a secret value
    Set {
        /// Secret name
        name: Option<String>,
        /// Secret value (will prompt if not provided)
        #[arg(short, long)]
        value: Option<String>,
        /// Tags for organizing secrets
        #[arg(short, long)]
        tags: Vec<String>,
    },
    /// Get a secret value
    Get {
        /// Secret name
        name: String,
        /// Show the actual value (default: hidden)
        #[arg(short, long)]
        show: bool,
    },
    /// List all secrets
    List {
        /// Show secret values (default: names only)
        #[arg(short = 'v', long)]
        with_values: bool,
    },
    /// Delete a secret
    Delete {
        /// Secret name
        name: String,
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
    /// Export secrets as .env format
    Export,
}

#[derive(Subcommand)]
enum HookAction {
    /// Install pre-push and pre-commit hooks
    Install,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    // Background update check (silent)
    let _ = commands::update::check_for_updates_silently().await;
    
    // Ensure .shipwright directory exists
    fs::create_dir_all(".shipwright")?;

    // Initialize DB
    let conn = db::init_db()?;
    let conn = Arc::new(Mutex::new(conn));
    
    let cli = Cli::parse();

    // Spawn metrics collector if VPS is configured
    if let Ok(config_content) = fs::read_to_string(".shipwright.yml") {
        if let Ok(config) = serde_yaml::from_str::<Config>(&config_content) {
            if let Some(vps) = &config.deploy.vps {
                // Use fixed Agent port (17671)
                // This must match agent/src/main.rs SHIPWRIGHT_WS_PORT
                let ws_port = 17671;
                let url = format!("ws://{}:{}", vps.host, ws_port);
                let conn_clone = Arc::clone(&conn);
                tokio::spawn(async move {
                    let _ = websocket::client::connect_to_agent(&url, conn_clone).await;
                });
            }
        }
    }

    match &cli.command {
        Commands::Init => {
            commands::init::run().await?;
        }
        Commands::Setup => {
            commands::setup::run().await?;
        }
        Commands::Up { dry_run } => {
            commands::deploy::run(*dry_run).await?;
        }
        Commands::Logs => {
            commands::logs::run().await?;
        }
        Commands::Status => {
            commands::status::run().await?;
        }
        Commands::Register => {
            commands::register::run().await?;
        }
        Commands::Watch => {
            commands::watch::run().await?;
        }
        Commands::UpdateAgent => {
            commands::update_agent::run().await?;
        }
        Commands::Hooks { action } => {
            match action {
                HookAction::Install => {
                    commands::hooks::install().await?;
                }
            }
        }
        Commands::Secrets { action } => {
            match action {
                SecretsAction::Set { name, value, tags } => {
                    commands::secrets::run_set(name.clone(), value.clone(), tags.clone()).await?;
                }
                SecretsAction::Get { name, show } => {
                    commands::secrets::run_get(name.clone(), *show).await?;
                }
                SecretsAction::List { with_values } => {
                    commands::secrets::run_list(*with_values).await?;
                }
                SecretsAction::Delete { name, force } => {
                    commands::secrets::run_delete(name.clone(), *force).await?;
                }
                SecretsAction::Export => {
                    commands::secrets::run_export().await?;
                }
            }
        }
        Commands::Retry => {
            commands::retry::run().await?;
        }
        Commands::Version => {
            commands::version::run();
        }
        Commands::Update { agent } => {
            commands::update::run(*agent).await?;
        }
    }

    Ok(())
}

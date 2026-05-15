use anyhow::{Result, Context};
use shipwright_common::config::Config;
use std::fs;
use std::path::Path;
use tokio_tungstenite::connect_async;
use futures_util::StreamExt;
use shipwright_common::protocol::{AgentMessage, BuildEvent};
use tracing::error;

pub async fn run() -> Result<()> {
    let config_path = Path::new(".shipwright.yml");
    if !config_path.exists() {
        anyhow::bail!(".shipwright.yml not found. Run 'shipwright init' first.");
    }

    let config_content = fs::read_to_string(config_path)?;
    let config: Config = serde_yaml::from_str(&config_content)?;
    
    let vps = config.deploy.vps.as_ref().context("No VPS configured in .shipwright.yml")?;
    let url = format!("ws://{}:8081", vps.host);

    println!("👀 Watching for build events from {}...", vps.host);
    
    let (ws_stream, _) = connect_async(&url).await.context("Failed to connect to agent")?;
    let (_, mut ws_receiver) = ws_stream.split();

    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                let message: AgentMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(_) => continue, // Ignore non-JSON or other messages
                };

                match message {
                    AgentMessage::BuildUpdate { project_name, event } => {
                        match event {
                            BuildEvent::Started => {
                                println!("\n🚀 Build started for project: {}", project_name);
                            }
                            BuildEvent::Log(line) => {
                                println!("  {}", line);
                            }
                            BuildEvent::Success => {
                                println!("✅ Build and deployment successful for {}!", project_name);
                            }
                            BuildEvent::Failed(err) => {
                                println!("❌ Build failed for {}: {}", project_name, err);
                            }
                        }
                    }
                    _ => {} // Ignore metrics/health in watch for now
                }
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                println!("Connection closed by agent.");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

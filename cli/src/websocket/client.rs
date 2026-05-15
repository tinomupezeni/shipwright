use anyhow::{Result, Context};
use tokio_tungstenite::connect_async;
use futures_util::StreamExt;
use shipwright_common::protocol::AgentMessage;
use tracing::{info, error};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

pub async fn connect_to_agent(url: &str, conn: Arc<Mutex<Connection>>) -> Result<()> {
    info!("Connecting to Agent WebSocket at: {}", url);
    
    let (ws_stream, _) = connect_async(url).await.context("Failed to connect to agent")?;
    let (mut _ws_sender, mut ws_receiver) = ws_stream.split();

    info!("Connected to agent. Listening for messages...");

    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                let message: AgentMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!("Error parsing agent message: {}", e);
                        continue;
                    }
                };

                match message {
                    AgentMessage::Metrics(snapshot) => {
                        // Store metrics in DB
                        let db = conn.lock().unwrap();
                        db.execute(
                            "INSERT INTO metrics (timestamp, deploy_id, metric_name, metric_value) 
                             VALUES (?1, ?2, ?3, ?4)",
                            (
                                snapshot.timestamp.timestamp(),
                                "current", // For now, we use "current" as deploy_id
                                "cpu_usage",
                                snapshot.cpu_usage as f64,
                            ),
                        )?;
                        db.execute(
                            "INSERT INTO metrics (timestamp, deploy_id, metric_name, metric_value) 
                             VALUES (?1, ?2, ?3, ?4)",
                            (
                                snapshot.timestamp.timestamp(),
                                "current",
                                "memory_used",
                                snapshot.memory_used as f64,
                            ),
                        )?;
                    }
                    _ => {}
                }
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                info!("Agent connection closed");
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

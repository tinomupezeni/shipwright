use anyhow::{Result, Context};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;
use futures_util::{StreamExt, SinkExt};
use shipwright_common::protocol::{AgentMessage, CliCommand};
use crate::metrics::collector::Collector;
use crate::websocket::message_buffer::MessageBuffer;
use tracing::{info, error};
use tokio::time::{interval, Duration};
use chrono::Utc;
use shipwright_common::metrics::SystemSnapshot;
use tokio::sync::broadcast;

pub async fn start_server(
    addr: &str,
    broadcast_tx: broadcast::Sender<AgentMessage>,
    message_buffer: MessageBuffer,
) -> Result<()> {
    let listener = TcpListener::bind(addr).await.context("Failed to bind to address")?;
    info!("WebSocket server listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let rx = broadcast_tx.subscribe();
        let buffer = message_buffer.clone();
        tokio::spawn(handle_connection(stream, rx, buffer));
    }

    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    mut broadcast_rx: broadcast::Receiver<AgentMessage>,
    message_buffer: MessageBuffer,
) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            error!("Error during WebSocket handshake: {}", e);
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let mut collector = Collector::new();
    let mut metrics_interval = interval(Duration::from_secs(2));

    info!("New CLI connection established");

    // Replay buffered messages for context
    let buffered_messages = message_buffer.get_all();
    if !buffered_messages.is_empty() {
        info!("Replaying {} buffered messages to new client", buffered_messages.len());
        for msg in buffered_messages {
            let json = serde_json::to_string(&msg).unwrap();
            if let Err(e) = ws_sender.send(tokio_tungstenite::tungstenite::Message::Text(json)).await {
                error!("Error sending buffered message: {}", e);
                return;
            }
        }
    }

    loop {
        tokio::select! {
            _ = metrics_interval.tick() => {
                let metrics = collector.collect();
                let snapshot = SystemSnapshot {
                    timestamp: Utc::now(),
                    cpu_usage: metrics.cpu_usage,
                    memory_used: metrics.memory_used,
                    memory_total: metrics.memory_total,
                    disk_usage: 0.0,
                };
                
                let message = AgentMessage::Metrics(snapshot);
                let json = serde_json::to_string(&message).unwrap();
                if let Err(e) = ws_sender.send(tokio_tungstenite::tungstenite::Message::Text(json)).await {
                    error!("Error sending metrics: {}", e);
                    break;
                }
            }
            Ok(msg) = broadcast_rx.recv() => {
                let json = serde_json::to_string(&msg).unwrap();
                if let Err(e) = ws_sender.send(tokio_tungstenite::tungstenite::Message::Text(json)).await {
                    error!("Error sending broadcast message: {}", e);
                    break;
                }
            }
            Some(msg) = ws_receiver.next() => {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        let command: CliCommand = match serde_json::from_str(&text) {
                            Ok(c) => c,
                            Err(e) => {
                                error!("Error parsing CLI command: {}", e);
                                continue;
                            }
                        };
                        info!("Received CLI command: {:?}", command);
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                        info!("CLI connection closed");
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
}

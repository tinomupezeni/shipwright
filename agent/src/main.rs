mod metrics;
mod websocket;
mod db;
mod webhooks;
mod pipeline;

use tracing_subscriber;
use std::sync::{Arc, Mutex};
use tracing::info;
use crate::webhooks::server::AppState;
use tokio::sync::broadcast;
use shipwright_common::protocol::AgentMessage;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    
    info!("Initializing Shipwright Agent DB...");
    let conn = db::init_db()?;
    let conn = Arc::new(Mutex::new(conn));

    // Channel for broadcasting build events to all connected WebSocket clients
    let (tx, _rx) = broadcast::channel::<AgentMessage>(100);

    let ws_addr = "0.0.0.0:8081";
    let http_addr = "0.0.0.0:8082";

    info!("Starting Shipwright Agent...");

    let state = AppState {
        db: Arc::clone(&conn),
        broadcast_tx: tx.clone(),
    };

    let ws_tx = tx.clone();
    let ws_handle = tokio::spawn(async move {
        websocket::server::start_server(ws_addr, ws_tx).await
    });

    let http_handle = tokio::spawn(async move {
        webhooks::server::start_server(http_addr, state).await
    });

    info!("Agent is running. WebSockets: {}, Webhooks: {}", ws_addr, http_addr);

    // Wait for either server to finish (though they should run forever)
    tokio::select! {
        res = ws_handle => res??,
        res = http_handle => res??,
    }

    Ok(())
}

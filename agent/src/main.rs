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

use std::net::TcpListener;
use std::fs;

fn find_available_port(start_port: u16) -> u16 {
    let mut port = start_port;
    loop {
        if TcpListener::bind(format!("0.0.0.0:{}", port)).is_ok() {
            return port;
        }
        port += 1;
        if port > start_port + 100 {
            panic!("Could not find an available port in range {}-{}", start_port, start_port + 100);
        }
    }
}

fn open_firewall_port(port: u16) {
    info!("🛡️  Automatically opening port {} in firewall...", port);
    let output = std::process::Command::new("sudo")
        .args(["ufw", "allow", &format!("{}/tcp", port)])
        .output();
    
    match output {
        Ok(out) if out.status.success() => info!("✅ Port {} opened successfully.", port),
        Ok(out) => info!("⚠️  Note: Could not automatically open port {} (UFW might be disabled or missing sudo)", port),
        Err(_) => info!("⚠️  Note: Firewall command failed. Please ensure port {} is open manually.", port),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    
    info!("Initializing Shipwright Agent DB...");
    let conn = db::init_db()?;
    let conn = Arc::new(Mutex::new(conn));

    // Channel for broadcasting build events to all connected WebSocket clients
    let (tx, _rx) = broadcast::channel::<AgentMessage>(100);

    // Dynamic Port Discovery
    let ws_port = find_available_port(8081);
    let http_port = find_available_port(8083);

    let ws_addr = format!("0.0.0.0:{}", ws_port);
    let http_addr = format!("0.0.0.0:{}", http_port);

    // Automatic Firewall Management
    open_firewall_port(ws_port);
    open_firewall_port(http_port);

    // Persist port selection for CLI discovery
    let state_dir = "/etc/shipwright";
    let _ = fs::create_dir_all(state_dir);
    let state_file = format!("{}/agent.env", state_dir);
    let state_content = format!("SHIPWRIGHT_WS_PORT={}\nSHIPWRIGHT_HTTP_PORT={}\n", ws_port, http_port);
    if let Err(_e) = fs::write(&state_file, state_content) {
        // Fallback to local directory if /etc isn't writable (e.g. during dev)
        fs::write("agent.env", format!("SHIPWRIGHT_WS_PORT={}\nSHIPWRIGHT_HTTP_PORT={}\n", ws_port, http_port))?;
    }

    info!("Starting Shipwright Agent...");

    let state = AppState {
        db: Arc::clone(&conn),
        broadcast_tx: tx.clone(),
    };

    let ws_addr_clone = ws_addr.clone();
    let ws_tx = tx.clone();
    let ws_handle = tokio::spawn(async move {
        websocket::server::start_server(&ws_addr_clone, ws_tx).await
    });

    let http_addr_clone = http_addr.clone();
    let http_handle = tokio::spawn(async move {
        webhooks::server::start_server(&http_addr_clone, state).await
    });

    info!("Agent is running. WebSockets: {}, Webhooks: {}", ws_addr, http_addr);

    // Wait for either server to finish (though they should run forever)
    tokio::select! {
        res = ws_handle => res??,
        res = http_handle => res??,
    }

    Ok(())
}

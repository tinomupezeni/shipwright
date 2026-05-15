mod metrics;
mod websocket;
mod db;
mod webhooks;
mod pipeline;

use tracing_subscriber;
use std::sync::{Arc, Mutex};
use tracing::{info, error};
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
    info!("🛡️  Opening port {} in firewall...", port);
    
    // 1. Try UFW first (common on Ubuntu/Debian)
    let ufw_output = std::process::Command::new("ufw")
        .args(["allow", &format!("{}/tcp", port)])
        .output();
    
    if let Ok(out) = ufw_output {
        if out.status.success() {
            info!("✅ UFW: Port {} opened.", port);
            return;
        }
    }

    // 2. Fallback to raw iptables if UFW failed or isn't present
    info!("🔗 UFW failed, trying raw iptables fallback for port {}...", port);
    let iptables_output = std::process::Command::new("iptables")
        .args(["-I", "INPUT", "-p", "tcp", "--dport", &port.to_string(), "-j", "ACCEPT"])
        .output();

    match iptables_output {
        Ok(out) if out.status.success() => info!("✅ iptables: Port {} opened.", port),
        Ok(out) => error!("❌ Both UFW and iptables failed for port {}: {}", port, String::from_utf8_lossy(&out.stderr)),
        Err(e) => error!("❌ Firewall execution error: {}", e),
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

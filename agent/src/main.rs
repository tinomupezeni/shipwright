mod metrics;
mod websocket;
mod db;
mod webhooks;
mod pipeline;
mod infrastructure;
mod smoke_tests;
mod env_validator;
mod crypto;
mod secrets;

use tracing_subscriber;
use std::sync::{Arc, Mutex};
use tracing::{info, error};
use crate::webhooks::server::AppState;
use tokio::sync::broadcast;
use shipwright_common::protocol::AgentMessage;

use std::fs;

// Fixed ports for Shipwright Agent
// Chosen to avoid conflicts with common web development ports (3000-9999)
const SHIPWRIGHT_WS_PORT: u16 = 17671;
const SHIPWRIGHT_HTTP_PORT: u16 = 17670;

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

    info!("🚢 Shipwright Agent v0.1.3 starting...");

    // Detect infrastructure on startup
    info!("🔍 Detecting infrastructure...");
    match infrastructure::detect_infrastructure().await {
        Ok(infra_info) => {
            if let Some((proxy_type, container_name)) = &infra_info.proxy {
                info!("   ✓ Detected {} proxy: {}", proxy_type, container_name);
            }
            info!("   ✓ Found {} Docker networks", infra_info.networks.len());
            if infra_info.shared_resources.postgres.is_some() {
                info!("   ✓ Found shared PostgreSQL");
            }
            if infra_info.shared_resources.redis.is_some() {
                info!("   ✓ Found shared Redis");
            }
            if infra_info.is_multi_project {
                info!("   ✓ Multi-project setup detected");
            }

            let strategy = infrastructure::detector::recommend_strategy(&infra_info);
            info!("   ✓ Recommended deployment strategy: {}", strategy);
        }
        Err(e) => {
            error!("   ✗ Infrastructure detection failed: {}", e);
        }
    }

    info!("Initializing Shipwright Agent DB...");
    let conn = db::init_db()?;
    let conn = Arc::new(Mutex::new(conn));

    // Channel for broadcasting build events to all connected WebSocket clients
    let (tx, _rx) = broadcast::channel::<AgentMessage>(100);

    // Check if running in Docker
    let is_docker = std::path::Path::new("/.dockerenv").exists();

    // Port Configuration - use env vars if set (for testing), otherwise use fixed ports
    let ws_port = std::env::var("SHIPWRIGHT_WS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(SHIPWRIGHT_WS_PORT);

    let http_port = std::env::var("SHIPWRIGHT_HTTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(SHIPWRIGHT_HTTP_PORT);

    // Verify ports are available before proceeding
    if std::net::TcpListener::bind(format!("0.0.0.0:{}", ws_port)).is_err() {
        error!("❌ Port {} is already in use!", ws_port);
        error!("   Shipwright Agent requires port {} for WebSocket connections.", ws_port);
        error!("   Please free this port or set SHIPWRIGHT_WS_PORT environment variable to use a different port.");
        anyhow::bail!("Port {} unavailable", ws_port);
    }

    if std::net::TcpListener::bind(format!("0.0.0.0:{}", http_port)).is_err() {
        error!("❌ Port {} is already in use!", http_port);
        error!("   Shipwright Agent requires port {} for HTTP webhook connections.", http_port);
        error!("   Please free this port or set SHIPWRIGHT_HTTP_PORT environment variable to use a different port.");
        anyhow::bail!("Port {} unavailable", http_port);
    }

    let ws_addr = format!("0.0.0.0:{}", ws_port);
    let http_addr = format!("0.0.0.0:{}", http_port);

    // Automatic Firewall Management (skip in Docker)
    if !is_docker {
        open_firewall_port(ws_port);
        open_firewall_port(http_port);
    } else {
        info!("🐳 Running in Docker - skipping firewall management");
    }

    // Persist port selection for CLI discovery (not needed in Docker)
    if !is_docker {
        let state_dir = "/etc/shipwright";
        let _ = fs::create_dir_all(state_dir);
        let state_file = format!("{}/agent.env", state_dir);
        let state_content = format!("SHIPWRIGHT_WS_PORT={}\nSHIPWRIGHT_HTTP_PORT={}\n", ws_port, http_port);
        if let Err(_e) = fs::write(&state_file, state_content) {
            // Fallback to local directory if /etc isn't writable (e.g. during dev)
            fs::write("agent.env", format!("SHIPWRIGHT_WS_PORT={}\nSHIPWRIGHT_HTTP_PORT={}\n", ws_port, http_port))?;
        }
    } else {
        info!("🐳 Running in Docker - ports configured via environment");
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

use serde::{Deserialize, Serialize};
use crate::metrics::SystemSnapshot;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum AgentMessage {
    /// Initial handshake from agent to CLI
    Connect { 
        agent_id: String,
        hostname: String,
        version: String 
    },
    /// Periodic metrics update
    Metrics(SystemSnapshot),
    /// Health check result
    HealthResult {
        check_name: String,
        success: bool,
        message: Option<String>,
        duration_ms: u64,
    },
    /// Log line stream
    LogLine {
        container_name: String,
        line: String,
        timestamp: String,
    },
    /// Error notification
    Error(String),
    /// Build status update
    BuildUpdate {
        project_name: String,
        event: BuildEvent,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum BuildEvent {
    Started,
    Log(String),
    Success,
    Failed(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum CliCommand {
    /// Request a health check
    RunHealthCheck {
        check_type: String, // http, tcp, command
        target: String,
    },
    /// Start streaming logs
    StreamLogs {
        container_name: String,
        follow: bool,
    },
    /// Execute a remote command
    Execute(String),
}

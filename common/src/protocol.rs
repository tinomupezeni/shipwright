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
    /// Rollback status update
    RollbackUpdate {
        project_name: String,
        event: RollbackEvent,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum BuildEvent {
    Started,
    Log(String),
    Success,
    Failed(String),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum RollbackEvent {
    /// Snapshot creation started
    SnapshotStarted {
        snapshot_id: String,
        strategy: String,
    },
    /// Snapshot created successfully
    SnapshotCreated {
        snapshot_id: String,
        strategy: String,
    },
    /// Rollback initiated
    RollbackStarted {
        from_snapshot_id: String,
        to_snapshot_id: String,
        reason: String,
    },
    /// Rollback progress update
    RollbackProgress(String),
    /// Rollback completed successfully
    RollbackSuccess {
        snapshot_id: String,
        duration_secs: u64,
    },
    /// Rollback failed
    RollbackFailed {
        error: String,
    },
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

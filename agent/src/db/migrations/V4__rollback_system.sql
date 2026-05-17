-- Rollback System Schema
-- Tracks deployment history and enables rollback capabilities

-- Deployment snapshots for rollback
CREATE TABLE deployment_snapshots (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    deployed_at INTEGER NOT NULL,
    status TEXT NOT NULL, -- 'active', 'rolled_back', 'failed', 'superseded'
    strategy TEXT NOT NULL, -- 'image-tagging', 'git-commit', 'snapshot'

    -- Image information for image-tagging strategy
    image_tags TEXT, -- JSON: {"service": "tag"}

    -- Git information for git-commit strategy
    git_branch TEXT,
    git_message TEXT,

    -- Snapshot paths for snapshot strategy
    snapshot_path TEXT,
    database_backup_path TEXT,

    -- Test results
    smoke_test_passed BOOLEAN,
    smoke_test_results TEXT, -- JSON

    -- Metadata
    triggered_by TEXT, -- 'webhook', 'cli', 'manual'
    rollback_from_id TEXT, -- Reference to deployment that was rolled back

    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

-- Service-specific deployment info
CREATE TABLE service_deployments (
    id TEXT PRIMARY KEY,
    snapshot_id TEXT NOT NULL,
    service_name TEXT NOT NULL,

    -- Container state
    container_id TEXT,
    image_name TEXT,
    image_tag TEXT,

    -- Health status
    health_status TEXT, -- 'healthy', 'unhealthy', 'starting', 'failed'
    health_check_output TEXT,

    -- Rollback strategy for this specific service
    rollback_strategy TEXT,

    -- Performance metrics
    startup_time_ms INTEGER,
    memory_usage_mb INTEGER,

    FOREIGN KEY(snapshot_id) REFERENCES deployment_snapshots(id) ON DELETE CASCADE
);

-- Rollback events log
CREATE TABLE rollback_events (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    from_snapshot_id TEXT NOT NULL,
    to_snapshot_id TEXT NOT NULL,

    reason TEXT NOT NULL, -- 'smoke_test_failure', 'manual', 'health_check_failure'
    failure_details TEXT, -- JSON with error details

    rollback_started_at INTEGER NOT NULL,
    rollback_completed_at INTEGER,
    rollback_success BOOLEAN,

    performed_by TEXT, -- 'auto', 'cli', username

    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY(from_snapshot_id) REFERENCES deployment_snapshots(id),
    FOREIGN KEY(to_snapshot_id) REFERENCES deployment_snapshots(id)
);

-- Indexes for performance
CREATE INDEX idx_snapshots_project_id ON deployment_snapshots(project_id);
CREATE INDEX idx_snapshots_status ON deployment_snapshots(status);
CREATE INDEX idx_snapshots_deployed_at ON deployment_snapshots(deployed_at DESC);
CREATE INDEX idx_service_deployments_snapshot ON service_deployments(snapshot_id);
CREATE INDEX idx_rollback_events_project ON rollback_events(project_id);
CREATE INDEX idx_rollback_events_started ON rollback_events(rollback_started_at DESC);

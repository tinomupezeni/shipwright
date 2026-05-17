-- Deployment Tracking Schema
-- Tracks deployment attempts for retry functionality

-- Deployment attempts history
CREATE TABLE deployment_attempts (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    project_name TEXT NOT NULL,
    commit_sha TEXT NOT NULL,

    -- Deployment context
    deploy_dir TEXT NOT NULL,
    config_path TEXT NOT NULL,
    triggered_by TEXT NOT NULL, -- 'webhook', 'cli', 'retry'

    -- Status tracking
    status TEXT NOT NULL, -- 'pending', 'running', 'success', 'failed'
    started_at INTEGER NOT NULL,
    completed_at INTEGER,

    -- Failure details
    failure_reason TEXT,
    failure_details TEXT, -- JSON with detailed error info

    -- Retry tracking
    retry_count INTEGER DEFAULT 0,
    original_attempt_id TEXT, -- Reference to original deployment if this is a retry

    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

-- Indexes for performance
CREATE INDEX idx_deployment_attempts_project ON deployment_attempts(project_id);
CREATE INDEX idx_deployment_attempts_status ON deployment_attempts(status);
CREATE INDEX idx_deployment_attempts_started ON deployment_attempts(started_at DESC);

-- View for latest deployment per project
CREATE VIEW latest_deployments AS
SELECT *
FROM deployment_attempts
WHERE id IN (
    SELECT id
    FROM deployment_attempts d1
    WHERE started_at = (
        SELECT MAX(started_at)
        FROM deployment_attempts d2
        WHERE d2.project_id = d1.project_id
    )
);

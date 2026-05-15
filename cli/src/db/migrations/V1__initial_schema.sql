-- Deploy history
CREATE TABLE deploys (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    environment TEXT NOT NULL,
    status TEXT NOT NULL,  -- pending, running, success, failed
    started_at INTEGER NOT NULL,
    completed_at INTEGER,
    deployed_by TEXT,
    rollback_from TEXT,
    confidence_score INTEGER
);

-- Metrics
CREATE TABLE metrics (
    timestamp INTEGER NOT NULL,
    deploy_id TEXT NOT NULL,
    metric_name TEXT NOT NULL,
    metric_value REAL NOT NULL,
    labels TEXT,
    FOREIGN KEY(deploy_id) REFERENCES deploys(id)
);

CREATE INDEX idx_metrics_time ON metrics(timestamp);
CREATE INDEX idx_metrics_deploy ON metrics(deploy_id);

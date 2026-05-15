CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    repo_url TEXT NOT NULL,
    webhook_secret TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE deployments (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    completed_at INTEGER,
    log_output TEXT,
    FOREIGN KEY(project_id) REFERENCES projects(id)
);

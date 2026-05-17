-- Secret Storage Schema
-- Implements the Secret Management Protocol v1 (SMP/v1)

-- Secret stores (one per project)
-- Note: project_id is a string identifier, not a foreign key
-- This allows secrets to be managed independently of webhook registration
CREATE TABLE secret_stores (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL UNIQUE,
    agent_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Individual secrets within a store
CREATE TABLE secrets (
    id TEXT PRIMARY KEY,
    store_id TEXT NOT NULL,
    name TEXT NOT NULL,
    value_encrypted BLOB NOT NULL,
    nonce BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    tags TEXT, -- JSON array
    FOREIGN KEY(store_id) REFERENCES secret_stores(id) ON DELETE CASCADE,
    UNIQUE(store_id, name)
);

-- Audit log for secret access and modifications
CREATE TABLE secret_audit_log (
    id TEXT PRIMARY KEY,
    secret_id TEXT,
    store_id TEXT NOT NULL,
    action TEXT NOT NULL, -- 'created', 'updated', 'deleted', 'accessed'
    secret_name TEXT, -- Store name for deleted secrets
    performed_by TEXT, -- 'cli', 'agent', 'webhook', or user identifier
    timestamp INTEGER NOT NULL,
    FOREIGN KEY(secret_id) REFERENCES secrets(id) ON DELETE SET NULL,
    FOREIGN KEY(store_id) REFERENCES secret_stores(id) ON DELETE CASCADE
);

-- Backup metadata for tracking encrypted backups
CREATE TABLE backup_metadata (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    backup_path TEXT NOT NULL,
    checksum TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

-- Indexes for performance
CREATE INDEX idx_secrets_store_id ON secrets(store_id);
CREATE INDEX idx_secrets_name ON secrets(name);
CREATE INDEX idx_audit_log_store_id ON secret_audit_log(store_id);
CREATE INDEX idx_audit_log_timestamp ON secret_audit_log(timestamp);
CREATE INDEX idx_backup_project_id ON backup_metadata(project_id);

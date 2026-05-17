# Secret Management Architecture

## Overview

Agent-managed secret store with multi-layer backup and portability, addressing:
- **Coupling**: Uses standard encrypted format, portable between agents
- **Data Loss**: 4-layer backup system with automatic syncing

## Architecture

### 1. Storage Layers

```
┌──────────────────────┐
│   CLI (Local)        │
│  ~/.shipwright/      │
│    secrets/          │  ← Layer 3: Remote backup
│      project1.enc    │
│      project2.enc    │
└──────────┬───────────┘
           │ HTTPS
           ▼
┌──────────────────────┐
│   Agent (VPS)        │
│                      │
│  secrets.db          │  ← Layer 1: Primary storage
│    (encrypted)       │
│                      │
│  backups/            │  ← Layer 2: Local backup
│    project1.enc      │
│    project2.enc      │
│    (auto-backup)     │
└──────────────────────┘
```

### 2. Encryption Strategy

**Encryption Key Hierarchy:**

```
Master Key (per project)
  └─> Generated from:
      - Project name
      - Agent installation ID
      - User-provided passphrase (optional)

Secret Encryption:
  └─> AES-256-GCM with project master key
      - Each secret value encrypted separately
      - Metadata (name, timestamps) in plaintext
```

**Why this approach:**
- Each project has separate encryption key
- Compromising one project doesn't expose others
- User can add passphrase for extra security
- Standard encryption (AES-256-GCM)

### 3. File Format

**Encrypted Secret File** (`.enc` extension):

```json
{
  "version": 1,
  "format": "shipwright-secrets-v1",
  "project": "TESE-MARKET---BFF-ARCHITECTURE",
  "agent_id": "agent-uuid-here",
  "created_at": "2026-05-16T14:00:00Z",
  "updated_at": "2026-05-16T15:30:00Z",
  "encryption": {
    "algorithm": "AES-256-GCM",
    "key_derivation": "PBKDF2-SHA256"
  },
  "secrets": [
    {
      "name": "GITHUB_REPOSITORY_OWNER",
      "value_encrypted": "base64-encrypted-value",
      "nonce": "base64-nonce",
      "created_at": "2026-05-16T14:00:00Z",
      "updated_at": "2026-05-16T14:00:00Z",
      "tags": ["github", "auth"]
    },
    {
      "name": "JWT_SECRET",
      "value_encrypted": "base64-encrypted-value",
      "nonce": "base64-nonce",
      "created_at": "2026-05-16T14:00:00Z",
      "updated_at": "2026-05-16T14:00:00Z",
      "tags": ["auth", "critical"]
    }
  ]
}
```

**Benefits:**
- ✅ Human-readable metadata (names, timestamps)
- ✅ Portable between agents
- ✅ Version controlled (can track format changes)
- ✅ Supports migration/import/export

### 4. CLI Commands

```bash
# Set/Update secrets (auto-syncs to all layers)
shipwright secrets set KEY VALUE
shipwright secrets set KEY VALUE --tag production
shipwright secrets set-from-file .env.production  # Bulk import

# View secrets
shipwright secrets list                    # Show names only
shipwright secrets list --with-metadata    # Show timestamps, tags
shipwright secrets get KEY                 # Show decrypted value

# Backup/Restore
shipwright secrets backup                  # Manual backup to local machine
shipwright secrets restore                 # Restore from local backup
shipwright secrets export > secrets.env    # Export as .env format
shipwright secrets import secrets.env      # Import from .env format

# Migration
shipwright secrets export --encrypted PROJECT.enc   # Export encrypted file
shipwright secrets import --encrypted PROJECT.enc   # Import from another agent

# Management
shipwright secrets delete KEY
shipwright secrets rotate KEY              # Change encryption key
shipwright secrets audit                   # Show change history
```

### 5. Database Schema

```sql
-- Agent database
CREATE TABLE secret_stores (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id)
);

CREATE TABLE secrets (
    id TEXT PRIMARY KEY,
    store_id TEXT NOT NULL,
    name TEXT NOT NULL,
    value_encrypted BLOB NOT NULL,
    nonce BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    tags TEXT, -- JSON array
    FOREIGN KEY(store_id) REFERENCES secret_stores(id),
    UNIQUE(store_id, name)
);

CREATE TABLE secret_audit_log (
    id TEXT PRIMARY KEY,
    secret_id TEXT NOT NULL,
    action TEXT NOT NULL, -- 'created', 'updated', 'deleted', 'accessed'
    performed_by TEXT, -- 'cli', 'agent', 'webhook'
    timestamp INTEGER NOT NULL,
    FOREIGN KEY(secret_id) REFERENCES secrets(id)
);

CREATE TABLE backup_metadata (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    backup_path TEXT NOT NULL,
    checksum TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id)
);
```

### 6. Backup Automation

**Automatic Backup Triggers:**

1. **On Secret Change**:
   ```
   User sets secret → Agent DB updated → Immediate backup to:
     - /var/lib/shipwright/backups/PROJECT.enc
     - User's ~/.shipwright/secrets/PROJECT.enc (synced via CLI)
   ```

2. **Scheduled Backups**:
   ```
   Daily cron: 2 AM UTC
     - Backup all secret stores
     - Verify checksums
     - Clean old backups (keep last 7 days)
   ```

3. **On Deployment**:
   ```
   Before deploy → Verify secrets exist → Backup current state
   ```

### 7. Recovery Procedures

**Scenario 1: Agent DB Corrupted**
```bash
# Automatic recovery (agent does this on startup)
1. Detect corruption
2. Restore from /var/lib/shipwright/backups/
3. Verify integrity
4. Resume operation
```

**Scenario 2: VPS Rebuilt**
```bash
# Manual recovery from local machine
shipwright secrets push --all          # Push all local secrets to new agent
# OR
shipwright secrets restore PROJECT    # Restore specific project
```

**Scenario 3: Migration to New Server**
```bash
# Old server
shipwright secrets export --encrypted all-secrets.enc

# New server
shipwright secrets import --encrypted all-secrets.enc
```

### 8. Security Considerations

**Encryption at Rest:**
- Agent DB encrypted with installation-specific key
- Backup files encrypted with project-specific key
- Keys derived from installation ID + project name
- Optional user passphrase for additional security

**Encryption in Transit:**
- HTTPS for CLI ↔ Agent communication
- Certificate pinning (optional)
- API authentication tokens

**Access Control:**
- Agent endpoints require authentication
- Rate limiting on secret endpoints
- Audit logging of all access

**Key Rotation:**
```bash
# Rotate encryption key
shipwright secrets rotate-key PROJECT
# Re-encrypts all secrets with new key
# Updates backups
# Invalidates old backups
```

### 9. Decoupling Strategy

**Standard Protocol:**
```
Secret Management Protocol v1 (SMP/v1)

HTTP API:
  POST   /api/v1/secrets/:project        # Set secret
  GET    /api/v1/secrets/:project        # List secrets
  GET    /api/v1/secrets/:project/:key   # Get secret
  DELETE /api/v1/secrets/:project/:key   # Delete secret
  POST   /api/v1/secrets/:project/import # Import from file
  GET    /api/v1/secrets/:project/export # Export to file

File Format:
  - Standard JSON format (see above)
  - Versioned for compatibility
  - Self-describing metadata
```

**Benefits:**
- Any SMP/v1 compatible agent can read the secrets
- Easy migration between Shipwright versions
- Could support other deployment tools
- Projects aren't locked to specific agent

### 10. Migration Path

**From Current (Band-aid) to Secret Store:**

```bash
# Step 1: Extract secrets from .shipwright.yml
shipwright migrate-secrets

# Prompts:
# Found 5 secrets in .shipwright.yml:
#   - GITHUB_REPOSITORY_OWNER
#   - JWT_SECRET
#   - DATABASE_URL
#   - REDIS_URL
#   - API_KEY
#
# Move to secret store? [Y/n] y
# Creating secret store...
# Uploading secrets to agent...
# Updating .shipwright.yml...
# Done! Secrets now managed by agent.

# Step 2: .shipwright.yml updated to:
version: 1
project:
  name: TESE-MARKET---BFF-ARCHITECTURE
build:
  compose_file: docker-compose.vps.yml
  # Secrets removed, now referenced by agent
```

### 11. Implementation Phases

**Phase 1: Core Infrastructure** (Week 1)
- [ ] Database schema migration
- [ ] Encryption utilities (AES-256-GCM)
- [ ] Agent API endpoints
- [ ] CLI commands (set, get, list, delete)

**Phase 2: Backup System** (Week 2)
- [ ] Local backup on VPS
- [ ] Remote sync to user machine
- [ ] Auto-backup on changes
- [ ] Integrity verification

**Phase 3: Recovery & Migration** (Week 3)
- [ ] Import/export encrypted files
- [ ] Agent startup recovery
- [ ] Migration tool from .shipwright.yml
- [ ] Documentation

**Phase 4: Advanced Features** (Week 4)
- [ ] Audit logging
- [ ] Key rotation
- [ ] Git backup integration (optional)
- [ ] Secret templates/presets

## Comparison: Final Solution vs Band-aid

| Aspect | Band-aid Fix | Secret Store |
|--------|--------------|--------------|
| Secrets in git? | ✅ Yes (config) | ❌ No |
| Survives git pull? | ✅ Yes | ✅ Yes |
| Survives VPS rebuild? | ❌ No | ✅ Yes (local backup) |
| Encryption? | ❌ No | ✅ Yes |
| Rotation? | ❌ Manual | ✅ Automated |
| Audit trail? | ❌ No | ✅ Yes |
| Migration? | ❌ Hard | ✅ Easy |
| Coupling? | ✅ Low | ⚠️  Medium (mitigated) |
| Complexity? | ✅ Low | ❌ High |

## Recommendation

**Start with band-aid fix NOW**, then migrate to secret store in phases:

1. **Immediate** (Today): Deploy band-aid fix to unblock deployments
2. **Phase 1** (Next week): Implement core secret store
3. **Phase 2-4** (Following weeks): Add backup, recovery, advanced features
4. **Migration**: Smooth transition with `shipwright migrate-secrets` command

This gives you:
- ✅ Immediate solution to current problem
- ✅ Path to proper architecture
- ✅ No rushed implementation bugs
- ✅ Time to test thoroughly

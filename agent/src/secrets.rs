use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

use crate::crypto::{decrypt_secret, derive_master_key, encrypt_secret, EncryptedValue, NONCE_SIZE};

/// Secret metadata and encrypted value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    pub id: String,
    pub store_id: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub value_encrypted: Vec<u8>,
    #[serde(skip_serializing)]
    pub nonce: Vec<u8>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Secret with decrypted value (for API responses)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretWithValue {
    pub name: String,
    pub value: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Secret metadata without value (for list operations)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretMetadata {
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Secret store for a project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretStore {
    pub id: String,
    pub project_id: String,
    pub agent_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Audit log entry
#[derive(Debug, Clone)]
pub enum AuditAction {
    Created,
    Updated,
    Deleted,
    Accessed,
}

impl AuditAction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Deleted => "deleted",
            Self::Accessed => "accessed",
        }
    }
}

/// Secret storage manager
pub struct SecretStorage {
    db: Arc<Mutex<Connection>>,
    agent_id: String,
}

impl SecretStorage {
    pub fn new(db: Arc<Mutex<Connection>>, agent_id: String) -> Self {
        Self { db, agent_id }
    }

    /// Get or create a secret store for a project
    pub fn get_or_create_store(&self, project_id: &str) -> Result<SecretStore> {
        let db = self.db.lock().unwrap();

        // Try to get existing store
        let mut stmt = db.prepare(
            "SELECT id, project_id, agent_id, created_at, updated_at
             FROM secret_stores WHERE project_id = ?"
        )?;

        let result = stmt.query_row(params![project_id], |row| {
            Ok(SecretStore {
                id: row.get(0)?,
                project_id: row.get(1)?,
                agent_id: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        });

        match result {
            Ok(store) => Ok(store),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Create new store
                let store_id = uuid::Uuid::new_v4().to_string();
                let now = chrono::Utc::now().timestamp();

                db.execute(
                    "INSERT INTO secret_stores (id, project_id, agent_id, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?)",
                    params![store_id, project_id, &self.agent_id, now, now],
                )?;

                info!("🔐 Created secret store for project: {}", project_id);

                Ok(SecretStore {
                    id: store_id,
                    project_id: project_id.to_string(),
                    agent_id: self.agent_id.clone(),
                    created_at: now,
                    updated_at: now,
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Set a secret value (create or update)
    pub fn set_secret(
        &self,
        project_id: &str,
        project_name: &str,
        name: &str,
        value: &str,
        tags: Option<Vec<String>>,
        performed_by: &str,
    ) -> Result<()> {
        let store = self.get_or_create_store(project_id)?;

        // Derive master key for this project
        let master_key = derive_master_key(project_name, &self.agent_id, None)?;

        // Encrypt the secret
        let encrypted = encrypt_secret(value, &master_key)?;

        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().timestamp();

        // Check if secret exists
        let exists: bool = db
            .query_row(
                "SELECT 1 FROM secrets WHERE store_id = ? AND name = ?",
                params![&store.id, name],
                |_| Ok(true),
            )
            .unwrap_or(false);

        let tags_json = tags.as_ref().map(|t| serde_json::to_string(t).unwrap());

        if exists {
            // Update existing secret
            db.execute(
                "UPDATE secrets
                 SET value_encrypted = ?, nonce = ?, updated_at = ?, tags = ?
                 WHERE store_id = ? AND name = ?",
                params![
                    &encrypted.ciphertext,
                    &encrypted.nonce,
                    now,
                    tags_json,
                    &store.id,
                    name
                ],
            )?;

            // Get secret ID for audit log
            let secret_id: String = db.query_row(
                "SELECT id FROM secrets WHERE store_id = ? AND name = ?",
                params![&store.id, name],
                |row| row.get(0),
            )?;

            self.log_audit(&db, Some(&secret_id), &store.id, Some(name), AuditAction::Updated, performed_by)?;
            info!("🔐 Updated secret '{}' for project '{}'", name, project_name);
        } else {
            // Create new secret
            let secret_id = uuid::Uuid::new_v4().to_string();

            db.execute(
                "INSERT INTO secrets (id, store_id, name, value_encrypted, nonce, created_at, updated_at, tags)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    &secret_id,
                    &store.id,
                    name,
                    &encrypted.ciphertext,
                    &encrypted.nonce,
                    now,
                    now,
                    tags_json
                ],
            )?;

            self.log_audit(&db, Some(&secret_id), &store.id, Some(name), AuditAction::Created, performed_by)?;
            info!("🔐 Created secret '{}' for project '{}'", name, project_name);
        }

        // Update store's updated_at timestamp
        db.execute(
            "UPDATE secret_stores SET updated_at = ? WHERE id = ?",
            params![now, &store.id],
        )?;

        Ok(())
    }

    /// Get a secret value (decrypted)
    pub fn get_secret(
        &self,
        project_id: &str,
        project_name: &str,
        name: &str,
        performed_by: &str,
    ) -> Result<SecretWithValue> {
        let store = self.get_or_create_store(project_id)?;

        // Derive master key
        let master_key = derive_master_key(project_name, &self.agent_id, None)?;

        let db = self.db.lock().unwrap();

        let secret: Secret = db.query_row(
            "SELECT id, store_id, name, value_encrypted, nonce, created_at, updated_at, tags
             FROM secrets WHERE store_id = ? AND name = ?",
            params![&store.id, name],
            |row| {
                let tags_json: Option<String> = row.get(7)?;
                let tags = tags_json.and_then(|s| serde_json::from_str(&s).ok());

                Ok(Secret {
                    id: row.get(0)?,
                    store_id: row.get(1)?,
                    name: row.get(2)?,
                    value_encrypted: row.get(3)?,
                    nonce: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    tags,
                })
            },
        ).context(format!("Secret '{}' not found", name))?;

        // Decrypt the value
        let encrypted = EncryptedValue {
            ciphertext: secret.value_encrypted.clone(),
            nonce: secret.nonce.clone(),
        };

        let value = decrypt_secret(&encrypted, &master_key)?;

        // Log access
        self.log_audit(&db, Some(&secret.id), &store.id, Some(name), AuditAction::Accessed, performed_by)?;

        Ok(SecretWithValue {
            name: secret.name,
            value,
            created_at: secret.created_at,
            updated_at: secret.updated_at,
            tags: secret.tags,
        })
    }

    /// List all secrets (metadata only, no values)
    pub fn list_secrets(&self, project_id: &str) -> Result<Vec<SecretMetadata>> {
        let store = self.get_or_create_store(project_id)?;

        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT name, created_at, updated_at, tags
             FROM secrets WHERE store_id = ? ORDER BY name"
        )?;

        let secrets = stmt.query_map(params![&store.id], |row| {
            let tags_json: Option<String> = row.get(3)?;
            let tags = tags_json.and_then(|s| serde_json::from_str(&s).ok());

            Ok(SecretMetadata {
                name: row.get(0)?,
                created_at: row.get(1)?,
                updated_at: row.get(2)?,
                tags,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(secrets)
    }

    /// Delete a secret
    pub fn delete_secret(
        &self,
        project_id: &str,
        name: &str,
        performed_by: &str,
    ) -> Result<()> {
        let store = self.get_or_create_store(project_id)?;

        let db = self.db.lock().unwrap();

        // Get secret ID before deletion for audit log
        let secret_id: Option<String> = db.query_row(
            "SELECT id FROM secrets WHERE store_id = ? AND name = ?",
            params![&store.id, name],
            |row| row.get(0),
        ).ok();

        if secret_id.is_none() {
            anyhow::bail!("Secret '{}' not found", name);
        }

        // Delete the secret
        let deleted = db.execute(
            "DELETE FROM secrets WHERE store_id = ? AND name = ?",
            params![&store.id, name],
        )?;

        if deleted == 0 {
            anyhow::bail!("Secret '{}' not found", name);
        }

        // Log deletion (secret_id will be set to NULL due to ON DELETE SET NULL)
        self.log_audit(&db, None, &store.id, Some(name), AuditAction::Deleted, performed_by)?;

        // Update store's updated_at timestamp
        let now = chrono::Utc::now().timestamp();
        db.execute(
            "UPDATE secret_stores SET updated_at = ? WHERE id = ?",
            params![now, &store.id],
        )?;

        info!("🔐 Deleted secret '{}' from project", name);

        Ok(())
    }

    /// Get all secrets for a project (decrypted)
    pub fn get_all_secrets(
        &self,
        project_id: &str,
        project_name: &str,
        performed_by: &str,
    ) -> Result<Vec<SecretWithValue>> {
        let store = self.get_or_create_store(project_id)?;

        // Derive master key
        let master_key = derive_master_key(project_name, &self.agent_id, None)?;

        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, name, value_encrypted, nonce, created_at, updated_at, tags
             FROM secrets WHERE store_id = ? ORDER BY name"
        )?;

        let secrets: Vec<SecretWithValue> = stmt.query_map(params![&store.id], |row| {
            let secret_id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let value_encrypted: Vec<u8> = row.get(2)?;
            let nonce: Vec<u8> = row.get(3)?;
            let created_at: i64 = row.get(4)?;
            let updated_at: i64 = row.get(5)?;
            let tags_json: Option<String> = row.get(6)?;
            let tags = tags_json.and_then(|s| serde_json::from_str(&s).ok());

            Ok((secret_id, name, value_encrypted, nonce, created_at, updated_at, tags))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|(secret_id, name, value_encrypted, nonce, created_at, updated_at, tags)| {
            let encrypted = EncryptedValue {
                ciphertext: value_encrypted,
                nonce,
            };

            match decrypt_secret(&encrypted, &master_key) {
                Ok(value) => {
                    // Log access for each secret
                    let _ = self.log_audit(&db, Some(&secret_id), &store.id, Some(&name), AuditAction::Accessed, performed_by);

                    Some(SecretWithValue {
                        name,
                        value,
                        created_at,
                        updated_at,
                        tags,
                    })
                }
                Err(e) => {
                    warn!("Failed to decrypt secret '{}': {}", name, e);
                    None
                }
            }
        })
        .collect();

        Ok(secrets)
    }

    /// Log an audit event
    fn log_audit(
        &self,
        db: &Connection,
        secret_id: Option<&str>,
        store_id: &str,
        secret_name: Option<&str>,
        action: AuditAction,
        performed_by: &str,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        db.execute(
            "INSERT INTO secret_audit_log (id, secret_id, store_id, action, secret_name, performed_by, timestamp)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![id, secret_id, store_id, action.as_str(), secret_name, performed_by, now],
        )?;

        Ok(())
    }

    /// Get audit log for a project
    pub fn get_audit_log(&self, project_id: &str, limit: Option<usize>) -> Result<Vec<serde_json::Value>> {
        let store = self.get_or_create_store(project_id)?;

        let db = self.db.lock().unwrap();

        let query = if let Some(limit) = limit {
            format!(
                "SELECT action, secret_name, performed_by, timestamp
                 FROM secret_audit_log
                 WHERE store_id = ?
                 ORDER BY timestamp DESC
                 LIMIT {}",
                limit
            )
        } else {
            "SELECT action, secret_name, performed_by, timestamp
             FROM secret_audit_log
             WHERE store_id = ?
             ORDER BY timestamp DESC"
                .to_string()
        };

        let mut stmt = db.prepare(&query)?;

        let logs = stmt.query_map(params![&store.id], |row| {
            let action: String = row.get(0)?;
            let secret_name: Option<String> = row.get(1)?;
            let performed_by: String = row.get(2)?;
            let timestamp: i64 = row.get(3)?;

            Ok(serde_json::json!({
                "action": action,
                "secret_name": secret_name,
                "performed_by": performed_by,
                "timestamp": timestamp,
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(logs)
    }
}

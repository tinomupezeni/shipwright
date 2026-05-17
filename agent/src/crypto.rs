use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;

/// Number of PBKDF2 iterations (100,000 is OWASP recommendation)
const PBKDF2_ITERATIONS: u32 = 100_000;

/// Salt size for key derivation (16 bytes)
const SALT_SIZE: usize = 16;

/// Nonce size for AES-GCM (12 bytes)
pub const NONCE_SIZE: usize = 12;

/// Derived key size for AES-256 (32 bytes)
const KEY_SIZE: usize = 32;

/// Encrypted value with its nonce
#[derive(Debug, Clone)]
pub struct EncryptedValue {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
}

/// Generate a cryptographically secure master key for a project
///
/// Key is derived from:
/// - Project name
/// - Agent installation ID
/// - Optional user passphrase
pub fn derive_master_key(
    project_name: &str,
    agent_id: &str,
    passphrase: Option<&str>,
) -> Result<Vec<u8>> {
    // Create password from project name + agent ID + optional passphrase
    let password = match passphrase {
        Some(pass) => format!("{}:{}:{}", project_name, agent_id, pass),
        None => format!("{}:{}", project_name, agent_id),
    };

    // Generate or derive salt from project name and agent ID
    // This ensures the same salt is used for the same project/agent combination
    let salt_material = format!("shipwright:{}:{}", project_name, agent_id);
    let mut salt = [0u8; SALT_SIZE];
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(salt_material.as_bytes());
    let hash = hasher.finalize();
    salt.copy_from_slice(&hash[..SALT_SIZE]);

    // Derive key using PBKDF2
    let mut key = [0u8; KEY_SIZE];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ITERATIONS, &mut key);

    Ok(key.to_vec())
}

/// Encrypt a secret value using AES-256-GCM
pub fn encrypt_secret(value: &str, master_key: &[u8]) -> Result<EncryptedValue> {
    if master_key.len() != KEY_SIZE {
        anyhow::bail!("Invalid key size: expected {}, got {}", KEY_SIZE, master_key.len());
    }

    // Create cipher from master key
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(master_key);
    let cipher = Aes256Gcm::new(key);

    // Generate random nonce
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encrypt the value
    let ciphertext = cipher
        .encrypt(nonce, value.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to encrypt secret: {:?}", e))?;

    Ok(EncryptedValue {
        ciphertext,
        nonce: nonce_bytes.to_vec(),
    })
}

/// Decrypt a secret value using AES-256-GCM
pub fn decrypt_secret(encrypted: &EncryptedValue, master_key: &[u8]) -> Result<String> {
    if master_key.len() != KEY_SIZE {
        anyhow::bail!("Invalid key size: expected {}, got {}", KEY_SIZE, master_key.len());
    }

    if encrypted.nonce.len() != NONCE_SIZE {
        anyhow::bail!("Invalid nonce size: expected {}, got {}", NONCE_SIZE, encrypted.nonce.len());
    }

    // Create cipher from master key
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(master_key);
    let cipher = Aes256Gcm::new(key);

    // Create nonce from stored bytes
    let nonce = Nonce::from_slice(&encrypted.nonce);

    // Decrypt the value
    let plaintext = cipher
        .decrypt(nonce, encrypted.ciphertext.as_ref())
        .map_err(|e| anyhow::anyhow!("Failed to decrypt secret (wrong key or corrupted data): {:?}", e))?;

    String::from_utf8(plaintext).context("Decrypted data is not valid UTF-8")
}

/// Generate a random agent installation ID
pub fn generate_agent_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_derivation() {
        let key1 = derive_master_key("my-project", "agent-123", None).unwrap();
        let key2 = derive_master_key("my-project", "agent-123", None).unwrap();
        let key3 = derive_master_key("my-project", "agent-456", None).unwrap();
        let key4 = derive_master_key("other-project", "agent-123", None).unwrap();

        // Same inputs should produce same key
        assert_eq!(key1, key2);

        // Different agent ID should produce different key
        assert_ne!(key1, key3);

        // Different project should produce different key
        assert_ne!(key1, key4);

        // Key should be 32 bytes
        assert_eq!(key1.len(), 32);
    }

    #[test]
    fn test_encryption_decryption() {
        let master_key = derive_master_key("test-project", "agent-123", None).unwrap();
        let secret_value = "my-super-secret-password";

        // Encrypt
        let encrypted = encrypt_secret(secret_value, &master_key).unwrap();

        // Decrypt
        let decrypted = decrypt_secret(&encrypted, &master_key).unwrap();

        assert_eq!(secret_value, decrypted);
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = derive_master_key("project1", "agent-123", None).unwrap();
        let key2 = derive_master_key("project2", "agent-123", None).unwrap();

        let encrypted = encrypt_secret("secret", &key1).unwrap();

        // Decrypting with wrong key should fail
        let result = decrypt_secret(&encrypted, &key2);
        assert!(result.is_err());
    }

    #[test]
    fn test_nonce_uniqueness() {
        let master_key = derive_master_key("test", "agent", None).unwrap();

        let enc1 = encrypt_secret("same value", &master_key).unwrap();
        let enc2 = encrypt_secret("same value", &master_key).unwrap();

        // Different nonces should be generated
        assert_ne!(enc1.nonce, enc2.nonce);

        // Both should decrypt correctly
        assert_eq!(decrypt_secret(&enc1, &master_key).unwrap(), "same value");
        assert_eq!(decrypt_secret(&enc2, &master_key).unwrap(), "same value");
    }

    #[test]
    fn test_passphrase_changes_key() {
        let key1 = derive_master_key("project", "agent", None).unwrap();
        let key2 = derive_master_key("project", "agent", Some("password")).unwrap();

        assert_ne!(key1, key2);
    }
}

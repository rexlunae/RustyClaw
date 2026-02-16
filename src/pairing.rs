//! DM pairing security system for messenger authorization.
//!
//! This module implements OpenClaw-compatible pairing codes and allowlist
//! to prevent unauthorized users from controlling the AI via messengers.
//!
//! ## Architecture
//!
//! - **Allowlist**: Persistent JSON file storing authorized senders
//! - **Pending codes**: In-memory HashMap with TTL for verification
//! - **Pairing flow**: Unknown sender → code generation → admin approval → verification
//!
//! ## Usage
//!
//! ```rust
//! let pairing = PairingManager::new("/path/to/allowlist.json")?;
//!
//! // Check if sender is authorized
//! if !pairing.is_authorized("telegram", "user123").await {
//!     let code = pairing.generate_code("telegram", "user123").await;
//!     // Show code to admin and send challenge to user
//! }
//!
//! // Admin approves via /pair command
//! pairing.approve_sender("telegram:user123", "ABCD1234", "John Doe").await?;
//! ```

use anyhow::{Context, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Default pairing code expiry (5 minutes)
const DEFAULT_CODE_EXPIRY_SECS: u64 = 300;

/// Pairing code length
const PAIRING_CODE_LENGTH: usize = 8;

/// Allowlist entry for an authorized sender
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowlistEntry {
    /// Human-readable name/identifier
    pub name: String,
    /// Unix timestamp when paired
    pub paired_at: u64,
    /// Optional notes
    #[serde(default)]
    pub notes: Option<String>,
}

/// Pending pairing code with expiry
#[derive(Debug, Clone)]
struct PendingCode {
    code: String,
    expires_at: SystemTime,
}

/// Pairing manager for DM security
pub struct PairingManager {
    /// Path to allowlist JSON file
    allowlist_path: PathBuf,
    /// Authorized senders: "messenger_type:sender_id" -> AllowlistEntry
    allowlist: Arc<RwLock<HashMap<String, AllowlistEntry>>>,
    /// Pending pairing codes: "messenger_type:sender_id" -> PendingCode
    pending: Arc<RwLock<HashMap<String, PendingCode>>>,
    /// Code expiry duration
    code_expiry: Duration,
}

impl PairingManager {
    /// Create a new pairing manager
    pub fn new<P: AsRef<Path>>(allowlist_path: P) -> Result<Self> {
        let allowlist_path = allowlist_path.as_ref().to_path_buf();

        // Load existing allowlist
        let allowlist = if allowlist_path.exists() {
            let content = std::fs::read_to_string(&allowlist_path)
                .with_context(|| format!("Failed to read allowlist from {:?}", allowlist_path))?;
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse allowlist from {:?}", allowlist_path))?
        } else {
            HashMap::new()
        };

        Ok(Self {
            allowlist_path,
            allowlist: Arc::new(RwLock::new(allowlist)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            code_expiry: Duration::from_secs(DEFAULT_CODE_EXPIRY_SECS),
        })
    }

    /// Check if a sender is authorized
    pub async fn is_authorized(&self, messenger_type: &str, sender_id: &str) -> bool {
        let key = format!("{}:{}", messenger_type, sender_id);
        self.allowlist.read().await.contains_key(&key)
    }

    /// Generate a new pairing code for an unknown sender
    pub async fn generate_code(&self, messenger_type: &str, sender_id: &str) -> String {
        let key = format!("{}:{}", messenger_type, sender_id);

        // Clean up expired codes first
        self.cleanup_expired_codes().await;

        // Check if there's already a pending code
        {
            let pending = self.pending.read().await;
            if let Some(entry) = pending.get(&key) {
                if entry.expires_at > SystemTime::now() {
                    // Return existing code if not expired
                    return entry.code.clone();
                }
            }
        }

        // Generate new code
        let code = generate_random_code(PAIRING_CODE_LENGTH);
        let expires_at = SystemTime::now() + self.code_expiry;

        let mut pending = self.pending.write().await;
        pending.insert(key, PendingCode { code: code.clone(), expires_at });

        code
    }

    /// Verify a pairing code submitted by a sender
    pub async fn verify_code(&self, messenger_type: &str, sender_id: &str, submitted_code: &str) -> bool {
        let key = format!("{}:{}", messenger_type, sender_id);

        let pending = self.pending.read().await;
        if let Some(entry) = pending.get(&key) {
            // Check if code matches and hasn't expired
            if entry.code == submitted_code && entry.expires_at > SystemTime::now() {
                return true;
            }
        }

        false
    }

    /// Approve a sender and add to allowlist (admin action)
    pub async fn approve_sender(&self, messenger_type: &str, sender_id: &str, name: String) -> Result<()> {
        let key = format!("{}:{}", messenger_type, sender_id);

        let paired_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let entry = AllowlistEntry {
            name,
            paired_at,
            notes: None,
        };

        // Add to allowlist
        {
            let mut allowlist = self.allowlist.write().await;
            allowlist.insert(key.clone(), entry);
        }

        // Remove from pending
        {
            let mut pending = self.pending.write().await;
            pending.remove(&key);
        }

        // Persist to disk
        self.save_allowlist().await?;

        Ok(())
    }

    /// Remove a sender from the allowlist (admin action)
    pub async fn revoke_sender(&self, messenger_type: &str, sender_id: &str) -> Result<bool> {
        let key = format!("{}:{}", messenger_type, sender_id);

        let removed = {
            let mut allowlist = self.allowlist.write().await;
            allowlist.remove(&key).is_some()
        };

        if removed {
            self.save_allowlist().await?;
        }

        Ok(removed)
    }

    /// List all authorized senders
    pub async fn list_authorized(&self) -> Vec<(String, AllowlistEntry)> {
        let allowlist = self.allowlist.read().await;
        allowlist.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// List all pending pairing codes (admin view)
    pub async fn list_pending(&self) -> Vec<(String, String, SystemTime)> {
        self.cleanup_expired_codes().await;

        let pending = self.pending.read().await;
        pending.iter()
            .map(|(k, v)| (k.clone(), v.code.clone(), v.expires_at))
            .collect()
    }

    /// Clean up expired pending codes
    async fn cleanup_expired_codes(&self) {
        let now = SystemTime::now();
        let mut pending = self.pending.write().await;
        pending.retain(|_, entry| entry.expires_at > now);
    }

    /// Save allowlist to disk
    async fn save_allowlist(&self) -> Result<()> {
        let allowlist = self.allowlist.read().await;
        let json = serde_json::to_string_pretty(&*allowlist)
            .context("Failed to serialize allowlist")?;

        // Ensure parent directory exists
        if let Some(parent) = self.allowlist_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {:?}", parent))?;
        }

        std::fs::write(&self.allowlist_path, json)
            .with_context(|| format!("Failed to write allowlist to {:?}", self.allowlist_path))?;

        Ok(())
    }

    /// Get pending code for a sender (if any)
    pub async fn get_pending_code(&self, messenger_type: &str, sender_id: &str) -> Option<String> {
        let key = format!("{}:{}", messenger_type, sender_id);
        let pending = self.pending.read().await;
        pending.get(&key).map(|e| e.code.clone())
    }
}

/// Generate a random alphanumeric pairing code
fn generate_random_code(length: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // Exclude ambiguous chars
    let mut rng = rand::thread_rng();

    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_pairing_flow() {
        let temp = NamedTempFile::new().unwrap();
        let manager = PairingManager::new(temp.path()).unwrap();

        // Unknown sender should not be authorized
        assert!(!manager.is_authorized("telegram", "user123").await);

        // Generate code
        let code = manager.generate_code("telegram", "user123").await;
        assert_eq!(code.len(), PAIRING_CODE_LENGTH);

        // Verify code works
        assert!(manager.verify_code("telegram", "user123", &code).await);

        // Wrong code should fail
        assert!(!manager.verify_code("telegram", "user123", "WRONGCODE").await);

        // Approve sender
        manager.approve_sender("telegram", "user123", "John Doe".to_string()).await.unwrap();

        // Now should be authorized
        assert!(manager.is_authorized("telegram", "user123").await);

        // Code should be removed from pending
        assert!(manager.get_pending_code("telegram", "user123").await.is_none());
    }

    #[tokio::test]
    async fn test_revoke_sender() {
        let temp = NamedTempFile::new().unwrap();
        let manager = PairingManager::new(temp.path()).unwrap();

        // Approve sender
        manager.approve_sender("discord", "user456", "Jane Doe".to_string()).await.unwrap();
        assert!(manager.is_authorized("discord", "user456").await);

        // Revoke
        let removed = manager.revoke_sender("discord", "user456").await.unwrap();
        assert!(removed);
        assert!(!manager.is_authorized("discord", "user456").await);
    }

    #[tokio::test]
    async fn test_persistence() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path().to_path_buf();

        {
            let manager = PairingManager::new(&path).unwrap();
            manager.approve_sender("telegram", "user789", "Test User".to_string()).await.unwrap();
        }

        // Create new manager instance - should load from disk
        let manager2 = PairingManager::new(&path).unwrap();
        assert!(manager2.is_authorized("telegram", "user789").await);
    }

    #[test]
    fn test_code_generation() {
        let code = generate_random_code(8);
        assert_eq!(code.len(), 8);
        assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(code.chars().all(|c| c.is_uppercase() || c.is_ascii_digit()));
    }
}

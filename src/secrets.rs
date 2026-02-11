use anyhow::{Context, Result};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Secrets manager with user-controlled access
pub struct SecretsManager {
    service_name: String,
    /// Cached secrets that have been approved for use
    cached_secrets: HashMap<String, String>,
    /// Whether the agent can access secrets without prompting
    agent_access_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    pub key: String,
    pub description: Option<String>,
}

impl SecretsManager {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            cached_secrets: HashMap::new(),
            agent_access_enabled: false,
        }
    }

    /// Enable or disable automatic agent access to secrets
    pub fn set_agent_access(&mut self, enabled: bool) {
        self.agent_access_enabled = enabled;
        if !enabled {
            self.cached_secrets.clear();
        }
    }

    /// Check if agent has access to secrets
    pub fn has_agent_access(&self) -> bool {
        self.agent_access_enabled
    }

    /// Store a secret in the system keyring
    pub fn store_secret(&mut self, key: &str, value: &str) -> Result<()> {
        let entry = Entry::new(&self.service_name, key)
            .context("Failed to create keyring entry")?;
        entry.set_password(value)
            .context("Failed to store secret in keyring")?;
        
        // If agent access is enabled, cache it
        if self.agent_access_enabled {
            self.cached_secrets.insert(key.to_string(), value.to_string());
        }
        
        Ok(())
    }

    /// Retrieve a secret from the system keyring
    /// This requires user approval if agent access is not enabled
    pub fn get_secret(&mut self, key: &str, user_approved: bool) -> Result<Option<String>> {
        // Check cache first
        if let Some(value) = self.cached_secrets.get(key) {
            return Ok(Some(value.clone()));
        }

        // If agent access is not enabled and user hasn't approved, deny
        if !self.agent_access_enabled && !user_approved {
            return Ok(None);
        }

        let entry = Entry::new(&self.service_name, key)
            .context("Failed to create keyring entry")?;
        
        match entry.get_password() {
            Ok(value) => {
                // Cache if agent access is enabled or user approved
                if self.agent_access_enabled || user_approved {
                    self.cached_secrets.insert(key.to_string(), value.clone());
                }
                Ok(Some(value))
            }
            Err(_) => Ok(None),
        }
    }

    /// Delete a secret from the system keyring
    pub fn delete_secret(&mut self, key: &str) -> Result<()> {
        let entry = Entry::new(&self.service_name, key)
            .context("Failed to create keyring entry")?;
        entry.delete_password()
            .context("Failed to delete secret from keyring")?;
        
        self.cached_secrets.remove(key);
        Ok(())
    }

    /// List all stored secret keys (not values)
    pub fn list_secrets(&self) -> Vec<String> {
        // Note: keyring doesn't provide a list function, so we maintain a separate metadata file
        // For now, return cached keys
        self.cached_secrets.keys().cloned().collect()
    }

    /// Clear the secret cache
    pub fn clear_cache(&mut self) {
        self.cached_secrets.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secrets_manager_creation() {
        let manager = SecretsManager::new("test_service");
        assert!(!manager.has_agent_access());
        assert_eq!(manager.list_secrets().len(), 0);
    }

    #[test]
    fn test_agent_access_control() {
        let mut manager = SecretsManager::new("test_service");
        assert!(!manager.has_agent_access());
        
        manager.set_agent_access(true);
        assert!(manager.has_agent_access());
        
        manager.set_agent_access(false);
        assert!(!manager.has_agent_access());
    }
}

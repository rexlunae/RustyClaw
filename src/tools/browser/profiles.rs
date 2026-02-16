//! Browser profile management
//!
//! Supports multiple isolated browser profiles, each with separate:
//! - Cookies and session storage
//! - Local storage
//! - Cache
//! - History
//! - Extensions (future)

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use std::fs;

/// Browser profile metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    /// Profile name (unique identifier)
    pub name: String,
    /// Profile display name
    pub display_name: String,
    /// Profile description
    pub description: Option<String>,
    /// Path to user data directory
    pub data_dir: PathBuf,
    /// Created timestamp
    pub created_at: i64,
    /// Last used timestamp
    pub last_used: Option<i64>,
    /// Whether this is the default profile
    pub is_default: bool,
}

/// Profile manager for handling multiple browser profiles
pub struct ProfileManager {
    base_dir: PathBuf,
}

impl ProfileManager {
    /// Create a new profile manager
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Get default profiles directory
    pub fn default_base_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rustyclaw")
            .join("browser-profiles")
    }

    /// List all profiles
    pub fn list_profiles(&self) -> Result<Vec<ProfileInfo>, String> {
        if !self.base_dir.exists() {
            return Ok(Vec::new());
        }

        let mut profiles = Vec::new();

        let entries = fs::read_dir(&self.base_dir)
            .map_err(|e| format!("Failed to read profiles directory: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            if path.is_dir() {
                let meta_file = path.join("profile.json");
                if meta_file.exists() {
                    let content = fs::read_to_string(&meta_file)
                        .map_err(|e| format!("Failed to read profile metadata: {}", e))?;
                    let profile: ProfileInfo = serde_json::from_str(&content)
                        .map_err(|e| format!("Failed to parse profile metadata: {}", e))?;
                    profiles.push(profile);
                }
            }
        }

        // Sort by last used (most recent first)
        profiles.sort_by(|a, b| {
            b.last_used
                .unwrap_or(0)
                .cmp(&a.last_used.unwrap_or(0))
        });

        Ok(profiles)
    }

    /// Get profile by name
    pub fn get_profile(&self, name: &str) -> Result<ProfileInfo, String> {
        let profile_dir = self.base_dir.join(name);
        let meta_file = profile_dir.join("profile.json");

        if !meta_file.exists() {
            return Err(format!("Profile not found: {}", name));
        }

        let content = fs::read_to_string(&meta_file)
            .map_err(|e| format!("Failed to read profile: {}", e))?;
        let profile: ProfileInfo = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse profile: {}", e))?;

        Ok(profile)
    }

    /// Create a new profile
    pub fn create_profile(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
    ) -> Result<ProfileInfo, String> {
        // Validate name
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err("Invalid profile name".to_string());
        }

        let profile_dir = self.base_dir.join(name);

        if profile_dir.exists() {
            return Err(format!("Profile already exists: {}", name));
        }

        // Create profile directory
        fs::create_dir_all(&profile_dir)
            .map_err(|e| format!("Failed to create profile directory: {}", e))?;

        // Create profile metadata
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let profile = ProfileInfo {
            name: name.to_string(),
            display_name: display_name.unwrap_or(name).to_string(),
            description: description.map(String::from),
            data_dir: profile_dir.clone(),
            created_at: now,
            last_used: None,
            is_default: false,
        };

        // Save metadata
        self.save_profile(&profile)?;

        Ok(profile)
    }

    /// Delete a profile
    pub fn delete_profile(&self, name: &str) -> Result<(), String> {
        let profile_dir = self.base_dir.join(name);

        if !profile_dir.exists() {
            return Err(format!("Profile not found: {}", name));
        }

        fs::remove_dir_all(&profile_dir)
            .map_err(|e| format!("Failed to delete profile: {}", e))?;

        Ok(())
    }

    /// Update profile last used timestamp
    pub fn update_last_used(&self, name: &str) -> Result<(), String> {
        let mut profile = self.get_profile(name)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        profile.last_used = Some(now);
        self.save_profile(&profile)?;

        Ok(())
    }

    /// Set default profile
    pub fn set_default(&self, name: &str) -> Result<(), String> {
        // Clear existing default
        for mut profile in self.list_profiles()? {
            if profile.is_default {
                profile.is_default = false;
                self.save_profile(&profile)?;
            }
        }

        // Set new default
        let mut profile = self.get_profile(name)?;
        profile.is_default = true;
        self.save_profile(&profile)?;

        Ok(())
    }

    /// Get default profile name
    pub fn get_default_profile_name(&self) -> Option<String> {
        self.list_profiles()
            .ok()?
            .into_iter()
            .find(|p| p.is_default)
            .map(|p| p.name)
    }

    /// Save profile metadata
    fn save_profile(&self, profile: &ProfileInfo) -> Result<(), String> {
        let meta_file = profile.data_dir.join("profile.json");
        let content = serde_json::to_string_pretty(profile)
            .map_err(|e| format!("Failed to serialize profile: {}", e))?;
        fs::write(&meta_file, content)
            .map_err(|e| format!("Failed to write profile: {}", e))?;
        Ok(())
    }

    /// Ensure default profile exists
    pub fn ensure_default_profile(&self) -> Result<ProfileInfo, String> {
        // Check if default profile exists
        if let Some(name) = self.get_default_profile_name() {
            return self.get_profile(&name);
        }

        // Check if "rustyclaw" profile exists
        if let Ok(profile) = self.get_profile("rustyclaw") {
            self.set_default("rustyclaw")?;
            return Ok(profile);
        }

        // Create default profile
        let mut profile = self.create_profile(
            "rustyclaw",
            Some("RustyClaw Default"),
            Some("Default browser profile for RustyClaw automation"),
        )?;
        profile.is_default = true;
        self.save_profile(&profile)?;

        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_list_profiles() {
        let dir = tempdir().unwrap();
        let manager = ProfileManager::new(dir.path().to_path_buf());

        // Create profile
        let profile = manager
            .create_profile("test", Some("Test Profile"), Some("Testing"))
            .unwrap();
        assert_eq!(profile.name, "test");
        assert_eq!(profile.display_name, "Test Profile");

        // List profiles
        let profiles = manager.list_profiles().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "test");
    }

    #[test]
    fn test_default_profile() {
        let dir = tempdir().unwrap();
        let manager = ProfileManager::new(dir.path().to_path_buf());

        // Ensure default
        let profile = manager.ensure_default_profile().unwrap();
        assert_eq!(profile.name, "rustyclaw");
        assert!(profile.is_default);

        // Get default
        let default_name = manager.get_default_profile_name().unwrap();
        assert_eq!(default_name, "rustyclaw");
    }

    #[test]
    fn test_delete_profile() {
        let dir = tempdir().unwrap();
        let manager = ProfileManager::new(dir.path().to_path_buf());

        // Create and delete
        manager.create_profile("temp", None, None).unwrap();
        assert!(manager.get_profile("temp").is_ok());

        manager.delete_profile("temp").unwrap();
        assert!(manager.get_profile("temp").is_err());
    }
}

//! Filesystem-backed swarm records.
//!
//! One TOML file per swarm at `<settings_dir>/swarms/<name>.toml`. Like the
//! agent registry, there is deliberately no in-memory cache: the gateway,
//! CLI, and model tools may run on different threads or in different
//! processes and must always agree on which swarms exist.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::config::SwarmConfig;
use crate::agents::is_valid_agent_id;

/// Binding from a swarm member to the registry agent that embodies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwarmMemberBinding {
    /// Member id within the swarm (matches `SwarmAgent::id`).
    pub member_id: String,
    /// Registry agent id the member was materialized as.
    pub agent_id: String,
}

/// A swarm as persisted on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmRecord {
    /// Unix timestamp (seconds) when the swarm was created.
    #[serde(default)]
    pub created_at: Option<u64>,
    /// Member → registry-agent bindings, in roster order.
    pub members: Vec<SwarmMemberBinding>,
    /// The configuration the swarm was created from.
    pub config: SwarmConfig,
}

impl SwarmRecord {
    /// The registry agent id for a member, if the member exists.
    pub fn agent_id_for(&self, member_id: &str) -> Option<&str> {
        self.members
            .iter()
            .find(|b| b.member_id == member_id)
            .map(|b| b.agent_id.as_str())
    }

    /// Age in seconds since creation (0 when unknown).
    pub fn age_secs(&self) -> u64 {
        let Some(created) = self.created_at else {
            return 0;
        };
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().saturating_sub(created))
            .unwrap_or(0)
    }
}

/// Store rooted at `<settings_dir>/swarms`.
#[derive(Debug, Clone)]
pub struct SwarmStore {
    swarms_root: PathBuf,
}

impl SwarmStore {
    /// Create a store from the installation's settings dir.
    pub fn new(settings_dir: &Path) -> Self {
        Self {
            swarms_root: settings_dir.join("swarms"),
        }
    }

    /// Root directory holding one `<name>.toml` per swarm.
    pub fn swarms_root(&self) -> &Path {
        &self.swarms_root
    }

    fn record_path(&self, name: &str) -> PathBuf {
        self.swarms_root.join(format!("{name}.toml"))
    }

    /// Whether a swarm with this name exists. Names that could escape the
    /// store directory (path separators, `..`, uppercase, …) never exist.
    pub fn exists(&self, name: &str) -> bool {
        is_valid_agent_id(name) && self.record_path(name).exists()
    }

    /// Load a swarm record by name.
    pub fn get(&self, name: &str) -> Option<SwarmRecord> {
        if !is_valid_agent_id(name) {
            return None;
        }
        let content = std::fs::read_to_string(self.record_path(name)).ok()?;
        toml::from_str(&content).ok()
    }

    /// List all swarms, sorted by name.
    pub fn list(&self) -> Vec<SwarmRecord> {
        let mut names: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.swarms_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if is_valid_agent_id(stem) {
                        names.push(stem.to_string());
                    }
                }
            }
        }
        names.sort();
        names.iter().filter_map(|n| self.get(n)).collect()
    }

    /// Persist a swarm record. The record's `config.name` is the key; it
    /// must be a valid id (same character rules as agent ids, since it
    /// prefixes every member's agent id).
    pub fn save(&self, record: &SwarmRecord) -> Result<()> {
        let name = record.config.name.as_str();
        if !is_valid_agent_id(name) {
            bail!(
                "Invalid swarm name '{}': use lowercase letters, digits, '-' or '_'",
                name
            );
        }
        crate::persist::write_atomically(
            &self.record_path(name),
            toml::to_string_pretty(record)?.as_bytes(),
        )?;
        Ok(())
    }

    /// Delete a swarm record.
    pub fn delete(&self, name: &str) -> Result<()> {
        if !is_valid_agent_id(name) {
            bail!("Invalid swarm name '{}'", name);
        }
        if !self.exists(name) {
            bail!("Swarm '{}' not found", name);
        }
        std::fs::remove_file(self.record_path(name))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::builtin_templates;
    use tempfile::tempdir;

    fn record(name: &str) -> SwarmRecord {
        let mut config = builtin_templates()[0].clone();
        config.name = name.to_string();
        SwarmRecord {
            created_at: Some(1),
            members: config
                .agents
                .iter()
                .map(|a| SwarmMemberBinding {
                    member_id: a.id.clone(),
                    agent_id: format!("{name}-{}", a.id),
                })
                .collect(),
            config,
        }
    }

    #[test]
    fn save_get_list_delete_roundtrip() {
        let tmp = tempdir().unwrap();
        let store = SwarmStore::new(tmp.path());

        assert!(store.list().is_empty());
        store.save(&record("alpha")).unwrap();
        store.save(&record("beta")).unwrap();

        let listed = store.list();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].config.name, "alpha");

        let got = store.get("beta").unwrap();
        assert_eq!(got.members.len(), got.config.agents.len());
        assert_eq!(got.agent_id_for("orchestrator"), Some("beta-orchestrator"));

        store.delete("alpha").unwrap();
        assert!(!store.exists("alpha"));
        assert!(store.exists("beta"));
        assert!(store.delete("alpha").is_err());
    }

    #[test]
    fn invalid_names_are_rejected_everywhere() {
        let tmp = tempdir().unwrap();
        let store = SwarmStore::new(tmp.path());

        let mut bad = record("ok");
        bad.config.name = "../escape".into();
        assert!(store.save(&bad).is_err());

        for name in ["../x", "..", "a/b", "A", "", ".hidden"] {
            assert!(!store.exists(name), "{name:?} must not exist");
            assert!(store.get(name).is_none());
            assert!(store.delete(name).is_err());
        }
    }
}

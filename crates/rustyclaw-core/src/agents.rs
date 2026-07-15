//! Agent registry: multiple named agents in a single installation.
//!
//! Each agent is a directory under `<settings_dir>/agents/<id>/` holding an
//! `agent.toml` manifest plus that agent's private state (sessions, threads,
//! projects, workspace). The `main` agent is special: it always exists, its
//! display name comes from `Config::agent_name`, and its workspace is the
//! installation-wide `<settings_dir>/workspace` for backwards compatibility.
//!
//! The registry is deliberately filesystem-backed with no in-memory cache so
//! the gateway, CLI, and model tools (which may run on different threads or
//! in different processes) always agree on the agent list.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The agent id that always exists and maps onto the legacy single-agent
/// directory layout.
pub const MAIN_AGENT_ID: &str = "main";

/// Manifest persisted as `agent.toml` inside an agent's directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Display name shown in UIs (defaults to the id).
    pub name: String,
    /// Optional human/model-readable description of the agent's purpose.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional base system prompt override for this agent.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Unix timestamp (seconds) when the agent was created.
    #[serde(default)]
    pub created_at: Option<u64>,
    /// Id of the agent that created this one, when spawned by another agent.
    #[serde(default)]
    pub created_by: Option<String>,
}

/// Summary of a registered agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// Filesystem-backed agent registry rooted at `<settings_dir>/agents`.
#[derive(Debug, Clone)]
pub struct AgentRegistry {
    agents_root: PathBuf,
    /// Display name for the implicit `main` agent (from `Config::agent_name`).
    main_name: String,
}

impl AgentRegistry {
    /// Create a registry from the config's settings dir and agent name.
    pub fn new(settings_dir: &Path, main_name: &str) -> Self {
        Self {
            agents_root: settings_dir.join("agents"),
            main_name: main_name.to_string(),
        }
    }

    /// Root directory that holds one subdirectory per agent.
    pub fn agents_root(&self) -> &Path {
        &self.agents_root
    }

    /// Directory for a specific agent id.
    pub fn agent_dir(&self, id: &str) -> PathBuf {
        self.agents_root.join(id)
    }

    fn manifest_path(&self, id: &str) -> PathBuf {
        self.agent_dir(id).join("agent.toml")
    }

    /// Whether an agent with this id exists (`main` always does).
    /// Invalid ids (path separators, `..`, uppercase, …) never exist.
    pub fn exists(&self, id: &str) -> bool {
        id == MAIN_AGENT_ID || (is_valid_agent_id(id) && self.manifest_path(id).exists())
    }

    /// Load an agent's manifest. `main` gets a synthetic manifest when none
    /// has been written yet. Invalid ids yield `None`.
    pub fn manifest(&self, id: &str) -> Option<AgentManifest> {
        if id != MAIN_AGENT_ID && !is_valid_agent_id(id) {
            return None;
        }
        let path = self.manifest_path(id);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(m) = toml::from_str::<AgentManifest>(&content) {
                return Some(m);
            }
        }
        (id == MAIN_AGENT_ID).then(|| AgentManifest {
            name: self.main_name.clone(),
            ..AgentManifest::default()
        })
    }

    /// Summary info for one agent, if it exists.
    pub fn get(&self, id: &str) -> Option<AgentInfo> {
        self.manifest(id).map(|m| AgentInfo {
            id: id.to_string(),
            name: if id == MAIN_AGENT_ID {
                // Config::agent_name is authoritative for main's display name.
                self.main_name.clone()
            } else if m.name.is_empty() {
                id.to_string()
            } else {
                m.name
            },
            description: m.description,
        })
    }

    /// List all agents, `main` first, the rest sorted by id.
    pub fn list(&self) -> Vec<AgentInfo> {
        let mut agents = vec![self.get(MAIN_AGENT_ID).unwrap_or(AgentInfo {
            id: MAIN_AGENT_ID.to_string(),
            name: self.main_name.clone(),
            description: None,
        })];

        let mut others: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.agents_root) {
            for entry in entries.flatten() {
                if !entry.path().is_dir() {
                    continue;
                }
                let Some(id) = entry.file_name().to_str().map(String::from) else {
                    continue;
                };
                if id == MAIN_AGENT_ID || id.starts_with('.') {
                    continue;
                }
                // Only manifest-bearing directories count as agents; this
                // skips stray session dirs and half-created agents.
                if self.manifest_path(&id).exists() {
                    others.push(id);
                }
            }
        }
        others.sort();
        agents.extend(others.iter().filter_map(|id| self.get(id)));
        agents
    }

    /// Create a new agent. When `id` is `None` it is derived from the name.
    /// Returns the created agent's info.
    pub fn create(
        &self,
        id: Option<&str>,
        name: &str,
        description: Option<&str>,
        system_prompt: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<AgentInfo> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Agent name must not be empty");
        }
        let id = match id {
            Some(explicit) => {
                let sanitized = sanitize_agent_id(explicit);
                if sanitized.is_empty() || sanitized != explicit {
                    bail!(
                        "Invalid agent id '{}': use lowercase letters, digits, '-' or '_'",
                        explicit
                    );
                }
                sanitized
            }
            None => {
                let derived = sanitize_agent_id(name);
                if derived.is_empty() {
                    bail!("Could not derive an agent id from name '{}'", name);
                }
                derived
            }
        };
        if self.exists(&id) {
            bail!("Agent '{}' already exists", id);
        }

        let dir = self.agent_dir(&id);
        std::fs::create_dir_all(dir.join("sessions"))?;
        std::fs::create_dir_all(dir.join("workspace"))?;

        let manifest = AgentManifest {
            name: name.to_string(),
            description: description.map(String::from),
            system_prompt: system_prompt.map(String::from),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs()),
            created_by: created_by.map(String::from),
        };
        std::fs::write(self.manifest_path(&id), toml::to_string_pretty(&manifest)?)?;

        Ok(AgentInfo {
            id,
            name: name.to_string(),
            description: manifest.description,
        })
    }

    /// Rename an agent (updates the manifest; `main` is renamed via
    /// `Config::agent_name`, not here).
    pub fn rename(&self, id: &str, new_name: &str) -> Result<()> {
        if id == MAIN_AGENT_ID {
            bail!("Rename the main agent via the agent-name setting");
        }
        let mut manifest = self
            .manifest(id)
            .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", id))?;
        manifest.name = new_name.trim().to_string();
        std::fs::write(self.manifest_path(id), toml::to_string_pretty(&manifest)?)?;
        Ok(())
    }

    /// Delete an agent and all of its state. `main` cannot be deleted.
    pub fn delete(&self, id: &str) -> Result<()> {
        if id == MAIN_AGENT_ID {
            bail!("The main agent cannot be deleted");
        }
        // Re-validate here (not just via `exists`) so the recursive delete
        // below can never operate on a path outside `agents_root`.
        if !is_valid_agent_id(id) {
            bail!("Invalid agent id '{}'", id);
        }
        if !self.exists(id) {
            bail!("Agent '{}' not found", id);
        }
        std::fs::remove_dir_all(self.agent_dir(id))?;
        Ok(())
    }
}

/// Whether a string is a well-formed agent id: non-empty, lowercase ASCII
/// alphanumerics plus `-`/`_` only. Everything the registry does with an id
/// ends up in a filesystem path under `agents_root`, so ids that could
/// escape it (`..`, `/`, `\`, leading dots) are rejected wholesale.
pub fn is_valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Reduce an arbitrary string to a filesystem/wire-safe agent id:
/// lowercase alphanumerics with `-`/`_`, spaces collapsed to `-`.
pub fn sanitize_agent_id(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = false;
    for c in input.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            last_dash = false;
        } else if (c == '-' || c.is_whitespace()) && !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn registry(dir: &Path) -> AgentRegistry {
        AgentRegistry::new(dir, "RustyClaw")
    }

    #[test]
    fn main_always_listed_first() {
        let tmp = tempdir().unwrap();
        let reg = registry(tmp.path());
        let agents = reg.list();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, MAIN_AGENT_ID);
        assert_eq!(agents[0].name, "RustyClaw");
        assert!(reg.exists(MAIN_AGENT_ID));
    }

    #[test]
    fn create_list_delete_roundtrip() {
        let tmp = tempdir().unwrap();
        let reg = registry(tmp.path());

        let info = reg
            .create(
                None,
                "Research Bot",
                Some("does research"),
                None,
                Some("main"),
            )
            .unwrap();
        assert_eq!(info.id, "research-bot");

        let agents = reg.list();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[1].id, "research-bot");
        assert_eq!(agents[1].name, "Research Bot");
        assert_eq!(agents[1].description.as_deref(), Some("does research"));

        let manifest = reg.manifest("research-bot").unwrap();
        assert_eq!(manifest.created_by.as_deref(), Some("main"));

        reg.delete("research-bot").unwrap();
        assert_eq!(reg.list().len(), 1);
    }

    #[test]
    fn duplicate_and_invalid_ids_rejected() {
        let tmp = tempdir().unwrap();
        let reg = registry(tmp.path());
        reg.create(Some("helper"), "Helper", None, None, None)
            .unwrap();
        assert!(
            reg.create(Some("helper"), "Helper 2", None, None, None)
                .is_err()
        );
        assert!(reg.create(Some("Bad Id!"), "x", None, None, None).is_err());
        assert!(
            reg.create(Some("main"), "Main again", None, None, None)
                .is_err()
        );
        assert!(reg.delete("main").is_err());
        assert!(reg.delete("nonexistent").is_err());
    }

    #[test]
    fn traversal_ids_rejected() {
        let tmp = tempdir().unwrap();
        // Plant a decoy manifest *outside* the agents root that a traversal
        // id would resolve to.
        let outside = tmp.path().join("victim");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("agent.toml"), "name = \"victim\"\n").unwrap();

        let reg = registry(&tmp.path().join("state"));
        for id in ["../../victim", "..", "a/../b", "a\\b", ".hidden", ""] {
            assert!(!reg.exists(id), "id {:?} must not exist", id);
            assert!(
                reg.manifest(id).is_none(),
                "id {:?} must have no manifest",
                id
            );
            assert!(reg.delete(id).is_err(), "id {:?} must not delete", id);
        }
        // The decoy survives every attempt.
        assert!(outside.join("agent.toml").exists());
    }

    #[test]
    fn sanitize_ids() {
        assert_eq!(sanitize_agent_id("Research Bot"), "research-bot");
        assert_eq!(sanitize_agent_id("  Weird--Name!! "), "weird-name");
        assert_eq!(sanitize_agent_id("under_score"), "under_score");
        assert_eq!(sanitize_agent_id("!!!"), "");
    }

    #[test]
    fn rename_updates_manifest() {
        let tmp = tempdir().unwrap();
        let reg = registry(tmp.path());
        reg.create(Some("helper"), "Helper", None, None, None)
            .unwrap();
        reg.rename("helper", "Better Helper").unwrap();
        assert_eq!(reg.get("helper").unwrap().name, "Better Helper");
        assert!(reg.rename("main", "x").is_err());
    }
}

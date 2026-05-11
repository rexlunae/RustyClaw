//! Swarm lifecycle management.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::config::{SwarmConfig, SwarmStatus};

/// A live swarm instance tracking runtime state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmInstance {
    /// The static configuration this instance was created from.
    pub config: SwarmConfig,
    /// Current lifecycle status.
    pub status: SwarmStatus,
    /// Epoch-millis when the swarm was created.
    pub created_ms: u64,
    /// Epoch-millis when the swarm was started (if ever).
    pub started_ms: Option<u64>,
    /// Epoch-millis when the swarm was stopped (if ever).
    pub stopped_ms: Option<u64>,
    /// Map of agent-id → session key for spawned sub-agent sessions.
    #[serde(default)]
    pub agent_sessions: HashMap<String, String>,
    /// Number of tasks routed through the orchestrator.
    pub tasks_routed: u64,
}

impl SwarmInstance {
    /// Create a new idle instance from a config.
    pub fn new(config: SwarmConfig) -> Self {
        Self {
            config,
            status: SwarmStatus::Idle,
            created_ms: now_ms(),
            started_ms: None,
            stopped_ms: None,
            agent_sessions: HashMap::new(),
            tasks_routed: 0,
        }
    }

    /// Mark the swarm as running.
    pub fn start(&mut self) {
        self.status = SwarmStatus::Running;
        self.started_ms = Some(now_ms());
    }

    /// Mark the swarm as stopped.
    pub fn stop(&mut self) {
        self.status = SwarmStatus::Stopped;
        self.stopped_ms = Some(now_ms());
        self.agent_sessions.clear();
    }

    /// Mark the swarm as paused.
    pub fn pause(&mut self) {
        self.status = SwarmStatus::Paused;
    }

    /// Resume a paused swarm.
    pub fn resume(&mut self) {
        if self.status == SwarmStatus::Paused {
            self.status = SwarmStatus::Running;
        }
    }

    /// Record that a task was routed.
    pub fn record_task(&mut self) {
        self.tasks_routed += 1;
    }

    /// Runtime in seconds since start (or 0 if never started).
    pub fn runtime_secs(&self) -> u64 {
        let start = match self.started_ms {
            Some(ms) => ms,
            None => return 0,
        };
        let end = self.stopped_ms.unwrap_or_else(now_ms);
        (end - start) / 1000
    }
}

/// Manages all swarm instances.
pub struct SwarmManager {
    swarms: HashMap<String, SwarmInstance>,
}

impl SwarmManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            swarms: HashMap::new(),
        }
    }

    /// Create a swarm from a config.  Returns an error if the name is taken.
    pub fn create(&mut self, config: SwarmConfig) -> Result<&SwarmInstance, String> {
        if self.swarms.contains_key(&config.name) {
            return Err(format!("Swarm '{}' already exists", config.name));
        }
        let name = config.name.clone();
        self.swarms.insert(name.clone(), SwarmInstance::new(config));
        Ok(self.swarms.get(&name).expect("just inserted"))
    }

    /// Start a swarm by name.
    pub fn start(&mut self, name: &str) -> Result<(), String> {
        let inst = self
            .swarms
            .get_mut(name)
            .ok_or_else(|| format!("Swarm '{}' not found", name))?;
        if inst.status == SwarmStatus::Running {
            return Err(format!("Swarm '{}' is already running", name));
        }
        inst.start();
        Ok(())
    }

    /// Stop a swarm by name.
    pub fn stop(&mut self, name: &str) -> Result<(), String> {
        let inst = self
            .swarms
            .get_mut(name)
            .ok_or_else(|| format!("Swarm '{}' not found", name))?;
        inst.stop();
        Ok(())
    }

    /// Get a swarm instance by name.
    pub fn get(&self, name: &str) -> Option<&SwarmInstance> {
        self.swarms.get(name)
    }

    /// Get a mutable swarm instance by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut SwarmInstance> {
        self.swarms.get_mut(name)
    }

    /// List all swarms.
    pub fn list(&self) -> Vec<&SwarmInstance> {
        let mut v: Vec<_> = self.swarms.values().collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.created_ms));
        v
    }

    /// Remove a stopped swarm.
    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        let inst = self
            .swarms
            .get(name)
            .ok_or_else(|| format!("Swarm '{}' not found", name))?;
        if inst.status == SwarmStatus::Running {
            return Err(format!(
                "Cannot remove running swarm '{}'; stop it first",
                name
            ));
        }
        self.swarms.remove(name);
        Ok(())
    }
}

impl Default for SwarmManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe handle to the swarm manager.
pub type SharedSwarmManager = Arc<Mutex<SwarmManager>>;

/// Global singleton.
static SWARM_MANAGER: OnceLock<SharedSwarmManager> = OnceLock::new();

/// Get the global swarm manager.
pub fn swarm_manager() -> &'static SharedSwarmManager {
    SWARM_MANAGER.get_or_init(|| Arc::new(Mutex::new(SwarmManager::new())))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::templates::builtin_templates;

    #[test]
    fn create_and_start_swarm() {
        let mut mgr = SwarmManager::new();
        let templates = builtin_templates();
        let cfg = templates[0].clone();
        let name = cfg.name.clone();
        mgr.create(cfg).unwrap();
        mgr.start(&name).unwrap();

        let inst = mgr.get(&name).unwrap();
        assert_eq!(inst.status, SwarmStatus::Running);
    }

    #[test]
    fn duplicate_name_errors() {
        let mut mgr = SwarmManager::new();
        let templates = builtin_templates();
        let cfg = templates[0].clone();
        mgr.create(cfg.clone()).unwrap();
        assert!(mgr.create(cfg).is_err());
    }

    #[test]
    fn list_returns_all() {
        let mut mgr = SwarmManager::new();
        let templates = builtin_templates();
        for t in &templates {
            let mut cfg = t.clone();
            cfg.name = format!("{}-test", cfg.name);
            mgr.create(cfg).unwrap();
        }
        assert_eq!(mgr.list().len(), templates.len());
    }

    #[test]
    fn stop_and_remove() {
        let mut mgr = SwarmManager::new();
        let templates = builtin_templates();
        let cfg = templates[0].clone();
        let name = cfg.name.clone();
        mgr.create(cfg).unwrap();
        mgr.start(&name).unwrap();
        mgr.stop(&name).unwrap();

        let inst = mgr.get(&name).unwrap();
        assert_eq!(inst.status, SwarmStatus::Stopped);

        mgr.remove(&name).unwrap();
        assert!(mgr.get(&name).is_none());
    }
}

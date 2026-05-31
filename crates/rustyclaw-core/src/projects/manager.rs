//! Project manager — owns the project registry and the active project.
//!
//! Unlike [`ThreadId`](crate::threads::ThreadId), which uses a process-global
//! counter, `ProjectId`s are minted from an instance `next_id` that is
//! persisted and reconciled on load, so ids never collide after a restart.

use super::model::{DEFAULT_PROJECT_ID, Project, ProjectId, ProjectInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::debug;

/// Manages all projects and which one is active.
pub struct ProjectManager {
    projects: HashMap<ProjectId, Project>,
    active_id: ProjectId,
    /// Next id to mint. Reconciled past the max loaded id on load.
    next_id: u64,
}

impl ProjectManager {
    /// Create an empty manager (no default project yet — seed it with
    /// [`ensure_default`](Self::ensure_default) once the workspace path is known).
    pub fn new() -> Self {
        Self {
            projects: HashMap::new(),
            active_id: DEFAULT_PROJECT_ID,
            next_id: DEFAULT_PROJECT_ID.0 + 1,
        }
    }

    /// Ensure the implicit "Default" project exists, pointing at `path`. Used
    /// as the migration target for pre-projects threads. Returns its id.
    pub fn ensure_default(&mut self, path: impl Into<PathBuf>) -> ProjectId {
        self.projects
            .entry(DEFAULT_PROJECT_ID)
            .or_insert_with(|| Project::new(DEFAULT_PROJECT_ID, "Default", path));
        DEFAULT_PROJECT_ID
    }

    /// Create a new project at `path`. Returns the new id.
    pub fn create(&mut self, name: impl Into<String>, path: impl Into<PathBuf>) -> ProjectId {
        let id = ProjectId(self.next_id);
        self.next_id += 1;
        self.projects.insert(id, Project::new(id, name, path));
        id
    }

    pub fn get(&self, id: ProjectId) -> Option<&Project> {
        self.projects.get(&id)
    }

    pub fn get_mut(&mut self, id: ProjectId) -> Option<&mut Project> {
        self.projects.get_mut(&id)
    }

    pub fn contains(&self, id: ProjectId) -> bool {
        self.projects.contains_key(&id)
    }

    pub fn active_id(&self) -> ProjectId {
        self.active_id
    }

    pub fn active(&self) -> Option<&Project> {
        self.projects.get(&self.active_id)
    }

    /// Working directory of a project, if it exists.
    pub fn path_of(&self, id: ProjectId) -> Option<PathBuf> {
        self.projects.get(&id).map(|p| p.path.clone())
    }

    /// Set the active project. Bumps its `last_active`. Returns false if the
    /// project doesn't exist.
    pub fn set_active(&mut self, id: ProjectId) -> bool {
        if let Some(p) = self.projects.get_mut(&id) {
            p.last_active = SystemTime::now();
            self.active_id = id;
            true
        } else {
            false
        }
    }

    pub fn rename(&mut self, id: ProjectId, name: impl Into<String>) -> bool {
        if let Some(p) = self.projects.get_mut(&id) {
            p.name = name.into();
            true
        } else {
            false
        }
    }

    /// Remove a project. Refuses to remove the Default project or the last
    /// remaining project. If the active project is removed, falls back to
    /// Default. Returns the removed project.
    pub fn remove(&mut self, id: ProjectId) -> Option<Project> {
        if id == DEFAULT_PROJECT_ID || self.projects.len() <= 1 {
            return None;
        }
        let removed = self.projects.remove(&id);
        if removed.is_some() && self.active_id == id {
            self.active_id = DEFAULT_PROJECT_ID;
        }
        removed
    }

    /// All projects, ordered by id (Default first).
    pub fn list(&self) -> Vec<&Project> {
        let mut v: Vec<&Project> = self.projects.values().collect();
        v.sort_by_key(|p| p.id.0);
        v
    }

    pub fn list_info(&self) -> Vec<ProjectInfo> {
        self.list().into_iter().map(Project::to_info).collect()
    }

    pub fn len(&self) -> usize {
        self.projects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    // ── Persistence ─────────────────────────────────────────────────────────

    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let state = PersistentState {
            projects: self.list().into_iter().cloned().collect(),
            active_id: self.active_id,
            next_id: self.next_id,
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    pub fn load_from_file(path: &Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let state: PersistentState = serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut projects = HashMap::new();
        let mut max_id = DEFAULT_PROJECT_ID.0;
        for p in state.projects {
            max_id = max_id.max(p.id.0);
            projects.insert(p.id, p);
        }
        let next_id = state.next_id.max(max_id + 1);
        let active_id = if projects.contains_key(&state.active_id) {
            state.active_id
        } else {
            DEFAULT_PROJECT_ID
        };

        Ok(Self {
            projects,
            active_id,
            next_id,
        })
    }

    /// Load from file or start empty (caller seeds the default project).
    pub fn load_or_new(path: &Path) -> Self {
        match Self::load_from_file(path) {
            Ok(mgr) => {
                debug!("Loaded {} projects from {:?}", mgr.projects.len(), path);
                mgr
            }
            Err(e) => {
                debug!("Creating new project manager (load failed: {})", e);
                Self::new()
            }
        }
    }
}

impl Default for ProjectManager {
    fn default() -> Self {
        Self::new()
    }
}

/// State for persistence.
#[derive(Debug, Serialize, Deserialize)]
struct PersistentState {
    projects: Vec<Project>,
    active_id: ProjectId,
    #[serde(default)]
    next_id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_project_and_active() {
        let mut mgr = ProjectManager::new();
        let id = mgr.ensure_default("/tmp/ws");
        assert_eq!(id, DEFAULT_PROJECT_ID);
        assert_eq!(mgr.active_id(), DEFAULT_PROJECT_ID);
        assert_eq!(mgr.path_of(id).unwrap().to_str(), Some("/tmp/ws"));
    }

    #[test]
    fn create_switch_remove() {
        let mut mgr = ProjectManager::new();
        mgr.ensure_default("/tmp/ws");
        let p2 = mgr.create("Side", "/tmp/side");
        assert!(mgr.set_active(p2));
        assert_eq!(mgr.active_id(), p2);

        // Default and last-project are protected.
        assert!(mgr.remove(DEFAULT_PROJECT_ID).is_none());
        assert!(mgr.remove(p2).is_some());
        assert_eq!(mgr.active_id(), DEFAULT_PROJECT_ID, "active falls back to default");
    }

    #[test]
    fn next_id_reconciled_on_load() {
        let dir = std::env::temp_dir().join(format!("rc-proj-{}", std::process::id()));
        let path = dir.join("projects.json");
        let mut mgr = ProjectManager::new();
        mgr.ensure_default("/tmp/ws");
        let p = mgr.create("A", "/tmp/a");
        mgr.save_to_file(&path).unwrap();

        let mut loaded = ProjectManager::load_from_file(&path).unwrap();
        // A freshly minted id must not collide with the loaded one.
        let p_new = loaded.create("B", "/tmp/b");
        assert_ne!(p_new, p);
        assert!(p_new.0 > p.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

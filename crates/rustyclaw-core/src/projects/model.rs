//! Project model — a project is a named working directory that groups threads.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// Unique identifier for a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub u64);

/// The well-known id of the implicit "Default" project that pre-projects
/// threads are migrated into. `ProjectId::default()` resolves here so a thread
/// deserialized from an old `threads.json` (with no `project_id`) lands in it.
pub const DEFAULT_PROJECT_ID: ProjectId = ProjectId(1);

impl Default for ProjectId {
    fn default() -> Self {
        DEFAULT_PROJECT_ID
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "p{}", self.0)
    }
}

/// A project: a named working directory that owns a set of threads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    /// User-visible name.
    pub name: String,
    /// Working directory the project's threads run in.
    pub path: PathBuf,
    pub created_at: SystemTime,
    /// When a thread in this project was last brought to the foreground.
    pub last_active: SystemTime,
}

impl Project {
    pub fn new(id: ProjectId, name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        let now = SystemTime::now();
        Self {
            id,
            name: name.into(),
            path: path.into(),
            created_at: now,
            last_active: now,
        }
    }
}

/// Summary for the sidebar / wire.
impl From<&Project> for ProjectInfo {
    fn from(p: &Project) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            path: p.path.display().to_string(),
        }
    }
}

/// Summary info for sidebar display and the gateway wire protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: ProjectId,
    pub name: String,
    pub path: String,
}

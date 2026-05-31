//! Projects — a project is a named working directory that groups threads.
//!
//! Each thread belongs to exactly one project (see
//! [`AgentThread::project_id`](crate::threads::AgentThread)). The active
//! project's [`path`](Project::path) is the agent's working directory; the
//! gateway flips `config.workspace_dir()` to it whenever the foreground
//! thread (and thus the active project) changes.

mod manager;
mod model;

pub use manager::ProjectManager;
pub use model::{DEFAULT_PROJECT_ID, Project, ProjectId, ProjectInfo};

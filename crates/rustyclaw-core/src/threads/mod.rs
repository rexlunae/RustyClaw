//! Unified Agent Thread System
//!
//! All concurrent agent-managed work is represented as threads:
//! - Chat threads (user-interactive conversations)
//! - Sub-agent threads (spawned workers)
//! - Background threads (long-running monitors)
//! - Task threads (one-shot work that returns a result)
//!
//! All threads share:
//! - Unique ID and label
//! - Agent-settable description
//! - Status tracking
//! - Sidebar visibility
//! - Event emission on state changes

mod events;
mod manager;
mod model;
pub mod subtask;

pub use events::*;
pub use manager::*;
pub use model::*;
pub use subtask::{
    SpawnOptions, SubtaskHandle, SubtaskRegistry, SubtaskResult, spawn_background, spawn_subagent,
    spawn_task,
};

// Backwards compatibility: TaskId is now ThreadId
pub type TaskId = ThreadId;

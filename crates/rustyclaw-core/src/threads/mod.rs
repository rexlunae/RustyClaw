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

mod model;
mod manager;
mod events;

pub use model::*;
pub use manager::*;
pub use events::*;

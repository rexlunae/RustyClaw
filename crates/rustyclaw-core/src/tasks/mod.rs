//! Task management system for RustyClaw.
//!
//! Tasks are first-class entities that can be:
//! - Foregrounded (streaming output to user)
//! - Backgrounded (running silently)
//! - Paused/resumed
//! - Cancelled
//!
//! Task sources include:
//! - Shell commands (exec)
//! - Sub-agent sessions
//! - Cron jobs
//! - MCP tool calls
//! - Long-running tools (browser, web_fetch, etc.)

mod model;
mod manager;
mod display;

pub use model::{Task, TaskId, TaskStatus, TaskKind, TaskProgress};
pub use manager::{TaskManager, TaskHandle, TaskEvent};
pub use display::{TaskIcon, TaskIndicator, format_task_status, format_task_indicators, format_task_icons};

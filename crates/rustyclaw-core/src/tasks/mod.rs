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

mod display;
mod manager;
mod model;
mod thread;

pub use display::{
    TaskIcon, TaskIndicator, format_task_icons, format_task_indicators, format_task_status,
};
pub use manager::{TaskEvent, TaskHandle, TaskManager};
pub use model::{Task, TaskId, TaskKind, TaskProgress, TaskStatus};
pub use thread::{
    MessageRole, SharedThreadManager, TaskThread, ThreadInfo, ThreadManager, ThreadMessage,
    ToolInteraction,
};

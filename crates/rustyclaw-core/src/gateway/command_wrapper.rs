//! Command wrapper — wraps execute_command in Tasks.
//!
//! Intercepts execute_command tool calls at the gateway level to create
//! first-class Task entries for all command executions.

use serde_json::{json, Value};
use tracing::{debug, instrument};

use super::SharedTaskManager;
use crate::tasks::{TaskId, TaskKind, TaskProgress};

/// Check if a tool call should be wrapped in a Task.
pub fn should_wrap_in_task(tool_name: &str) -> bool {
    matches!(tool_name, "execute_command")
}

/// Wrap a command execution in a Task.
///
/// This is called BEFORE the actual tool execution. It creates the task,
/// starts it, and returns the task ID. The caller should then execute
/// the tool and call `complete_command_task` or `fail_command_task`.
#[instrument(skip(task_mgr, args))]
pub async fn start_command_task(
    task_mgr: &SharedTaskManager,
    args: &Value,
    session_key: &str,
) -> TaskId {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");

    let background = args
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let kind = TaskKind::Command {
        command: truncate_command(command, 100),
        pid: None,
    };

    let handle = task_mgr.create(kind, Some(session_key.to_string())).await;
    let task_id = handle.id;

    // Start the task
    task_mgr.start(task_id).await;

    // If it's a background command, move to background immediately
    if background {
        let _ = task_mgr.set_background(task_id).await;
    }

    debug!(task_id = %task_id, command = %truncate_command(command, 50), "Command task started");

    task_id
}

/// Update a command task with the process session ID (for background commands).
pub async fn update_command_task_session(
    task_mgr: &SharedTaskManager,
    task_id: TaskId,
    session_id: &str,
) {
    // We can't easily update the TaskKind after creation, but we can add
    // progress message with the session ID
    task_mgr.update_status(
        task_id,
        crate::tasks::TaskStatus::Background {
            progress: None,
            message: Some(format!("Session: {}", session_id)),
        },
    ).await;
}

/// Mark a command task as completed.
#[instrument(skip(task_mgr))]
pub async fn complete_command_task(
    task_mgr: &SharedTaskManager,
    task_id: TaskId,
    output: &str,
) {
    let summary = if output.len() > 100 {
        Some(format!("{}...", &output[..100]))
    } else if !output.is_empty() {
        Some(output.to_string())
    } else {
        Some("Completed".to_string())
    };

    task_mgr.complete(task_id, summary).await;
    debug!(task_id = %task_id, "Command task completed");
}

/// Mark a command task as failed.
#[instrument(skip(task_mgr))]
pub async fn fail_command_task(
    task_mgr: &SharedTaskManager,
    task_id: TaskId,
    error: &str,
) {
    task_mgr.fail(task_id, error.to_string(), true).await;
    debug!(task_id = %task_id, error = %error, "Command task failed");
}

/// Parse the session ID from a background command response.
pub fn parse_session_id(output: &str) -> Option<String> {
    // Background commands return JSON like:
    // {"status":"running","sessionId":"abc123","message":"..."}
    if let Ok(v) = serde_json::from_str::<Value>(output) {
        v.get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    } else {
        None
    }
}

/// Truncate a command string for display.
fn truncate_command(cmd: &str, max_len: usize) -> String {
    let first_line = cmd.lines().next().unwrap_or(cmd);
    if first_line.len() > max_len {
        format!("{}...", &first_line[..max_len])
    } else {
        first_line.to_string()
    }
}

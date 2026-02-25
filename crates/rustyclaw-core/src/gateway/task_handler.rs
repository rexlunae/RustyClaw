//! Task handler — gateway-side task tool dispatch.
//!
//! Handles task_* tool calls by interacting with the shared TaskManager.

use serde_json::{json, Value};
use tracing::{debug, instrument};

use super::SharedTaskManager;
use crate::tasks::{Task, TaskId, TaskStatus, TaskKind};
use crate::tasks::display::{format_task_status, format_task_indicators};

/// Check if a tool name is a task tool.
pub fn is_task_tool(name: &str) -> bool {
    matches!(
        name,
        "task_list" | "task_status" | "task_foreground" | "task_background"
            | "task_cancel" | "task_pause" | "task_resume" | "task_input"
    )
}

/// Execute a task tool call.
#[instrument(skip(task_mgr, args), fields(tool = %name))]
pub async fn execute_task_tool(
    name: &str,
    args: &Value,
    task_mgr: &SharedTaskManager,
    session_key: Option<&str>,
) -> Result<String, String> {
    match name {
        "task_list" => exec_task_list(args, task_mgr, session_key).await,
        "task_status" => exec_task_status(args, task_mgr).await,
        "task_foreground" => exec_task_foreground(args, task_mgr).await,
        "task_background" => exec_task_background(args, task_mgr).await,
        "task_cancel" => exec_task_cancel(args, task_mgr).await,
        "task_pause" => exec_task_pause(args, task_mgr).await,
        "task_resume" => exec_task_resume(args, task_mgr).await,
        "task_input" => exec_task_input(args, task_mgr).await,
        _ => Err(format!("Unknown task tool: {}", name)),
    }
}

/// List active tasks.
async fn exec_task_list(
    args: &Value,
    task_mgr: &SharedTaskManager,
    current_session: Option<&str>,
) -> Result<String, String> {
    let session_filter = args.get("session").and_then(|v| v.as_str());
    let include_completed = args.get("includeCompleted").and_then(|v| v.as_bool()).unwrap_or(false);

    let tasks: Vec<Task> = if let Some(session) = session_filter {
        task_mgr.for_session(session).await
    } else if let Some(session) = current_session {
        // Default to current session's tasks
        task_mgr.for_session(session).await
    } else {
        task_mgr.all().await
    };

    let filtered: Vec<&Task> = tasks.iter()
        .filter(|t| include_completed || !t.status.is_terminal())
        .collect();

    if filtered.is_empty() {
        return Ok(json!({
            "tasks": [],
            "message": "No active tasks"
        }).to_string());
    }

    let task_list: Vec<Value> = filtered.iter().map(|t| {
        json!({
            "id": t.id.0,
            "kind": t.kind.display_name(),
            "label": t.display_label(),
            "status": format_status_short(&t.status),
            "foreground": t.status.is_foreground(),
            "elapsed": t.elapsed().map(|d| d.as_secs()),
            "progress": t.status.progress(),
        })
    }).collect();

    let indicators = format_task_indicators(&filtered.iter().cloned().cloned().collect::<Vec<_>>(), 5);

    Ok(json!({
        "tasks": task_list,
        "count": filtered.len(),
        "indicators": indicators,
    }).to_string())
}

/// Get detailed task status.
async fn exec_task_status(args: &Value, task_mgr: &SharedTaskManager) -> Result<String, String> {
    let task_id = parse_task_id(args)?;
    
    let task = task_mgr.get(task_id).await
        .ok_or_else(|| format!("Task {} not found", task_id))?;

    Ok(json!({
        "id": task.id.0,
        "kind": task.kind.display_name(),
        "kindDetails": task.kind.description(),
        "label": task.display_label(),
        "status": format_task_status(&task),
        "statusCode": format_status_short(&task.status),
        "foreground": task.status.is_foreground(),
        "progress": task.status.progress(),
        "message": task.status.message(),
        "elapsed": task.elapsed().map(|d| d.as_secs()),
        "session": task.session_key,
        "output": if task.status.is_terminal() {
            task.output_buffer.clone()
        } else {
            String::new()
        },
    }).to_string())
}

/// Bring task to foreground.
async fn exec_task_foreground(args: &Value, task_mgr: &SharedTaskManager) -> Result<String, String> {
    let task_id = parse_task_id(args)?;
    
    task_mgr.set_foreground(task_id).await?;
    
    let task = task_mgr.get(task_id).await
        .ok_or_else(|| format!("Task {} not found", task_id))?;

    Ok(json!({
        "success": true,
        "id": task_id.0,
        "label": task.display_label(),
        "message": format!("Task {} is now in foreground", task_id),
    }).to_string())
}

/// Move task to background.
async fn exec_task_background(args: &Value, task_mgr: &SharedTaskManager) -> Result<String, String> {
    let task_id = parse_task_id(args)?;
    
    task_mgr.set_background(task_id).await?;
    
    let task = task_mgr.get(task_id).await
        .ok_or_else(|| format!("Task {} not found", task_id))?;

    Ok(json!({
        "success": true,
        "id": task_id.0,
        "label": task.display_label(),
        "message": format!("Task {} moved to background", task_id),
    }).to_string())
}

/// Cancel a task.
async fn exec_task_cancel(args: &Value, task_mgr: &SharedTaskManager) -> Result<String, String> {
    let task_id = parse_task_id(args)?;
    
    task_mgr.cancel(task_id).await?;

    Ok(json!({
        "success": true,
        "id": task_id.0,
        "message": format!("Task {} cancelled", task_id),
    }).to_string())
}

/// Pause a task.
async fn exec_task_pause(args: &Value, task_mgr: &SharedTaskManager) -> Result<String, String> {
    let task_id = parse_task_id(args)?;
    
    // Update status to paused
    task_mgr.update_status(task_id, TaskStatus::Paused { reason: None }).await;

    Ok(json!({
        "success": true,
        "id": task_id.0,
        "message": format!("Task {} paused", task_id),
        "note": "Not all task types support pause/resume",
    }).to_string())
}

/// Resume a paused task.
async fn exec_task_resume(args: &Value, task_mgr: &SharedTaskManager) -> Result<String, String> {
    let task_id = parse_task_id(args)?;
    
    // Check if task exists and is paused
    let task = task_mgr.get(task_id).await
        .ok_or_else(|| format!("Task {} not found", task_id))?;
    
    if !matches!(task.status, TaskStatus::Paused { .. }) {
        return Err(format!("Task {} is not paused (status: {})", task_id, format_status_short(&task.status)));
    }
    
    // Update status back to running
    task_mgr.update_status(task_id, TaskStatus::Running { 
        progress: None, 
        message: Some("Resumed".to_string()) 
    }).await;

    Ok(json!({
        "success": true,
        "id": task_id.0,
        "message": format!("Task {} resumed", task_id),
    }).to_string())
}

/// Send input to a task.
async fn exec_task_input(args: &Value, task_mgr: &SharedTaskManager) -> Result<String, String> {
    let task_id = parse_task_id(args)?;
    let input = args.get("input")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: input")?;
    
    let task = task_mgr.get(task_id).await
        .ok_or_else(|| format!("Task {} not found", task_id))?;
    
    if !matches!(task.status, TaskStatus::WaitingForInput { .. }) {
        return Err(format!("Task {} is not waiting for input (status: {})", task_id, format_status_short(&task.status)));
    }
    
    // TODO: Actually send input via TaskHandle
    // This requires storing TaskHandles in TaskManager
    
    Ok(json!({
        "success": true,
        "id": task_id.0,
        "input": input,
        "message": format!("Input sent to task {}", task_id),
        "note": "Task input delivery not yet fully implemented",
    }).to_string())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn parse_task_id(args: &Value) -> Result<TaskId, String> {
    let id = args.get("id")
        .or_else(|| args.get("taskId"))
        .and_then(|v| v.as_u64())
        .ok_or("Missing required parameter: id (task ID)")?;
    
    Ok(TaskId(id))
}

fn format_status_short(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running { .. } => "running",
        TaskStatus::Background { .. } => "background",
        TaskStatus::Paused { .. } => "paused",
        TaskStatus::Completed { .. } => "completed",
        TaskStatus::Failed { .. } => "failed",
        TaskStatus::Cancelled => "cancelled",
        TaskStatus::WaitingForInput { .. } => "waiting_input",
    }
}

/// Generate a system prompt section describing active tasks.
pub async fn generate_task_prompt_section(
    task_mgr: &SharedTaskManager,
    session_key: &str,
) -> Option<String> {
    let tasks = task_mgr.for_session(session_key).await;
    let active: Vec<_> = tasks.iter().filter(|t| !t.status.is_terminal()).collect();
    
    if active.is_empty() {
        return None;
    }
    
    let mut section = String::from("## Active Tasks\n");
    
    for task in &active {
        let icon = crate::tasks::display::TaskIcon::from_status(&task.status);
        let fg = if task.status.is_foreground() { " [foreground]" } else { "" };
        section.push_str(&format!(
            "- {} #{}: {}{}\n",
            icon.emoji(),
            task.id.0,
            task.display_label(),
            fg
        ));
    }
    
    section.push_str("\nUse task_foreground/task_background to switch focus, task_cancel to stop.\n");
    
    Some(section)
}

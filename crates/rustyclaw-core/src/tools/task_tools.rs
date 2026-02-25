//! Task management tools for the agent.

use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, instrument};

/// List active tasks.
#[instrument(skip(args, _workspace_dir), fields(action = "list"))]
pub fn exec_task_list(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let session = args.get("session").and_then(|v| v.as_str());
    let include_completed = args.get("includeCompleted").and_then(|v| v.as_bool()).unwrap_or(false);

    debug!(?session, include_completed, "Listing tasks");

    // This is a stub — the gateway intercepts this and uses its TaskManager
    Ok(json!({
        "status": "stub",
        "note": "Task listing requires gateway connection. The gateway maintains task state.",
        "session": session,
        "includeCompleted": include_completed,
    }).to_string())
}

/// Get task status.
#[instrument(skip(args, _workspace_dir), fields(action = "status"))]
pub fn exec_task_status(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let task_id = args.get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| args.get("taskId").and_then(|v| v.as_u64()))
        .ok_or("Missing required parameter: id (task ID)")?;

    debug!(task_id, "Getting task status");

    Ok(json!({
        "status": "stub",
        "note": "Task status requires gateway connection.",
        "taskId": task_id,
    }).to_string())
}

/// Foreground a task (bring to attention, stream output).
#[instrument(skip(args, _workspace_dir), fields(action = "foreground"))]
pub fn exec_task_foreground(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let task_id = args.get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| args.get("taskId").and_then(|v| v.as_u64()))
        .ok_or("Missing required parameter: id (task ID)")?;

    debug!(task_id, "Foregrounding task");

    Ok(json!({
        "status": "stub",
        "note": "Task foreground requires gateway connection.",
        "taskId": task_id,
        "action": "foreground",
    }).to_string())
}

/// Background a task (continue running but don't stream output).
#[instrument(skip(args, _workspace_dir), fields(action = "background"))]
pub fn exec_task_background(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let task_id = args.get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| args.get("taskId").and_then(|v| v.as_u64()))
        .ok_or("Missing required parameter: id (task ID)")?;

    debug!(task_id, "Backgrounding task");

    Ok(json!({
        "status": "stub",
        "note": "Task background requires gateway connection.",
        "taskId": task_id,
        "action": "background",
    }).to_string())
}

/// Cancel a task.
#[instrument(skip(args, _workspace_dir), fields(action = "cancel"))]
pub fn exec_task_cancel(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let task_id = args.get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| args.get("taskId").and_then(|v| v.as_u64()))
        .ok_or("Missing required parameter: id (task ID)")?;

    debug!(task_id, "Cancelling task");

    Ok(json!({
        "status": "stub",
        "note": "Task cancel requires gateway connection.",
        "taskId": task_id,
        "action": "cancel",
    }).to_string())
}

/// Pause a task.
#[instrument(skip(args, _workspace_dir), fields(action = "pause"))]
pub fn exec_task_pause(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let task_id = args.get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| args.get("taskId").and_then(|v| v.as_u64()))
        .ok_or("Missing required parameter: id (task ID)")?;

    debug!(task_id, "Pausing task");

    Ok(json!({
        "status": "stub",
        "note": "Task pause requires gateway connection.",
        "taskId": task_id,
        "action": "pause",
    }).to_string())
}

/// Resume a paused task.
#[instrument(skip(args, _workspace_dir), fields(action = "resume"))]
pub fn exec_task_resume(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let task_id = args.get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| args.get("taskId").and_then(|v| v.as_u64()))
        .ok_or("Missing required parameter: id (task ID)")?;

    debug!(task_id, "Resuming task");

    Ok(json!({
        "status": "stub",
        "note": "Task resume requires gateway connection.",
        "taskId": task_id,
        "action": "resume",
    }).to_string())
}

/// Send input to a task waiting for input.
#[instrument(skip(args, _workspace_dir), fields(action = "input"))]
pub fn exec_task_input(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let task_id = args.get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| args.get("taskId").and_then(|v| v.as_u64()))
        .ok_or("Missing required parameter: id (task ID)")?;
    
    let input = args.get("input")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: input")?;

    debug!(task_id, "Sending input to task");

    Ok(json!({
        "status": "stub",
        "note": "Task input requires gateway connection.",
        "taskId": task_id,
        "input": input,
    }).to_string())
}

// ── Parameter definitions ───────────────────────────────────────────────────

use crate::tools::params::{ParamDef, ParamType};

pub fn task_list_params() -> Vec<ParamDef> {
    vec![
        ParamDef {
            name: "session",
            description: "Filter tasks by session key",
            param_type: ParamType::String,
            required: false,
            enum_values: None,
        },
        ParamDef {
            name: "includeCompleted",
            description: "Include completed/cancelled tasks",
            param_type: ParamType::Boolean,
            required: false,
            enum_values: None,
        },
    ]
}

pub fn task_id_param() -> Vec<ParamDef> {
    vec![ParamDef {
        name: "id",
        description: "Task ID (number)",
        param_type: ParamType::Integer,
        required: true,
        enum_values: None,
    }]
}

pub fn task_input_params() -> Vec<ParamDef> {
    vec![
        ParamDef {
            name: "id",
            description: "Task ID (number)",
            param_type: ParamType::Integer,
            required: true,
            enum_values: None,
        },
        ParamDef {
            name: "input",
            description: "Input text to send to the task",
            param_type: ParamType::String,
            required: true,
            enum_values: None,
        },
    ]
}

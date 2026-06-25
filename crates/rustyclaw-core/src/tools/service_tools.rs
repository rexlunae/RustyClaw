//! Managed service tools — list, start, stop, restart, logs for gateway-managed backend processes.

use serde_json::{Value, json};
use std::path::Path;
use tracing::instrument;

use super::ToolParam;

/// Stub for `service_list`. The gateway intercepts this and responds
/// from the runtime service manager.
#[instrument(skip(_args, _workspace_dir), fields(action = "service_list"))]
pub fn exec_service_list_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Ok(json!({
        "status": "stub",
        "note": "service_list requires a gateway connection with services enabled.",
    })
    .to_string())
}

/// Stub for `service_start`.
#[instrument(skip(_args, _workspace_dir), fields(action = "service_start"))]
pub fn exec_service_start_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Ok(json!({
        "status": "stub",
        "note": "service_start requires a gateway connection with services enabled.",
    })
    .to_string())
}

/// Stub for `service_stop`.
#[instrument(skip(_args, _workspace_dir), fields(action = "service_stop"))]
pub fn exec_service_stop_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Ok(json!({
        "status": "stub",
        "note": "service_stop requires a gateway connection with services enabled.",
    })
    .to_string())
}

/// Stub for `service_restart`.
#[instrument(skip(_args, _workspace_dir), fields(action = "service_restart"))]
pub fn exec_service_restart_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Ok(json!({
        "status": "stub",
        "note": "service_restart requires a gateway connection with services enabled.",
    })
    .to_string())
}

/// Stub for `service_logs`.
#[instrument(skip(_args, _workspace_dir), fields(action = "service_logs"))]
pub fn exec_service_logs_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Ok(json!({
        "status": "stub",
        "note": "service_logs requires a gateway connection with services enabled.",
    })
    .to_string())
}

// ── Parameter definitions ───────────────────────────────────────────────────

pub fn service_list_params() -> Vec<ToolParam> {
    vec![]
}

pub fn service_start_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "name".into(),
        description: "Name of the service to start.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn service_stop_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "name".into(),
        description: "Name of the service to stop.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn service_restart_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "name".into(),
        description: "Name of the service to restart.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn service_logs_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".into(),
            description: "Name of the service to get logs from.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "tail".into(),
            description: "Number of recent log lines to return (default: 50).".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

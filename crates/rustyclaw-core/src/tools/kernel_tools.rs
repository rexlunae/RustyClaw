//! Kernel awareness tools — host hardware and load status for agents.

use serde_json::{Value, json};
use std::path::Path;
use tracing::instrument;

/// Return host hardware capabilities.
///
/// The gateway intercepts this and responds from `runtime_ctx`;
/// this stub is the fallback when running outside the gateway.
#[instrument(skip(_args, _workspace_dir), fields(action = "host_info"))]
pub fn exec_host_info_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Ok(json!({
        "status": "stub",
        "note": "host_info requires a gateway connection with host detection enabled.",
    })
    .to_string())
}

/// Return current system load status.
///
/// The gateway intercepts this and responds from the load tracker;
/// this stub is the fallback when running outside the gateway.
#[instrument(skip(_args, _workspace_dir), fields(action = "load_status"))]
pub fn exec_load_status_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Ok(json!({
        "status": "stub",
        "note": "load_status requires a gateway connection with load tracking enabled.",
    })
    .to_string())
}

// ── Parameter definitions ───────────────────────────────────────────────────

use super::ToolParam;

pub fn host_info_params() -> Vec<ToolParam> {
    vec![]
}

pub fn load_status_params() -> Vec<ToolParam> {
    vec![]
}

//! Model management tools for the agent.

use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, instrument};

/// List available models.
#[instrument(skip(args, _workspace_dir), fields(action = "list"))]
pub fn exec_model_list(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let tier = args.get("tier").and_then(|v| v.as_str());
    let enabled_only = args.get("enabledOnly").and_then(|v| v.as_bool()).unwrap_or(false);
    let usable_only = args.get("usableOnly").and_then(|v| v.as_bool()).unwrap_or(false);

    debug!(?tier, enabled_only, usable_only, "Listing models");

    // Stub — gateway intercepts this
    Ok(json!({
        "status": "stub",
        "note": "Model listing requires gateway connection with ModelRegistry.",
        "filter": {
            "tier": tier,
            "enabledOnly": enabled_only,
            "usableOnly": usable_only,
        }
    }).to_string())
}

/// Enable a model.
#[instrument(skip(args, _workspace_dir), fields(action = "enable"))]
pub fn exec_model_enable(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let model_id = args.get("id")
        .or_else(|| args.get("model"))
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: id (model ID)")?;

    debug!(model_id, "Enabling model");

    Ok(json!({
        "status": "stub",
        "note": "Model enable requires gateway connection.",
        "modelId": model_id,
    }).to_string())
}

/// Disable a model.
#[instrument(skip(args, _workspace_dir), fields(action = "disable"))]
pub fn exec_model_disable(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let model_id = args.get("id")
        .or_else(|| args.get("model"))
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: id (model ID)")?;

    debug!(model_id, "Disabling model");

    Ok(json!({
        "status": "stub",
        "note": "Model disable requires gateway connection.",
        "modelId": model_id,
    }).to_string())
}

/// Set the active model.
#[instrument(skip(args, _workspace_dir), fields(action = "set"))]
pub fn exec_model_set(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let model_id = args.get("id")
        .or_else(|| args.get("model"))
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: id (model ID)")?;

    debug!(model_id, "Setting active model");

    Ok(json!({
        "status": "stub",
        "note": "Model set requires gateway connection.",
        "modelId": model_id,
    }).to_string())
}

/// Get model recommendation for a task complexity.
#[instrument(skip(args, _workspace_dir), fields(action = "recommend"))]
pub fn exec_model_recommend(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let complexity = args.get("complexity")
        .and_then(|v| v.as_str())
        .unwrap_or("medium");

    debug!(complexity, "Getting model recommendation");

    Ok(json!({
        "status": "stub",
        "note": "Model recommendation requires gateway connection.",
        "complexity": complexity,
    }).to_string())
}

// ── Parameter definitions ───────────────────────────────────────────────────

use crate::tools::params::{ParamDef, ParamType};

pub fn model_list_params() -> Vec<ParamDef> {
    vec![
        ParamDef {
            name: "tier",
            description: "Filter by cost tier (free, economy, standard, premium)",
            param_type: ParamType::String,
            required: false,
            enum_values: Some(vec!["free", "economy", "standard", "premium"]),
        },
        ParamDef {
            name: "enabledOnly",
            description: "Only show enabled models",
            param_type: ParamType::Boolean,
            required: false,
            enum_values: None,
        },
        ParamDef {
            name: "usableOnly",
            description: "Only show usable models (enabled + available)",
            param_type: ParamType::Boolean,
            required: false,
            enum_values: None,
        },
    ]
}

pub fn model_id_param() -> Vec<ParamDef> {
    vec![ParamDef {
        name: "id",
        description: "Model ID (e.g., 'anthropic/claude-sonnet-4')",
        param_type: ParamType::String,
        required: true,
        enum_values: None,
    }]
}

pub fn model_recommend_params() -> Vec<ParamDef> {
    vec![ParamDef {
        name: "complexity",
        description: "Task complexity (simple, medium, complex, critical)",
        param_type: ParamType::String,
        required: false,
        enum_values: Some(vec!["simple", "medium", "complex", "critical"]),
    }]
}

//! Plugin tools — agent-facing commands for plugin management and interaction.
//!
//! Tools:
//!   - `plugin_list`      — list all plugins with their current state
//!   - `plugin_state_get` — read a plugin's state
//!   - `plugin_state_set` — full-replace a plugin's state
//!   - `plugin_state_patch` — merge a partial update into state
//!   - `plugin_create`    — create a new plugin from scratch

use crate::plugins::{PluginAction, PluginManager};
use crate::tools::error::ToolResult;
use crate::tools::ToolParam;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Mutex;
use tracing::instrument;

/// Global plugin manager (set by the gateway on startup).
static PLUGIN_MANAGER: std::sync::OnceLock<Mutex<PluginManager>> = std::sync::OnceLock::new();

/// Initialise the global plugin manager (idempotent).
pub fn init_plugin_manager(workspace_dir: &Path) {
    let dir = crate::plugins::default_plugins_dir(workspace_dir);
    PLUGIN_MANAGER.get_or_init(|| {
        let mut mgr = PluginManager::new(dir);
        if let Err(e) = mgr.load() {
            tracing::warn!("Failed to load plugins on init: {e}");
        }
        tracing::info!(count = mgr.plugins().len(), "Plugin manager initialised");
        Mutex::new(mgr)
    });
}

/// Access the plugin manager lock (returns an error if not initialised).
fn manager_lock() -> Result<std::sync::MutexGuard<'static, PluginManager>, String> {
    PLUGIN_MANAGER
        .get()
        .ok_or_else(|| "Plugin manager not initialised".to_string())?
        .lock()
        .map_err(|e| format!("Plugin manager lock: {e}"))
}

/// Get the plugin prompt context for the agent.
pub fn plugin_prompt_context() -> String {
    PLUGIN_MANAGER
        .get()
        .and_then(|m| m.lock().ok())
        .map(|mgr| mgr.prompt_context())
        .unwrap_or_default()
}

/// Reload all plugins.
pub fn reload_plugins() -> Result<(), String> {
    PLUGIN_MANAGER
        .get()
        .ok_or_else(|| "Plugin manager not initialised".to_string())?
        .lock()
        .map_err(|e| format!("Plugin manager lock: {e}"))?
        .load()
        .map_err(|e| e.to_string())
}

// ── Tool executors ─────────────────────────────────────────────────────────

#[instrument(skip(args))]
pub fn exec_plugin_list(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let include_state = args
        .get("include_state")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let mgr = manager_lock().map_err(|e| format!("{e}"))?;
    let plugins = mgr.plugins();
    let mut items = Vec::new();
    for p in plugins {
        let mut obj = json!({
            "name": p.name,
            "description": p.description,
            "emoji": p.emoji,
            "version": p.version,
            "enabled": p.enabled,
            "actions": p.actions.iter().map(|a| json!({
                "name": a.name,
                "description": a.description,
            })).collect::<Vec<_>>(),
        });
        if include_state {
            obj["state"] = mgr.get_state(&p.name).unwrap_or_default();
        }
        items.push(obj);
    }
    Ok(serde_json::to_string_pretty(&json!({
        "plugins": items,
        "count": items.len(),
    }))
    .unwrap_or_default())
}

#[instrument(skip(args))]
pub fn exec_plugin_state_get(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("plugin_name")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("name").and_then(|v| v.as_str()))
        .ok_or_else(|| "Missing required parameter: plugin_name".to_string())?;

    let mgr = manager_lock().map_err(|e| format!("{e}"))?;
    let state = mgr.get_state(name).map_err(|e| format!("{e}"))?;
    Ok(serde_json::to_string_pretty(&json!({
        "plugin_name": name,
        "state": state,
    }))
    .unwrap_or_default())
}

#[instrument(skip(args))]
pub fn exec_plugin_state_set(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("plugin_name")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("name").and_then(|v| v.as_str()))
        .ok_or_else(|| "Missing required parameter: plugin_name".to_string())?;

    let state = args
        .get("state")
        .ok_or_else(|| "Missing required parameter: state".to_string())?;

    let mut mgr = manager_lock().map_err(|e| format!("{e}"))?;
    mgr.set_state(name, state.clone()).map_err(|e| format!("{e}"))?;
    Ok("ok".to_string())
}

#[instrument(skip(args))]
pub fn exec_plugin_state_patch(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("plugin_name")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("name").and_then(|v| v.as_str()))
        .ok_or_else(|| "Missing required parameter: plugin_name".to_string())?;

    let patch = args
        .get("patch")
        .ok_or_else(|| "Missing required parameter: patch".to_string())?;

    let mut mgr = manager_lock().map_err(|e| format!("{e}"))?;
    let new_state = mgr.patch_state(name, patch).map_err(|e| format!("{e}"))?;
    Ok(serde_json::to_string_pretty(&json!({
        "plugin_name": name,
        "state": new_state,
    }))
    .unwrap_or_default())
}

#[instrument(skip(args))]
pub fn exec_plugin_create(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: description".to_string())?;

    let emoji = args.get("emoji").and_then(|v| v.as_str());
    let instructions = args
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let schema = args.get("schema");
    let initial_state = args.get("initial_state");
    let html_template = args.get("html_template").and_then(|v| v.as_str());

    // Parse actions if provided
    let actions: Option<Vec<PluginAction>> = args
        .get("actions")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let mut mgr = manager_lock().map_err(|e| format!("{e}"))?;
    let plugin = mgr
        .create(
            name,
            description,
            emoji,
            instructions,
            schema,
            initial_state,
            actions,
            html_template,
        )
        .map_err(|e| format!("{e}"))?;

    Ok(serde_json::to_string_pretty(&json!({
        "created": true,
        "name": plugin.name,
        "path": plugin.path.to_string_lossy(),
        "note": "Plugin created. Use plugin_state_set to populate its initial data.",
    }))
    .unwrap_or_default())
}

// ── Parameter definitions ──────────────────────────────────────────────────

pub fn plugin_list_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "include_state".into(),
        description: "Include current state for each plugin (default: true).".into(),
        param_type: "boolean".into(),
        required: false,
    }]
}

pub fn plugin_state_get_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "plugin_name".into(),
        description: "Name of the plugin to read state from.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn plugin_state_set_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "plugin_name".into(),
            description: "Name of the plugin to update.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "state".into(),
            description: "Full new state object for the plugin (JSON object). Replaces the existing state entirely.".into(),
            param_type: "object".into(),
            required: true,
        },
    ]
}

pub fn plugin_state_patch_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "plugin_name".into(),
            description: "Name of the plugin to patch.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "patch".into(),
            description: "Partial state to merge into the plugin's current state (JSON Merge Patch). Only the keys you include are changed; all other keys remain as-is.".into(),
            param_type: "object".into(),
            required: true,
        },
    ]
}

pub fn plugin_create_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".into(),
            description: "Unique kebab-case identifier for the new plugin (e.g. 'stock-ticker').".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "description".into(),
            description: "Human-readable one-line description shown in the plugin list.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "emoji".into(),
            description: "Optional emoji icon for the plugin tab (e.g. '📊').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "instructions".into(),
            description: "Markdown instructions the agent should follow when working with this plugin (appears in the agent's plugin context).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "schema".into(),
            description: "JSON Schema for the plugin state (optional, used for validation).".into(),
            param_type: "object".into(),
            required: false,
        },
        ToolParam {
            name: "initial_state".into(),
            description: "Initial state object for the plugin (default: empty object).".into(),
            param_type: "object".into(),
            required: false,
        },
        ToolParam {
            name: "actions".into(),
            description: "Array of named actions the agent can invoke: [{\"name\":\"refresh\",\"description\":\"...\",\"params\":[...]}].".into(),
            param_type: "array".into(),
            required: false,
        },
        ToolParam {
            name: "html_template".into(),
            description: "Optional custom HTML template for rendering the plugin's panel (default: auto-rendered from state).".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

//! Trigger management tools: triggers_create, triggers_list, triggers_update,
//! triggers_delete, triggers_set_enabled.
//!
//! These let an agent define external triggers — small programs the gateway
//! runs for its whole lifetime that fire an agent with a JSON context by
//! calling back to the gateway. Definitions live in the encrypted
//! [`crate::triggers::TriggerStore`]; the gateway's trigger manager picks up
//! changes within a few seconds (or immediately when it is notified via the
//! runtime context).

use crate::tools::error::{ToolError, ToolResult};
use crate::triggers::{TriggerDef, TriggerStore, derive_trigger_id};
use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument};

fn store() -> Result<TriggerStore, ToolError> {
    let settings_dir = crate::runtime_ctx::get_settings_dir().ok_or_else(|| {
        ToolError::from(
            "Trigger store unavailable: the gateway has not published its settings directory",
        )
    })?;
    Ok(TriggerStore::open(&settings_dir))
}

fn notify_manager() {
    crate::runtime_ctx::notify_triggers_changed();
}

fn now_secs() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Create a new trigger.
#[instrument(skip(args, _workspace_dir), fields(name))]
pub fn exec_triggers_create(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: code".to_string())?;

    tracing::Span::current().record("name", name);

    let store = store()?;
    let id = match args.get("triggerId").and_then(|v| v.as_str()) {
        Some(explicit) => explicit.to_string(),
        None => {
            let derived = derive_trigger_id(name);
            if derived.is_empty() {
                return Err(format!("Could not derive a trigger id from name '{}'", name).into());
            }
            derived
        }
    };
    if store.get(&id).map_err(|e| e.to_string())?.is_some() {
        return Err(format!(
            "Trigger '{}' already exists — use triggers_update to change it",
            id
        )
        .into());
    }

    let agent_id = args
        .get("agentId")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(crate::runtime_ctx::get_active_agent)
        .unwrap_or_else(|| crate::agents::MAIN_AGENT_ID.to_string());

    // Point the trigger at a real agent so a typo doesn't silently create
    // a trigger that can never run anything.
    if let Some(registry) = crate::runtime_ctx::get_agent_registry() {
        if !registry.exists(&agent_id) {
            return Err(format!(
                "Unknown agent id '{}'. Use agents_list to see registered agents.",
                agent_id
            )
            .into());
        }
    }

    let def = TriggerDef {
        id: id.clone(),
        name: name.to_string(),
        description: args
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        agent_id,
        code: code.to_string(),
        interpreter: args
            .get("interpreter")
            .and_then(|v| v.as_str())
            .map(String::from),
        enabled: args
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        created_by: crate::runtime_ctx::get_active_agent(),
        created_at: now_secs(),
    };
    debug!(id = %def.id, agent = %def.agent_id, "Creating trigger");
    store.upsert(&def).map_err(|e| e.to_string())?;
    notify_manager();

    Ok(format!(
        "Trigger created.\n\nid: {}\nname: {}\nagent: {}\nenabled: {}\n\n\
         The gateway starts the trigger's code as a child process and keeps \
         it running while the gateway runs. Inside the code, fire the agent \
         by POSTing to http://$RUSTYCLAW_TRIGGER_ENDPOINT/fire with JSON \
         {{\"token\": \"$RUSTYCLAW_TRIGGER_TOKEN\", \"context\": {{...}}}}.",
        def.id, def.name, def.agent_id, def.enabled
    ))
}

/// List all triggers.
#[instrument(skip(_args, _workspace_dir))]
pub fn exec_triggers_list(_args: &Value, _workspace_dir: &Path) -> ToolResult {
    let defs = store()?.list().map_err(|e| e.to_string())?;
    if defs.is_empty() {
        return Ok("No triggers defined. Use triggers_create to add one.".to_string());
    }
    let mut out = String::from("Triggers:\n\n");
    for def in defs {
        out.push_str(&format!(
            "- {} ({}) → agent '{}' [{}]{}\n",
            def.id,
            def.name,
            def.agent_id,
            if def.enabled { "enabled" } else { "disabled" },
            def.description
                .as_deref()
                .map(|d| format!(" — {}", d))
                .unwrap_or_default(),
        ));
    }
    Ok(out)
}

/// Update fields of an existing trigger.
#[instrument(skip(args, _workspace_dir), fields(trigger_id))]
pub fn exec_triggers_update(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let id = args
        .get("triggerId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: triggerId".to_string())?;
    tracing::Span::current().record("trigger_id", id);

    let store = store()?;
    let mut def = store
        .get(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Trigger '{}' not found", id))?;

    if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
        def.name = name.to_string();
    }
    if let Some(desc) = args.get("description").and_then(|v| v.as_str()) {
        def.description = Some(desc.to_string());
    }
    if let Some(code) = args.get("code").and_then(|v| v.as_str()) {
        def.code = code.to_string();
    }
    if let Some(interp) = args.get("interpreter").and_then(|v| v.as_str()) {
        def.interpreter = Some(interp.to_string());
    }
    if let Some(agent_id) = args.get("agentId").and_then(|v| v.as_str()) {
        if let Some(registry) = crate::runtime_ctx::get_agent_registry() {
            if !registry.exists(agent_id) {
                return Err(format!("Unknown agent id '{}'", agent_id).into());
            }
        }
        def.agent_id = agent_id.to_string();
    }
    if let Some(enabled) = args.get("enabled").and_then(|v| v.as_bool()) {
        def.enabled = enabled;
    }

    store.upsert(&def).map_err(|e| e.to_string())?;
    notify_manager();
    Ok(format!(
        "Trigger '{}' updated. The gateway will restart its process with the new definition.",
        id
    ))
}

/// Delete a trigger.
#[instrument(skip(args, _workspace_dir), fields(trigger_id))]
pub fn exec_triggers_delete(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let id = args
        .get("triggerId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: triggerId".to_string())?;
    tracing::Span::current().record("trigger_id", id);

    store()?.remove(id).map_err(|e| e.to_string())?;
    notify_manager();
    Ok(format!("Trigger '{}' deleted.", id))
}

/// Enable or disable a trigger.
#[instrument(skip(args, _workspace_dir), fields(trigger_id))]
pub fn exec_triggers_set_enabled(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let id = args
        .get("triggerId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: triggerId".to_string())?;
    let enabled = args
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "Missing required parameter: enabled".to_string())?;
    tracing::Span::current().record("trigger_id", id);

    store()?
        .set_enabled(id, enabled)
        .map_err(|e| e.to_string())?;
    notify_manager();
    Ok(format!(
        "Trigger '{}' {}.",
        id,
        if enabled { "enabled" } else { "disabled" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ws() -> &'static Path {
        Path::new("/tmp")
    }

    #[test]
    fn create_requires_name_and_code() {
        // Missing params fail regardless of runtime-ctx state.
        assert!(exec_triggers_create(&json!({"code": "echo"}), ws()).is_err());
        assert!(exec_triggers_create(&json!({"name": "x"}), ws()).is_err());
    }

    #[test]
    fn crud_via_tools() {
        let tmp = tempfile::tempdir().unwrap();
        crate::runtime_ctx::set_agent_registry_info(tmp.path(), "RustyClaw");

        let out = exec_triggers_create(
            &json!({
                "name": "Repo Watcher",
                "code": "while true; do sleep 60; done",
                "description": "watches a repo",
            }),
            ws(),
        )
        .unwrap();
        assert!(out.contains("repo-watcher"));

        let listed = exec_triggers_list(&json!({}), ws()).unwrap();
        assert!(listed.contains("repo-watcher"));
        assert!(listed.contains("enabled"));

        exec_triggers_set_enabled(
            &json!({"triggerId": "repo-watcher", "enabled": false}),
            ws(),
        )
        .unwrap();
        assert!(
            exec_triggers_list(&json!({}), ws())
                .unwrap()
                .contains("disabled")
        );

        exec_triggers_update(
            &json!({"triggerId": "repo-watcher", "code": "echo updated"}),
            ws(),
        )
        .unwrap();

        exec_triggers_delete(&json!({"triggerId": "repo-watcher"}), ws()).unwrap();
        assert!(
            exec_triggers_list(&json!({}), ws())
                .unwrap()
                .contains("No triggers defined")
        );

        // Unknown agent id is rejected.
        assert!(
            exec_triggers_create(
                &json!({"name": "bad", "code": "echo", "agentId": "ghost"}),
                ws()
            )
            .is_err()
        );
    }
}

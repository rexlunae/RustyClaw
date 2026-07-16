//! Agent management tools: agents_create, agents_delete.
//!
//! These let an agent spawn (create) other persistent agents in the same
//! installation via the [`crate::agents::AgentRegistry`]. Newly-created
//! agents show up in `agents_list`, can be targeted with `sessions_spawn`,
//! and can be switched to from any connected client.

use crate::tools::error::{ToolError, ToolResult};
use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument};

fn registry() -> Result<crate::agents::AgentRegistry, ToolError> {
    crate::runtime_ctx::get_agent_registry().ok_or_else(|| {
        ToolError::from(
            "Agent registry unavailable: the gateway has not published its settings directory",
        )
    })
}

/// Create a new persistent agent in this installation.
#[instrument(skip(args, _workspace_dir), fields(name))]
pub fn exec_agents_create(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;
    let id = args.get("agentId").and_then(|v| v.as_str());
    let description = args.get("description").and_then(|v| v.as_str());
    let system_prompt = args.get("systemPrompt").and_then(|v| v.as_str());

    tracing::Span::current().record("name", name);
    debug!(id, "Creating agent");

    let created_by = crate::runtime_ctx::get_active_agent();
    let info = registry()?
        .create(id, name, description, system_prompt, created_by.as_deref())
        .map_err(|e| e.to_string())?;

    Ok(format!(
        "Agent created.\n\nid: {}\nname: {}\n{}\
         Spawn a task under it with sessions_spawn (agentId: \"{}\"), or switch \
         to it from the client UI.",
        info.id,
        info.name,
        info.description
            .as_deref()
            .map(|d| format!("description: {}\n", d))
            .unwrap_or_default(),
        info.id
    ))
}

/// Delete a persistent agent (and all its state). `main` is protected.
#[instrument(skip(args, _workspace_dir), fields(agent_id))]
pub fn exec_agents_delete(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let agent_id = args
        .get("agentId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: agentId".to_string())?;

    tracing::Span::current().record("agent_id", agent_id);
    debug!("Deleting agent");

    // Refuse to delete the agent this conversation is running as — its
    // session state would be removed out from under the live connection.
    // (Best-effort: see `runtime_ctx::get_active_agent` for the concurrency
    // caveat. The gateway's own AgentDelete frame handler enforces the same
    // rule per-connection.)
    if crate::runtime_ctx::get_active_agent().as_deref() == Some(agent_id) {
        return Err(format!(
            "Cannot delete agent '{}' while it is the active agent — switch to \
             another agent first.",
            agent_id
        )
        .into());
    }

    registry()?.delete(agent_id).map_err(|e| e.to_string())?;
    Ok(format!("Agent '{}' deleted.", agent_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ws() -> &'static Path {
        Path::new("/tmp")
    }

    #[test]
    fn create_requires_name() {
        let result = exec_agents_create(&json!({}), ws());
        assert!(result.is_err());
    }

    #[test]
    fn delete_requires_agent_id() {
        let result = exec_agents_delete(&json!({}), ws());
        assert!(result.is_err());
    }

    #[test]
    fn create_and_delete_roundtrip() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        crate::runtime_ctx::set_agent_registry_info(tmp.path(), "RustyClaw");

        let out = exec_agents_create(&json!({"name": "Helper Bot", "description": "helps"}), ws())
            .unwrap();
        assert!(out.contains("helper-bot"));

        // main cannot be deleted
        assert!(exec_agents_delete(&json!({"agentId": "main"}), ws()).is_err());

        let out = exec_agents_delete(&json!({"agentId": "helper-bot"}), ws()).unwrap();
        assert!(out.contains("deleted"));
    }
}

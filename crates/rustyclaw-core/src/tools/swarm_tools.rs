//! Swarm management tools.
//!
//! Agent-callable tools for creating, listing, inspecting, messaging,
//! stopping, and deleting swarms. Swarms are persistent groups of
//! registered agents (see [`crate::swarm`]): `swarm_create` materializes
//! every member as a real agent in the registry, and `swarm_send` routes
//! tasks into sub-agent sessions under those agents.

use crate::tools::error::{ToolError, ToolResult};
use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument};

use crate::agents::AgentRegistry;
use crate::swarm::{
    FlowKind, SwarmConfig, SwarmRecord, SwarmStore, builtin_templates, create_swarm, delete_swarm,
    member_statuses, send_to_member, stop_swarm_sessions,
};

/// The store and registry every swarm tool operates on. Both are
/// filesystem-backed off the gateway's settings dir, so tools, the CLI,
/// and the gateway always see the same swarms and agents.
fn swarm_context() -> Result<(SwarmStore, AgentRegistry), ToolError> {
    let store = crate::runtime_ctx::get_swarm_store();
    let registry = crate::runtime_ctx::get_agent_registry();
    match (store, registry) {
        (Some(store), Some(registry)) => Ok((store, registry)),
        _ => Err(ToolError::from(
            "Swarm store unavailable: the gateway has not published its settings directory",
        )),
    }
}

fn summarize(record: &SwarmRecord, registry: &AgentRegistry) -> String {
    let statuses = member_statuses(record, registry);
    let registered = statuses.iter().filter(|s| s.registered).count();
    let active = statuses.iter().filter(|s| s.session_active).count();
    let health = if registered == statuses.len() {
        "ready"
    } else {
        "degraded"
    };
    format!(
        "{} — {} agents ({} registered, {} active sessions) [{}]\n   {}",
        record.config.name,
        statuses.len(),
        registered,
        active,
        health,
        record.config.description,
    )
}

// ── swarm_create ────────────────────────────────────────────────────────────

/// Create a swarm from a built-in template or inline JSON config,
/// materializing every member as a persistent registered agent.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_create(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let template_name = args.get("template").and_then(|v| v.as_str());
    let custom_config = args.get("config");

    let mut config: SwarmConfig = if let Some(cfg_val) = custom_config {
        serde_json::from_value(cfg_val.clone())
            .map_err(|e| ToolError::context("Invalid swarm config", e))?
    } else {
        let tpl_name = template_name.unwrap_or("swarm");
        builtin_templates()
            .into_iter()
            .find(|t| t.name == tpl_name)
            .ok_or_else(|| {
                let available: Vec<String> =
                    builtin_templates().iter().map(|t| t.name.clone()).collect();
                format!(
                    "Unknown template '{tpl_name}'. Available: {}",
                    available.join(", ")
                )
            })?
    };
    // An explicit name renames a template instantiation (so one template
    // can back several swarms).
    if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
        config.name = name.to_string();
    }

    debug!(swarm = %config.name, agents = config.agents.len(), "Creating swarm");

    let (store, registry) = swarm_context()?;
    let record = create_swarm(&store, &registry, config).map_err(|e| format!("{e:#}"))?;

    let agent_list: Vec<String> = record
        .config
        .agents
        .iter()
        .map(|a| {
            format!(
                "  - {} ({}) → agent id: {}",
                a.name,
                a.role,
                record.agent_id_for(&a.id).unwrap_or("?"),
            )
        })
        .collect();

    Ok(format!(
        "Swarm '{}' created. Each member is now a registered agent:\n{}\n\n\
         Route tasks with swarm_send, or target members directly with \
         sessions_spawn using the agent ids above.",
        record.config.name,
        agent_list.join("\n")
    ))
}

// ── swarm_list ──────────────────────────────────────────────────────────────

/// List all swarms with derived health.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_list(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let _ = args; // no parameters needed
    let (store, registry) = swarm_context()?;

    let swarms = store.list();
    if swarms.is_empty() {
        return Ok("No swarms defined. Use swarm_create to create one (template: 'swarm').".into());
    }

    let mut output = String::from("Swarms:\n\n");
    for record in &swarms {
        output.push_str(&summarize(record, &registry));
        output.push('\n');
    }
    Ok(output)
}

// ── swarm_status ────────────────────────────────────────────────────────────

/// Get detailed status for a named swarm.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_status(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    let (store, registry) = swarm_context()?;
    let record = store
        .get(name)
        .ok_or_else(|| format!("Swarm '{name}' not found"))?;

    let statuses = member_statuses(&record, &registry);
    let registered = statuses.iter().filter(|s| s.registered).count();
    let health = if registered == statuses.len() {
        "ready"
    } else {
        "degraded (missing agents — recreate the swarm)"
    };

    let mut output = format!(
        "Swarm: {}\nHealth: {}\nDescription: {}\nAgents: {}\nAge: {}s\n\nMembers:\n",
        record.config.name,
        health,
        record.config.description,
        statuses.len(),
        record.age_secs(),
    );
    for s in &statuses {
        output.push_str(&format!(
            "  {} {} ({}) → {} — {}{}\n",
            if s.registered {
                "✅"
            } else {
                "⚠️ missing"
            },
            s.name,
            s.role,
            s.agent_id,
            s.description,
            if s.session_active {
                " [session active]"
            } else {
                ""
            },
        ));
    }

    output.push_str("\nCommunication Flows:\n");
    for flow in &record.config.flows {
        if flow.kind == FlowKind::SendMessage {
            output.push_str(&format!("  {} → {} [{}]\n", flow.from, flow.to, flow.kind));
        }
    }
    let handoff_count = record
        .config
        .flows
        .iter()
        .filter(|f| f.kind == FlowKind::Handoff)
        .count();
    output.push_str(&format!(
        "  + {} bidirectional Handoff flows\n",
        handoff_count
    ));

    Ok(output)
}

// ── swarm_send ──────────────────────────────────────────────────────────────

/// Send a task/message to a swarm member (default: the orchestrator).
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_send(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let swarm_name = args
        .get("swarm")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: swarm".to_string())?;
    let agent = args.get("agent").and_then(|v| v.as_str());
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: message".to_string())?;

    debug!(
        swarm = %swarm_name,
        agent,
        message_len = message.len(),
        "Routing message to swarm member"
    );

    let (store, registry) = swarm_context()?;
    let route = send_to_member(&store, &registry, swarm_name, agent, message)
        .map_err(|e| format!("{e:#}"))?;

    Ok(format!(
        "Message routed to {} ({}) in swarm '{}' — {} session {}",
        route.member_name,
        route.agent_id,
        swarm_name,
        if route.reused { "existing" } else { "new" },
        route.session_key,
    ))
}

// ── swarm_stop ──────────────────────────────────────────────────────────────

/// Complete a swarm's active sessions; the swarm and its agents remain.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_stop(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    debug!(swarm = %name, "Stopping swarm sessions");
    let (store, _registry) = swarm_context()?;
    let completed = stop_swarm_sessions(&store, name).map_err(|e| format!("{e:#}"))?;

    Ok(format!(
        "Swarm '{}': {} active session(s) completed. The swarm and its \
         agents remain — use swarm_delete to remove them.",
        name, completed
    ))
}

// ── swarm_delete ────────────────────────────────────────────────────────────

/// Delete a swarm: complete its sessions, remove the member agents it
/// created, and drop the record.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_delete(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;
    let keep_agents = args
        .get("keepAgents")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    debug!(swarm = %name, keep_agents, "Deleting swarm");
    let (store, registry) = swarm_context()?;
    let report =
        delete_swarm(&store, &registry, name, keep_agents).map_err(|e| format!("{e:#}"))?;

    let mut output = format!(
        "Swarm '{}' deleted. {} session(s) completed, {} agent(s) removed",
        name,
        report.sessions_completed,
        report.agents_deleted.len(),
    );
    if !report.agents_kept.is_empty() {
        output.push_str(&format!(
            ", {} kept ({})",
            report.agents_kept.len(),
            report.agents_kept.join(", ")
        ));
    }
    output.push('.');
    Ok(output)
}

// ── swarm_templates ─────────────────────────────────────────────────────────

/// List available swarm templates.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_templates(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let _ = args;
    let templates = builtin_templates();

    let mut output = String::from("Available swarm templates:\n\n");
    for t in &templates {
        output.push_str(&format!(
            "  {} — {} agents\n    {}\n",
            t.name,
            t.agents.len(),
            t.description
        ));
        for agent in &t.agents {
            output.push_str(&format!("      • {} ({})\n", agent.name, agent.role));
        }
        output.push('\n');
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ws() -> &'static Path {
        Path::new("/tmp")
    }

    /// End-to-end tool flow over a temp settings dir published via
    /// runtime_ctx (the same wiring the gateway performs at startup).
    #[test]
    fn swarm_tool_lifecycle() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        crate::runtime_ctx::set_agent_registry_info(tmp.path(), "RustyClaw");

        let out = exec_swarm_create(&json!({"name": "tool-crew"}), ws()).unwrap();
        assert!(out.contains("tool-crew"));
        assert!(out.contains("tool-crew-orchestrator"));

        let out = exec_swarm_list(&json!({}), ws()).unwrap();
        assert!(out.contains("tool-crew"));
        assert!(out.contains("[ready]"));

        let out = exec_swarm_send(&json!({"swarm": "tool-crew", "message": "go"}), ws()).unwrap();
        assert!(out.contains("new session"));

        let out = exec_swarm_status(&json!({"name": "tool-crew"}), ws()).unwrap();
        assert!(out.contains("Health: ready"));
        assert!(out.contains("[session active]"));

        let out = exec_swarm_stop(&json!({"name": "tool-crew"}), ws()).unwrap();
        assert!(out.contains("1 active session(s) completed"));

        let out = exec_swarm_delete(&json!({"name": "tool-crew"}), ws()).unwrap();
        assert!(out.contains("deleted"));
        assert!(
            exec_swarm_status(&json!({"name": "tool-crew"}), ws()).is_err(),
            "deleted swarm must not have status"
        );
    }

    #[test]
    fn create_requires_known_template() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        crate::runtime_ctx::set_agent_registry_info(tmp.path(), "RustyClaw");
        assert!(exec_swarm_create(&json!({"template": "nope"}), ws()).is_err());
    }

    #[test]
    fn send_and_delete_require_name_params() {
        assert!(exec_swarm_send(&json!({"message": "x"}), ws()).is_err());
        assert!(exec_swarm_send(&json!({"swarm": "s"}), ws()).is_err());
        assert!(exec_swarm_delete(&json!({}), ws()).is_err());
    }
}

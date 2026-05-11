//! Swarm management tools.
//!
//! Provides agent-callable tools for creating, listing, inspecting, messaging,
//! and stopping swarms.

use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument};

use crate::swarm::{SwarmConfig, SwarmStatus, builtin_templates, swarm_manager};

// ── swarm_create ────────────────────────────────────────────────────────────

/// Create a new swarm from a built-in template or inline JSON config.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_create(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let template_name = args.get("template").and_then(|v| v.as_str());
    let custom_config = args.get("config");

    let config: SwarmConfig = if let Some(cfg_val) = custom_config {
        serde_json::from_value(cfg_val.clone())
            .map_err(|e| format!("Invalid swarm config: {e}"))?
    } else {
        let tpl_name = template_name.unwrap_or("openswarm");
        let templates = builtin_templates();
        templates
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

    let name = config.name.clone();
    debug!(swarm = %name, agents = config.agents.len(), "Creating swarm");

    let manager = swarm_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire swarm manager lock".to_string())?;

    mgr.create(config)?;
    mgr.start(&name)?;

    let inst = mgr.get(&name).ok_or("Swarm vanished after creation")?;

    let agent_list: Vec<String> = inst
        .config
        .agents
        .iter()
        .map(|a| format!("  - {} ({})", a.name, a.role))
        .collect();

    Ok(format!(
        "Swarm '{}' created and started with {} agents:\n{}",
        name,
        inst.config.agents.len(),
        agent_list.join("\n")
    ))
}

// ── swarm_list ──────────────────────────────────────────────────────────────

/// List all swarms and their status.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_list(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let _ = args; // no parameters needed
    let manager = swarm_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire swarm manager lock".to_string())?;

    let swarms = mgr.list();
    if swarms.is_empty() {
        return Ok(
            "No swarms defined. Use swarm_create to create one (template: 'openswarm').".into(),
        );
    }

    let mut output = String::from("Swarms:\n\n");
    for inst in swarms {
        let status_icon = match inst.status {
            SwarmStatus::Running => "🔄",
            SwarmStatus::Idle => "⏸",
            SwarmStatus::Paused => "⏯",
            SwarmStatus::Stopped => "⏹",
            SwarmStatus::Error => "❌",
        };
        output.push_str(&format!(
            "{} {} — {} agents, {} tasks, {}s uptime\n   {}\n",
            status_icon,
            inst.config.name,
            inst.config.agents.len(),
            inst.tasks_routed,
            inst.runtime_secs(),
            inst.config.description,
        ));
    }

    Ok(output)
}

// ── swarm_status ────────────────────────────────────────────────────────────

/// Get detailed status for a named swarm.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_status(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    let manager = swarm_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire swarm manager lock".to_string())?;

    let inst = mgr
        .get(name)
        .ok_or_else(|| format!("Swarm '{name}' not found"))?;

    let mut output = format!(
        "Swarm: {}\nStatus: {}\nDescription: {}\nAgents: {}\nTasks Routed: {}\nUptime: {}s\n\n",
        inst.config.name,
        inst.status,
        inst.config.description,
        inst.config.agents.len(),
        inst.tasks_routed,
        inst.runtime_secs(),
    );

    output.push_str("Agents:\n");
    for agent in &inst.config.agents {
        let session_status = inst
            .agent_sessions
            .get(&agent.id)
            .map(|s| format!(" [session: {s}]"))
            .unwrap_or_default();
        output.push_str(&format!(
            "  {} ({}) — {}{}\n",
            agent.name, agent.role, agent.description, session_status
        ));
    }

    output.push_str("\nCommunication Flows:\n");
    for flow in &inst.config.flows {
        if flow.kind == crate::swarm::FlowKind::SendMessage {
            output.push_str(&format!("  {} → {} [{}]\n", flow.from, flow.to, flow.kind));
        }
    }
    let handoff_count = inst
        .config
        .flows
        .iter()
        .filter(|f| f.kind == crate::swarm::FlowKind::Handoff)
        .count();
    output.push_str(&format!("  + {} bidirectional Handoff flows\n", handoff_count));

    Ok(output)
}

// ── swarm_send ──────────────────────────────────────────────────────────────

/// Send a task/message to a specific agent within a swarm.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_send(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let swarm_name = args
        .get("swarm")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: swarm".to_string())?;

    let agent_id = args.get("agent").and_then(|v| v.as_str());

    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: message".to_string())?;

    // Default to orchestrator if no agent specified
    let target = agent_id.unwrap_or("orchestrator");

    let manager = swarm_manager();

    // Phase 1: validate and extract info while holding the swarm lock.
    let (agent_name, agent_instructions, existing_session) = {
        let mut mgr = manager
            .lock()
            .map_err(|_| "Failed to acquire swarm manager lock".to_string())?;

        let inst = mgr
            .get_mut(swarm_name)
            .ok_or_else(|| format!("Swarm '{swarm_name}' not found"))?;

        if inst.status != SwarmStatus::Running {
            return Err(format!(
                "Swarm '{swarm_name}' is not running (status: {})",
                inst.status
            ));
        }

        let agent = inst
            .config
            .agents
            .iter()
            .find(|a| a.id == target)
            .ok_or_else(|| {
                let ids: Vec<&str> = inst.config.agents.iter().map(|a| a.id.as_str()).collect();
                format!(
                    "Agent '{target}' not found in swarm '{swarm_name}'. Available: {}",
                    ids.join(", ")
                )
            })?;

        let name = agent.name.clone();
        let instructions = agent.instructions.clone();
        let existing = inst.agent_sessions.get(target).cloned();

        inst.record_task();

        (name, instructions, existing)
    };
    // Swarm lock released here.

    debug!(
        swarm = %swarm_name,
        agent = %target,
        message_len = message.len(),
        "Routing message to swarm agent"
    );

    // Phase 2: interact with the session manager (no swarm lock held).
    let session_mgr = crate::sessions::session_manager();
    let mut sess_mgr = session_mgr
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let session_key = if let Some(existing) = existing_session {
        sess_mgr.send_message(&existing, message)?;
        existing
    } else {
        let label = format!("swarm:{}:{}", swarm_name, target);
        let task = format!(
            "[Swarm: {} | Agent: {}]\n\n{}\n\nSystem Instructions:\n{}",
            swarm_name, agent_name, message, agent_instructions
        );
        let key = sess_mgr.spawn_subagent(target, &task, Some(label), None);
        drop(sess_mgr);

        // Phase 3: store session key back in the swarm manager.
        let mut mgr = manager
            .lock()
            .map_err(|_| "Failed to re-acquire swarm manager lock".to_string())?;
        if let Some(inst) = mgr.get_mut(swarm_name) {
            inst.agent_sessions.insert(target.to_string(), key.clone());
        }
        key
    };

    Ok(format!(
        "Message routed to {} ({}) in swarm '{}'. Session: {}",
        agent_name, target, swarm_name, session_key
    ))
}

// ── swarm_stop ──────────────────────────────────────────────────────────────

/// Stop a running swarm and clean up its agent sessions.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_stop(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    debug!(swarm = %name, "Stopping swarm");

    let manager = swarm_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire swarm manager lock".to_string())?;

    // Complete any active sessions
    let session_keys: Vec<String> = mgr
        .get(name)
        .map(|inst| inst.agent_sessions.values().cloned().collect())
        .unwrap_or_default();

    let session_mgr = crate::sessions::session_manager();
    if let Ok(mut sess_mgr) = session_mgr.lock() {
        for key in &session_keys {
            let _ = sess_mgr.complete_session(key);
        }
    }

    mgr.stop(name)?;

    Ok(format!(
        "Swarm '{}' stopped. {} agent sessions completed.",
        name,
        session_keys.len()
    ))
}

// ── swarm_templates ─────────────────────────────────────────────────────────

/// List available swarm templates.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_swarm_templates(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
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

//! Session tools: sessions_list, sessions_spawn, sessions_send, sessions_history, session_status, agents_list.

use crate::tools::error::{ToolError, ToolResult};
use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument};

/// List sessions.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_sessions_list(args: &Value, _workspace_dir: &Path) -> ToolResult {
    use crate::sessions::*;

    let manager = session_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    debug!(limit, "Listing sessions");

    let sessions = mgr.list(None, false, limit);

    if sessions.is_empty() {
        return Ok("No active sessions.".to_string());
    }

    let mut output = String::from("Sessions:\n\n");
    for session in sessions {
        let kind = match session.kind {
            SessionKind::Main => "main",
            SessionKind::Subagent => "subagent",
            SessionKind::Cron => "cron",
        };
        let status = match session.status {
            SessionStatus::Active => "🔄",
            SessionStatus::Completed => "✅",
            SessionStatus::Error => "❌",
            SessionStatus::Timeout => "⏱",
            SessionStatus::Stopped => "⏹",
        };
        let label = session.label.as_deref().unwrap_or("");
        let runtime = session.runtime_secs();

        output.push_str(&format!(
            "{} [{}] {} — {}s{}\n",
            status,
            kind,
            session.key,
            runtime,
            if label.is_empty() {
                String::new()
            } else {
                format!(" ({})", label)
            }
        ));
    }

    Ok(output)
}

/// Spawn a sub-agent.
#[instrument(skip(args, _workspace_dir), fields(task, agent_id))]
pub fn exec_sessions_spawn(args: &Value, _workspace_dir: &Path) -> ToolResult {
    use crate::sessions::*;

    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: task".to_string())?;

    let label = args.get("label").and_then(|v| v.as_str()).map(String::from);
    let agent_id = args
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("main");

    tracing::Span::current().record("task", &task[..task.len().min(50)]);
    tracing::Span::current().record("agent_id", agent_id);
    debug!(label = label.as_deref(), "Spawning sub-agent");

    // When the installation-wide registry is available, reject unknown agent
    // ids up front instead of silently spawning under a phantom agent.
    if let Some(registry) = crate::runtime_ctx::get_agent_registry() {
        if !registry.exists(agent_id) {
            return Err(format!(
                "Unknown agent id '{}'. Use agents_list to see registered agents \
                 or agents_create to create one.",
                agent_id
            )
            .into());
        }
    }

    // Nothing here can run an agent turn — core has no model client. Find the
    // runner before writing a record, so a spawn that cannot execute is an
    // honest refusal instead of a session that sits at Active forever.
    let runner = crate::sessions::spawn_runner().ok_or_else(|| {
        "Background sub-agents are unavailable: no gateway is hosting this tool. \
         Do the work in this session, or use subagent_run for a focused \
         synchronous run."
            .to_string()
    })?;

    let (session_key, run_id) = {
        let manager = session_manager();
        let mut mgr = manager
            .lock()
            .map_err(|_| "Failed to acquire session manager lock".to_string())?;

        // Record who asked, both so the fan-out ceiling below has something to
        // count and so a sub-agent's children are attributed to it rather than
        // looking like top-level work.
        let parent = crate::tool_caller::current();
        if let Some(parent_key) = parent.as_deref() {
            let running = mgr.active_subagents_of(parent_key);
            crate::tool_limits::check_subagent(running)
                .map_err(|e| crate::tools::error::ToolError::msg(e.to_string()))?;
        }

        let key = mgr.spawn_subagent(agent_id, task, label.clone(), parent.clone());
        debug!(session_key = %key, parent = ?parent, "Sub-agent spawned");

        let run_id = mgr
            .get(&key)
            .and_then(|s| s.run_id.clone())
            .unwrap_or_default();
        if let Some(session) = mgr.get_mut(&key) {
            session.add_message("user", task);
        }
        (key, run_id)
        // Lock released before starting the run: the runner records progress
        // into this same manager, and holding it across the start would
        // deadlock the moment it did.
    };

    if let Err(e) = runner.start(SpawnRequest {
        session_key: session_key.clone(),
        agent_id: agent_id.to_string(),
        task: task.to_string(),
        model: args.get("model").and_then(|v| v.as_str()).map(String::from),
        timeout_secs: args
            .get("runTimeoutSeconds")
            .and_then(|v| v.as_u64())
            .filter(|s| *s > 0),
    }) {
        // Nothing is running, so the record must not claim otherwise.
        if let Ok(mut mgr) = session_manager().lock() {
            if let Some(session) = mgr.get_mut(&session_key) {
                session.add_message("system", &format!("failed to start: {e}"));
                session.error();
            }
        }
        return Err(format!("Failed to start sub-agent: {e}").into());
    }

    // Only now, with a run genuinely detached, is this a background session.
    // `sessions_kill` needs the distinction: an Active record with no live
    // task means a dead run to clear here, but ongoing work it cannot touch
    // for a synchronous `subagent_run`.
    if let Ok(mut mgr) = session_manager().lock() {
        if let Some(session) = mgr.get_mut(&session_key) {
            session.background = true;
        }
    }

    let result = SpawnResult {
        status: "running".to_string(),
        run_id,
        session_key: session_key.clone(),
        message: format!(
            "Sub-agent running in the background on: '{}'. Check on it with \
             sessions_history or session_status (key '{}'), and stop it with \
             sessions_kill.",
            task, session_key
        ),
    };

    serde_json::to_string_pretty(&result)
        .map_err(|e| ToolError::context("Failed to serialize result", e))
}

/// Stop a running spawned sub-agent.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_sessions_kill(args: &Value, _workspace_dir: &Path) -> ToolResult {
    use crate::sessions::*;

    let target = args
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .map(String::from);
    let label = args.get("label").and_then(|v| v.as_str());

    let manager = session_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let key = match (target, label) {
        (Some(key), _) => key,
        (None, Some(label)) => mgr
            .get_by_label(label)
            .map(|s| s.key.clone())
            .ok_or_else(|| format!("No session found with label '{label}'"))?,
        (None, None) => {
            return Err("Provide either sessionKey or label".to_string().into());
        }
    };

    let Some(session) = mgr.get(&key) else {
        return Err(format!("No session found: {key}").into());
    };

    // Same rule as backgrounded processes: a session belongs to whoever
    // spawned it, and another conversation must not be able to reach in and
    // stop it. An unowned session predates ownership tracking and stays
    // stoppable by anyone.
    if let Some(owner) = session.parent_key.clone() {
        let caller = crate::tool_caller::current();
        if caller.as_deref() != Some(owner.as_str()) {
            return Err(format!("No session found: {key}").into());
        }
    }

    // Only a background run can be stopped, and saying so beats a comfortable
    // lie. A synchronous `subagent_run` files a record in this same manager
    // and passes the ownership check above, but nothing here can interrupt it:
    // marking the record Stopped would report success while the work carried
    // on and later overwrote the status with Completed. A main session is not
    // a task at all.
    if session.kind != SessionKind::Subagent {
        return Err(format!(
            "Session {key} is not a background sub-agent. sessions_kill stops \
             runs started by sessions_spawn."
        )
        .into());
    }

    let status = session.status.clone();
    let background = session.background;
    let attached = running_sessions()
        .lock()
        .map(|runs| runs.is_running(&key))
        .unwrap_or(false);

    // Active with no live task means one of two very different things, and
    // `background` is what tells them apart.
    //
    // For a synchronous `subagent_run` the work is still going inside its
    // caller and nothing here can interrupt it, so refuse rather than claim a
    // stop that did not happen.
    //
    // For a background run it means the task died without recording its
    // ending — a panic, say. That record would otherwise sit Active forever,
    // showing as a phantom running job and permanently consuming one of the
    // caller's `max_subagents` slots, with no way to clear it. Clearing it is
    // exactly what this tool should do; it falls through to the loop below.
    if status == SessionStatus::Active && !attached && !background {
        return Err(format!(
            "No background run is attached to session {key}, so there is \
             nothing to stop. A synchronous subagent_run cannot be stopped \
             this way — it ends when it returns to the agent that called it."
        )
        .into());
    }
    let orphaned = status == SessionStatus::Active && !attached;

    // The whole subtree, not just this run. A spawned run can spawn its own,
    // and those children are owned by the identity *it* ran as — an identity
    // nothing can present once its task is aborted. Left behind they would
    // keep calling the model with no way for anyone to stop them, which is
    // precisely what this tool exists to prevent.
    let mut targets = vec![key.clone()];
    targets.extend(mgr.descendants_of(&key));

    let mut stopped = Vec::new();
    for target in &targets {
        if mgr.get(target).map(|s| s.status.clone()) != Some(SessionStatus::Active) {
            continue;
        }
        // Stop the run first, then record it: if the abort landed and the
        // record still said Active, the parent would poll a corpse forever.
        //
        // A descendant with no registered task is still marked. Unlike the
        // target — which is refused above — it genuinely is finished: killing
        // the ancestor tears down the task its synchronous children were
        // running inside.
        //
        // The registry lock is taken while the session-manager lock is held.
        // Safe because no path holds the registry and then reaches for the
        // session manager, so the two cannot deadlock against each other.
        running_sessions()
            .lock()
            .map(|mut runs| runs.stop(target))
            .ok();
        // Already looked up above, so the only failure mode is a race that
        // removed it; either way the caller's answer is the same.
        mgr.stop_session(target).ok();
        if let Some(session) = mgr.get_mut(target) {
            let note = if orphaned && target == &key {
                "cleared by sessions_kill: the run ended without recording an outcome"
            } else {
                "stopped by sessions_kill"
            };
            session.add_message("system", note);
        }
        stopped.push(target.clone());
    }

    if stopped.is_empty() {
        return Ok(format!(
            "Session {key} is already {status:?}; nothing to stop."
        ));
    }

    let target_stopped = stopped.first() == Some(&key);
    let descendants = stopped.len() - usize::from(target_stopped);
    // Say which of the two happened. "Stopped" would be a small lie for a run
    // that had already died on its own.
    let verb = if orphaned { "Cleared" } else { "Stopped" };
    let tail = if orphaned {
        " Its task had already ended without recording an outcome."
    } else {
        ""
    };
    Ok(match (target_stopped, descendants) {
        (true, 0) => format!("{verb} session {key}.{tail}"),
        (true, n) => {
            format!("{verb} session {key} and {n} sub-agent(s) it had started.{tail}")
        }
        // The run itself had already ended, but it left children behind.
        (false, n) => format!(
            "Session {key} was already {status:?}; stopped {n} sub-agent(s) it had left running."
        ),
    })
}

/// Send a message to a session.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_sessions_send(args: &Value, _workspace_dir: &Path) -> ToolResult {
    use crate::sessions::*;

    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: message".to_string())?;

    let session_key = args.get("sessionKey").and_then(|v| v.as_str());
    let label = args.get("label").and_then(|v| v.as_str());

    debug!(
        session_key,
        label,
        message_len = message.len(),
        "Sending message to session"
    );

    let manager = session_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    // Find session by key or label
    let key = if let Some(k) = session_key {
        k.to_string()
    } else if let Some(l) = label {
        mgr.get_by_label(l)
            .map(|s| s.key.clone())
            .ok_or_else(|| format!("No session found with label: {}", l))?
    } else {
        return Err("Must provide sessionKey or label".into());
    };

    mgr.send_message(&key, message)?;

    Ok(format!("Message sent to session: {}", key))
}

/// Get session history.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_sessions_history(args: &Value, _workspace_dir: &Path) -> ToolResult {
    use crate::sessions::*;

    let session_key = args
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: sessionKey".to_string())?;

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    let include_tools = args
        .get("includeTools")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    debug!(
        session_key,
        limit, include_tools, "Fetching session history"
    );

    let manager = session_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let history = mgr
        .history(session_key, limit, include_tools)
        .ok_or_else(|| format!("Session not found: {}", session_key))?;

    if history.is_empty() {
        return Ok(format!("No messages in session: {}", session_key));
    }

    let mut output = format!("History for {}:\n\n", session_key);
    for msg in history {
        output.push_str(&format!("[{}] {}\n", msg.role, msg.content));
    }

    Ok(output)
}

/// Get session status.
#[instrument(skip(args, _workspace_dir))]
pub fn exec_session_status(args: &Value, _workspace_dir: &Path) -> ToolResult {
    use crate::sessions::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Get current time info
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let session_key = args.get("sessionKey").and_then(|v| v.as_str());
    debug!(session_key, "Getting session status");

    let manager = session_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let mut output = String::from("📊 Session Status\n\n");

    // Include model information from runtime context
    if let Some((provider, model, base_url)) = crate::runtime_ctx::get_model_info() {
        output.push_str("## Model\n");
        output.push_str(&format!("Provider: {}\n", provider));
        output.push_str(&format!("Model: {}\n", model));
        output.push_str(&format!("Base URL: {}\n\n", base_url));
    }

    if let Some(key) = session_key {
        if let Some(session) = mgr.get(key) {
            output.push_str(&format!("Session: {}\n", session.key));
            output.push_str(&format!("Agent: {}\n", session.agent_id));
            output.push_str(&format!("Kind: {:?}\n", session.kind));
            output.push_str(&format!("Status: {:?}\n", session.status));
            output.push_str(&format!("Runtime: {}s\n", session.runtime_secs()));
            output.push_str(&format!("Messages: {}\n", session.messages.len()));
        } else {
            return Err(format!("Session not found: {}", key).into());
        }
    } else {
        // Show general status
        let all_sessions = mgr.list(None, false, 100);
        let active = all_sessions
            .iter()
            .filter(|s| s.status == SessionStatus::Active)
            .count();

        output.push_str("## Sessions\n");
        output.push_str(&format!("Active sessions: {}\n", active));
        output.push_str(&format!("Total sessions: {}\n", all_sessions.len()));
        output.push_str(&format!("Timestamp: {} ms\n", now.as_millis()));
    }

    Ok(output)
}

/// List available agent IDs.
#[instrument(skip(_args, workspace_dir))]
pub fn exec_agents_list(_args: &Value, workspace_dir: &Path) -> ToolResult {
    debug!("Listing available agents");

    // Prefer the installation-wide agent registry when the gateway has
    // published its settings dir.
    if let Some(registry) = crate::runtime_ctx::get_agent_registry() {
        let mut output = String::from(
            "Registered agents (usable with sessions_spawn agentId, agents_create to add more):\n\n",
        );
        for agent in registry.list() {
            output.push_str(&format!("- {} ({})", agent.id, agent.name));
            if let Some(desc) = &agent.description {
                output.push_str(&format!(" — {}", desc));
            }
            output.push('\n');
        }
        return Ok(output);
    }

    // Fallback: legacy scan of `<workspace>/agents/*` directories.
    let mut agents = vec!["main".to_string()];
    let agents_dir = workspace_dir.join("agents");
    if agents_dir.exists() && agents_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !name.starts_with('.') && name != "main" {
                            agents.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    let mut output = String::from("Available agents for sessions_spawn:\n\n");
    for agent in &agents {
        output.push_str(&format!("- {}\n", agent));
    }

    Ok(output)
}

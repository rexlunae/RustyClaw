//! Session tools: sessions_list, sessions_spawn, sessions_send, sessions_history, session_status, agents_list.

use serde_json::Value;
use std::path::Path;

/// List sessions.
pub fn exec_sessions_list(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;

    let manager = session_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    let include_archived = args
        .get("includeArchived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let retention_days = args
        .get("retentionDays")
        .and_then(|v| v.as_u64());

    let _ = archive_completed_sessions(&mut mgr, _workspace_dir);
    if let Some(days) = retention_days {
        let _ = prune_archived_sessions(_workspace_dir, days);
    }

    let sessions = mgr.list(None, false, limit);
    let archived = if include_archived {
        list_archived_sessions(_workspace_dir, limit).unwrap_or_default()
    } else {
        Vec::new()
    };

    if sessions.is_empty() && archived.is_empty() {
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
    for session in &archived {
        let kind = match session.kind {
            SessionKind::Main => "main",
            SessionKind::Subagent => "subagent",
            SessionKind::Cron => "cron",
        };
        let label = session.label.as_deref().unwrap_or("");
        output.push_str(&format!(
            "📦 [{}] {} — archived{}\n",
            kind,
            session.key,
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
pub fn exec_sessions_spawn(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
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

    let manager = session_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let session_key = mgr.spawn_subagent(agent_id, task, label.clone(), None);

    // Get the run_id
    let run_id = mgr
        .get(&session_key)
        .and_then(|s| s.run_id.clone())
        .unwrap_or_default();

    let result = SpawnResult {
        status: "accepted".to_string(),
        run_id: run_id.clone(),
        session_key: session_key.clone(),
        message: format!(
            "Sub-agent spawned. Task: '{}'. Use sessions_history or sessions_send to interact.",
            task
        ),
    };

    serde_json::to_string_pretty(&result)
        .map_err(|e| format!("Failed to serialize result: {}", e))
}

/// Send a message to a session.
pub fn exec_sessions_send(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;

    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: message".to_string())?;

    let session_key = args.get("sessionKey").and_then(|v| v.as_str());
    let label = args.get("label").and_then(|v| v.as_str());

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
        return Err("Must provide sessionKey or label".to_string());
    };

    mgr.send_message(&key, message)?;

    Ok(format!("Message sent to session: {}", key))
}

/// Get session history.
pub fn exec_sessions_history(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;

    let session_key = args
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: sessionKey".to_string())?;

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let include_tools = args
        .get("includeTools")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let include_archived = args
        .get("includeArchived")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let manager = session_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;
    if let Some(history) = mgr.history(session_key, limit, include_tools) {
        if history.is_empty() {
            return Ok(format!("No messages in session: {}", session_key));
        }
        let mut output = format!("History for {}:\n\n", session_key);
        for msg in history {
            output.push_str(&format!("[{}] {}\n", msg.role, msg.content));
        }
        return Ok(output);
    }
    drop(mgr);

    if include_archived {
        if let Some(session) = get_archived_session(_workspace_dir, session_key)? {
            if session.messages.is_empty() {
                return Ok(format!("No messages in archived session: {}", session_key));
            }
            let mut output = format!("History for {} (archived):\n\n", session_key);
            let filtered: Vec<_> = session
                .messages
                .iter()
                .filter(|m| include_tools || m.role != "tool")
                .collect();
            let start = filtered.len().saturating_sub(limit);
            for msg in &filtered[start..] {
                output.push_str(&format!("[{}] {}\n", msg.role, msg.content));
            }
            return Ok(output);
        }
    }

    Err(format!("Session not found: {}", session_key))
}

/// Get session status.
pub fn exec_session_status(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Get current time info
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let session_key = args.get("sessionKey").and_then(|v| v.as_str());
    let archive = args
        .get("archive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let archive_completed = args
        .get("archiveCompleted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let include_archived = args
        .get("includeArchived")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let retention_days = args
        .get("retentionDays")
        .and_then(|v| v.as_u64());

    let manager = session_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let mut output = String::from("📊 Session Status\n\n");

    if archive_completed {
        let count = archive_completed_sessions(&mut mgr, _workspace_dir)?;
        output.push_str(&format!("Archived completed sessions: {}\n", count));
    }
    if let Some(days) = retention_days {
        let deleted = prune_archived_sessions(_workspace_dir, days)?;
        output.push_str(&format!("Pruned archived sessions: {}\n", deleted));
    }
    if archive {
        let key = session_key.ok_or("archive=true requires sessionKey")?;
        archive_session(&mut mgr, key, _workspace_dir)?;
        output.push_str(&format!("Archived session: {}\n", key));
        return Ok(output);
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
            drop(mgr);
            if include_archived {
                if let Some(session) = get_archived_session(_workspace_dir, key)? {
                    output.push_str(&format!("Session: {}\n", session.key));
                    output.push_str(&format!("Agent: {}\n", session.agent_id));
                    output.push_str(&format!("Kind: {:?}\n", session.kind));
                    output.push_str(&format!("Status: {:?}\n", session.status));
                    output.push_str("Archived: true\n");
                    output.push_str(&format!("Runtime: {}s\n", session.runtime_secs()));
                    output.push_str(&format!("Messages: {}\n", session.messages.len()));
                    return Ok(output);
                }
            }
            return Err(format!("Session not found: {}", key));
        }
    } else {
        // Show general status
        let all_sessions = mgr.list(None, false, 100);
        let archived_count = if include_archived {
            list_archived_sessions(_workspace_dir, usize::MAX)
                .map(|v| v.len())
                .unwrap_or(0)
        } else {
            0
        };
        let active = all_sessions
            .iter()
            .filter(|s| s.status == SessionStatus::Active)
            .count();

        output.push_str(&format!("Active sessions: {}\n", active));
        output.push_str(&format!("Total sessions: {}\n", all_sessions.len()));
        if include_archived {
            output.push_str(&format!("Archived sessions: {}\n", archived_count));
        }
        output.push_str(&format!("Timestamp: {} ms\n", now.as_millis()));
    }

    Ok(output)
}

/// List available agent IDs.
pub fn exec_agents_list(_args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let mut agents = vec!["main".to_string()];

    // Check for agents directory
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

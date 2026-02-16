//! Runtime tools: execute_command and process management.

use super::helpers::{
    command_references_credentials, is_protected_path, prepare_sandboxed_spawn, process_manager,
    resolve_path, run_sandboxed_command, VAULT_ACCESS_DENIED,
};
use crate::process_manager::SessionStatus;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Execute a shell command with background support and optional sandboxing.
pub fn exec_execute_command(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let mut command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: command".to_string())?
        .to_string();
    let working_dir = args.get("working_dir").and_then(|v| v.as_str());
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);

    // Background execution support
    let background = args
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let yield_ms = args
        .get("yieldMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(10000); // Default 10 seconds before auto-background

    // Elevated (sudo) mode support
    let elevated_mode = args
        .get("elevated_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Prepend sudo if elevated mode is enabled
    if elevated_mode {
        command = format!("sudo {}", command);
    }

    let cwd = match working_dir {
        Some(p) => resolve_path(workspace_dir, p),
        None => workspace_dir.to_path_buf(),
    };

    // Block commands that reference the credentials directory.
    if command_references_credentials(&command) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }
    if is_protected_path(&cwd) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

    // If background requested immediately, spawn and return session ID
    // Apply sandbox wrapper before spawning (Bubblewrap/macOS only; Landlock is process-wide)
    if background {
        let (sandbox_cmd, sandbox_args) = prepare_sandboxed_spawn(&command, &cwd);

        // Construct the wrapped command string for process manager
        let mut wrapped_command = sandbox_cmd.clone();
        for arg in &sandbox_args {
            wrapped_command.push(' ');
            // Quote arguments that contain spaces
            if arg.contains(' ') {
                wrapped_command.push('"');
                wrapped_command.push_str(arg);
                wrapped_command.push('"');
            } else {
                wrapped_command.push_str(arg);
            }
        }

        let manager = process_manager();
        let mut mgr = manager
            .lock()
            .map_err(|_| "Failed to acquire process manager lock".to_string())?;

        let session_id = mgr.spawn(&wrapped_command, cwd.to_string_lossy().as_ref(), Some(timeout_secs))?;

        return Ok(json!({
            "status": "running",
            "sessionId": session_id,
            "message": format!("Command backgrounded (sandboxed). Use process tool to poll session '{}'.", session_id)
        })
        .to_string());
    }

    // For foreground execution with no yield (immediate), use sandbox
    if yield_ms == 0 {
        let output = run_sandboxed_command(&command, &cwd)?;
        return format_output(output, timeout_secs);
    }

    // For commands with yield support, we need to spawn directly so we can
    // transfer the child to the process manager if it takes too long.
    // Apply sandbox wrapper before spawning (Bubblewrap/macOS only; Landlock is process-wide)
    let (sandbox_cmd, sandbox_args) = prepare_sandboxed_spawn(&command, &cwd);

    let mut child = std::process::Command::new(&sandbox_cmd)
        .args(&sandbox_args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    // Poll for completion with yield/timeout logic
    let yield_deadline = Instant::now() + Duration::from_millis(yield_ms);
    let timeout_deadline = Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break, // Process finished
            Ok(None) => {
                let now = Instant::now();

                // Check if we should auto-background
                if now >= yield_deadline {
                    // Move to background - transfer child to process manager
                    let manager = process_manager();
                    let mut mgr = manager
                        .lock()
                        .map_err(|_| "Failed to acquire process manager lock".to_string())?;

                    // Create a session from the existing child
                    let remaining_timeout = timeout_deadline.saturating_duration_since(now);
                    let mut session = crate::process_manager::ExecSession::new(
                        command.to_string(),
                        cwd.to_string_lossy().to_string(),
                        Some(remaining_timeout),
                        child,
                    );

                    // Try to read any output accumulated so far
                    session.try_read_output();

                    // Insert session into manager
                    let session_id = mgr.insert(session);

                    return Ok(json!({
                        "status": "running",
                        "sessionId": session_id,
                        "message": format!(
                            "Command still running after {}ms, backgrounded as session '{}'. \
                             Use process tool to poll.",
                            yield_ms, session_id
                        )
                    })
                    .to_string());
                }

                // Check timeout
                if now >= timeout_deadline {
                    let _ = child.kill();
                    return Err(format!("Command timed out after {} seconds", timeout_secs));
                }

                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Error waiting for command: {}", e)),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to get command output: {}", e))?;

    format_output(output, timeout_secs)
}

/// Format command output into a result string.
fn format_output(output: std::process::Output, _timeout_secs: u64) -> Result<String, String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str("[stderr]\n");
        result.push_str(&stderr);
    }

    if !output.status.success() {
        let exit = output.status.code().unwrap_or(-1);
        result.push_str(&format!("\n[exit code: {}]", exit));
    }

    // Truncate very long output.
    if result.len() > 50_000 {
        result.truncate(50_000);
        result.push_str("\n\n[output truncated at 50KB]");
    }

    if result.is_empty() {
        result = "(no output)".to_string();
    }

    Ok(result)
}

/// Manage background exec sessions.
pub fn exec_process(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    let session_id = args.get("sessionId").and_then(|v| v.as_str());

    let manager = process_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire process manager lock".to_string())?;

    match action {
        "list" => {
            // Poll all sessions first to update status
            mgr.poll_all();

            let sessions = mgr.list();
            if sessions.is_empty() {
                return Ok("No active sessions.".to_string());
            }

            let mut output = String::from("Background sessions:\n\n");
            for session in sessions {
                let status_str = match &session.status {
                    SessionStatus::Running => "running".to_string(),
                    SessionStatus::Exited(code) => format!("exited ({})", code),
                    SessionStatus::Killed => "killed".to_string(),
                    SessionStatus::TimedOut => "timed out".to_string(),
                };
                let elapsed = session.elapsed().as_secs();
                output.push_str(&format!(
                    "- {} [{}] ({}s)\n  {}\n",
                    session.id, status_str, elapsed, session.command
                ));
            }
            Ok(output)
        }

        "poll" => {
            let id = session_id.ok_or("Missing sessionId for poll action")?;

            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            // Try to read new output and check exit status
            session.try_read_output();
            let exited = session.check_exit();

            let new_output = session.poll_output().to_string();
            let status_str = match &session.status {
                SessionStatus::Running => "running".to_string(),
                SessionStatus::Exited(code) => format!("exited ({})", code),
                SessionStatus::Killed => "killed".to_string(),
                SessionStatus::TimedOut => "timed out".to_string(),
            };

            let mut result = String::new();
            if !new_output.is_empty() {
                result.push_str(&new_output);
                if !new_output.ends_with('\n') {
                    result.push('\n');
                }
                result.push('\n');
            }

            if exited {
                result.push_str(&format!("Process {}.", status_str));
            } else {
                result.push_str(&format!("Process still {}.", status_str));
            }

            Ok(result)
        }

        "log" => {
            let id = session_id.ok_or("Missing sessionId for log action")?;

            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            // Update output first
            session.try_read_output();

            let offset = args
                .get("offset")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .or(Some(50));

            let output = session.log_output(offset, limit);
            if output.is_empty() {
                Ok("(no output)".to_string())
            } else {
                Ok(output)
            }
        }

        "write" => {
            let id = session_id.ok_or("Missing sessionId for write action")?;
            let data = args
                .get("data")
                .and_then(|v| v.as_str())
                .ok_or("Missing data for write action")?;

            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            session.write_stdin(data)?;
            Ok(format!("Wrote {} bytes to session {}", data.len(), id))
        }

        "kill" => {
            let id = session_id.ok_or("Missing sessionId for kill action")?;

            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            session.kill()?;
            Ok(format!("Killed session {}", id))
        }

        "clear" => {
            mgr.clear_completed();
            Ok("Cleared completed sessions.".to_string())
        }

        "remove" => {
            let id = session_id.ok_or("Missing sessionId for remove action")?;

            if let Some(mut session) = mgr.remove(id) {
                // Kill if still running
                if session.status == SessionStatus::Running {
                    let _ = session.kill();
                }
                Ok(format!("Removed session {}", id))
            } else {
                Err(format!("No session found: {}", id))
            }
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: list, poll, log, write, kill, clear, remove",
            action
        )),
    }
}

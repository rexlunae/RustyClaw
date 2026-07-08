//! Runtime tools: execute_command and process management.
//!
//! These tools use async I/O for process spawning and management.

use super::helpers::{
    VAULT_ACCESS_DENIED, command_references_credentials, is_protected_path, process_manager,
    resolve_path, run_sandboxed_command, validate_command_safe,
};
use crate::process_manager::SessionStatus;
use crate::tools::error::{ToolError, ToolResult};
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tracing::{debug, instrument, warn};

/// Execute a shell command with background support and optional sandboxing,
/// streaming stdout/stderr chunks into `sink` (when given) as the process
/// produces them, so clients can show live progress for long commands.
#[instrument(skip(args, workspace_dir, sink), fields(command))]
pub async fn exec_execute_command_streaming(
    args: &Value,
    workspace_dir: &Path,
    sink: Option<super::ToolOutputSink>,
) -> ToolResult {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: command".to_string())?;

    tracing::Span::current().record("command", &command[..command.len().min(100)]);

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

    let cwd = match working_dir {
        Some(p) => resolve_path(workspace_dir, p),
        None => workspace_dir.to_path_buf(),
    };

    debug!(cwd = %cwd.display(), timeout_secs, background, yield_ms, "Executing command");

    // Block commands that reference the credentials directory.
    // Validate command safety (null byte rejection, length, exfiltration patterns).
    validate_command_safe(command)?;

    if command_references_credentials(command) {
        warn!("Command references credentials directory");
        return Err(VAULT_ACCESS_DENIED.into());
    }
    if is_protected_path(&cwd) {
        warn!(cwd = %cwd.display(), "Working directory is protected");
        return Err(VAULT_ACCESS_DENIED.into());
    }

    // If background requested immediately, spawn and return session ID
    if background {
        debug!("Spawning background process");
        let manager = process_manager();
        let mut mgr = manager
            .lock()
            .map_err(|_| "Failed to acquire process manager lock".to_string())?;

        let session_id = mgr.spawn(command, cwd.to_string_lossy().as_ref(), Some(timeout_secs))?;
        debug!(session_id = %session_id, "Background process spawned");

        return Ok(json!({
            "status": "running",
            "sessionId": session_id,
            "message": format!("Command backgrounded. Use process tool to poll session '{}'.", session_id)
        })
        .to_string());
    }

    // For foreground execution with no yield (immediate), use sandbox
    if yield_ms == 0 {
        // run_sandboxed_command is still sync - run on blocking pool
        let cmd = command.to_string();
        let cwd_clone = cwd.clone();
        let output = tokio::task::spawn_blocking(move || run_sandboxed_command(&cmd, &cwd_clone))
            .await
            .map_err(|e| ToolError::context("Task join error", e))??;

        return format_output(output, timeout_secs);
    }

    // For commands with yield support, use tokio::process
    #[cfg(unix)]
    let mut child = {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Lead a fresh process group so pause/stop/kill signals sent
            // via the exec-status registry reach the shell's children too.
            .process_group(0);
        cmd.spawn()
            .map_err(|e| ToolError::context("Failed to execute command", e))?
    };

    #[cfg(windows)]
    let mut child = Command::new("cmd")
        .arg("/C")
        .arg(command)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ToolError::context("Failed to execute command", e))?;

    #[cfg(not(any(unix, windows)))]
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ToolError::context("Failed to execute command", e))?;

    // Register the child for live status sampling and user control
    // (pause/stop/kill); the guard deregisters it when the wait ends.
    let child_pid = child.id();
    let _exec_guard = child_pid.map(|pid| crate::exec_status::register(pid, command));

    let mut yield_deadline = Instant::now() + Duration::from_millis(yield_ms);
    let mut timeout_deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut last_poll = Instant::now();

    // Read the pipes incrementally instead of buffering with
    // wait_with_output(): the readers accumulate the full output for the
    // final result AND forward each chunk to `sink` while the process
    // runs, which is what lets clients render live progress.
    let out_task = tokio::spawn(pump_stream(child.stdout.take(), sink.clone(), false));
    let err_task = tokio::spawn(pump_stream(child.stderr.take(), sink, true));

    // Wait for completion, timeout, or the auto-background deadline.
    loop {
        tokio::select! {
            // Check if process has output or exited
            result = async {
                // Small sleep to avoid busy-waiting
                tokio::time::sleep(Duration::from_millis(50)).await;
                child.try_wait()
            } => {
                match result {
                    Ok(Some(status)) => {
                        // Process finished — the readers drain to EOF and
                        // return the accumulated bytes.
                        let stdout = out_task.await.unwrap_or_default();
                        let stderr = err_task.await.unwrap_or_default();
                        let output = std::process::Output { status, stdout, stderr };
                        return format_output_async(output, timeout_secs);
                    }
                    Ok(None) => {
                        // Still running - check deadlines
                        let now = Instant::now();

                        // While the user has the process paused, freeze both
                        // deadlines — a paused process must not time out or
                        // auto-background underneath them.
                        if child_pid.is_some_and(crate::exec_status::is_paused) {
                            let frozen = now.saturating_duration_since(last_poll);
                            yield_deadline += frozen;
                            timeout_deadline += frozen;
                        }
                        last_poll = now;

                        // Check if we should auto-background
                        if now >= yield_deadline {
                            debug!(yield_ms, "Auto-backgrounding long-running process");
                            // The accumulated buffers are unused on this
                            // path and dropping the child closes the pipes
                            // anyway — reap the readers now rather than
                            // leaving them to linger until EOF.
                            out_task.abort();
                            err_task.abort();
                            return background_child(child, command, &cwd, timeout_deadline).await;
                        }

                        // Check timeout
                        if now >= timeout_deadline {
                            warn!(timeout_secs, "Command timed out");
                            let _ = child.kill().await;
                            out_task.abort();
                            err_task.abort();
                            return Err(format!("Command timed out after {} seconds", timeout_secs).into());
                        }

                        // Continue loop
                    }
                    Err(e) => {
                        out_task.abort();
                        err_task.abort();
                        return Err(ToolError::context("Error waiting for command", e));
                    }
                }
            }
        }
    }
}

/// Cap on how many bytes of live output are *streamed* per pipe. The full
/// output is still accumulated for the final result (which has its own
/// 50KB cap); this only bounds the frames pushed to clients while running.
const STREAM_CAP_BYTES: usize = 65_536;

/// Drain a child pipe to EOF, accumulating everything and forwarding
/// chunks to the sink (best-effort, capped). Returns the full bytes.
///
/// Chunks are decoded lossily for streaming; a multi-byte character split
/// across reads may render momentarily as a replacement character, but
/// the final result is decoded from the complete buffer.
async fn pump_stream<R>(
    reader: Option<R>,
    sink: Option<super::ToolOutputSink>,
    is_stderr: bool,
) -> Vec<u8>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let mut buf = Vec::new();
    let Some(mut reader) = reader else {
        return buf;
    };
    let mut chunk = [0u8; 4096];
    let mut streamed = 0usize;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(sink) = &sink
                    && streamed < STREAM_CAP_BYTES
                {
                    streamed += n;
                    let _ = sink.send(super::ToolOutputChunk {
                        chunk: String::from_utf8_lossy(&chunk[..n]).into_owned(),
                        is_stderr,
                    });
                }
            }
        }
    }
    buf
}

/// Move a tokio child process to the sync ProcessManager for background execution.
async fn background_child(
    mut child: tokio::process::Child,
    command: &str,
    cwd: &Path,
    timeout_deadline: Instant,
) -> ToolResult {
    // ProcessManager can't adopt a tokio::process::Child, so backgrounding
    // re-spawns the command under the manager and terminates the original.
    // Wasteful (the command restarts from scratch) but functional.
    // TODO: Refactor ProcessManager to support tokio::process::Child

    // Kill the original FIRST: dropping a tokio child does NOT kill it,
    // and killing only after the re-spawn would leave two copies of the
    // command running side by side.
    let _ = child.kill().await;

    let manager = process_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire process manager lock".to_string())?;

    let remaining_timeout = timeout_deadline
        .saturating_duration_since(Instant::now())
        .as_secs();

    // Spawn a new background process (ProcessManager uses std::process internally)
    let session_id = mgr.spawn(
        command,
        cwd.to_string_lossy().as_ref(),
        Some(remaining_timeout.max(1)),
    )?;

    debug!(session_id = %session_id, "Process backgrounded");

    Ok(json!({
        "status": "running",
        "sessionId": session_id,
        "message": format!(
            "Command re-spawned as background session '{}'. Use process tool to poll.",
            session_id
        )
    })
    .to_string())
}

/// Format command output into a result string (for std::process::Output).
fn format_output(output: std::process::Output, _timeout_secs: u64) -> ToolResult {
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

/// Format command output into a result string (for tokio Output).
fn format_output_async(output: std::process::Output, timeout_secs: u64) -> ToolResult {
    // Same format as sync version
    format_output(output, timeout_secs)
}

/// Sync wrapper for backwards compatibility with ToolDef.
/// This calls block_on internally - prefer using exec_execute_command_async directly.
#[instrument(skip(args, workspace_dir), fields(command))]
pub fn exec_execute_command(args: &Value, workspace_dir: &Path) -> ToolResult {
    // We're already in a tokio runtime, so we can use Handle::current()
    // But this is called from spawn_blocking, so we need to be careful.
    // Actually, since execute_tool now uses spawn_blocking, this sync function
    // just does the original sync behavior.

    // For the async path, execute_tool should call exec_execute_command_async directly.
    // This sync version is kept for compatibility but does the old sync implementation.

    exec_execute_command_sync(args, workspace_dir)
}

/// Original sync implementation (for fallback).
fn exec_execute_command_sync(args: &Value, workspace_dir: &Path) -> ToolResult {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: command".to_string())?;

    let working_dir = args.get("working_dir").and_then(|v| v.as_str());
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);

    let background = args
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let yield_ms = args
        .get("yieldMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(10000);

    let cwd = match working_dir {
        Some(p) => resolve_path(workspace_dir, p),
        None => workspace_dir.to_path_buf(),
    };

    // Validate command safety (null byte rejection, length, exfiltration patterns).
    validate_command_safe(command)?;

    if command_references_credentials(command) {
        return Err(VAULT_ACCESS_DENIED.into());
    }
    if is_protected_path(&cwd) {
        return Err(VAULT_ACCESS_DENIED.into());
    }

    if background {
        let manager = process_manager();
        let mut mgr = manager
            .lock()
            .map_err(|_| "Failed to acquire process manager lock".to_string())?;

        let session_id = mgr.spawn(command, cwd.to_string_lossy().as_ref(), Some(timeout_secs))?;

        return Ok(json!({
            "status": "running",
            "sessionId": session_id,
            "message": format!("Command backgrounded. Use process tool to poll session '{}'.", session_id)
        })
        .to_string());
    }

    if yield_ms == 0 {
        let output = run_sandboxed_command(command, &cwd)?;
        return format_output(output, timeout_secs);
    }

    let mut child = {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Lead a fresh process group so pause/stop/kill signals sent via
        // the exec-status registry reach the shell's children too.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn()
            .map_err(|e| ToolError::context("Failed to execute command", e))?
    };

    // Register for live status sampling and user control; the guard
    // deregisters when the wait ends (including the backgrounding path).
    let child_pid = child.id();
    let _exec_guard = crate::exec_status::register(child_pid, command);

    let mut yield_deadline = Instant::now() + Duration::from_millis(yield_ms);
    let mut timeout_deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut last_poll = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                let now = Instant::now();

                // Freeze deadlines while the user has the process paused.
                if crate::exec_status::is_paused(child_pid) {
                    let frozen = now.saturating_duration_since(last_poll);
                    yield_deadline += frozen;
                    timeout_deadline += frozen;
                }
                last_poll = now;

                if now >= yield_deadline {
                    let manager = process_manager();
                    let mut mgr = manager
                        .lock()
                        .map_err(|_| "Failed to acquire process manager lock".to_string())?;

                    let remaining_timeout = timeout_deadline.saturating_duration_since(now);
                    let mut session = crate::process_manager::ExecSession::new(
                        command.to_string(),
                        cwd.to_string_lossy().to_string(),
                        Some(remaining_timeout),
                        child,
                    );

                    session.try_read_output();
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

                if now >= timeout_deadline {
                    let _ = child.kill();
                    return Err(format!("Command timed out after {} seconds", timeout_secs).into());
                }

                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(ToolError::context("Error waiting for command", e)),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| ToolError::context("Failed to get command output", e))?;

    format_output(output, timeout_secs)
}

/// Manage background exec sessions (async version).
#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_process_async(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);

    let session_id = args.get("sessionId").and_then(|v| v.as_str());

    debug!(session_id, "Processing exec session action");

    // ProcessManager still uses std::sync::Mutex, which is fine for quick operations
    let manager = process_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire process manager lock".to_string())?;

    match action {
        "list" => {
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

        "send_keys" | "sendkeys" | "send-keys" => {
            let id = session_id.ok_or("Missing sessionId for send_keys action")?;
            let keys = args
                .get("keys")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'keys' for send_keys action")?;

            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            let bytes_sent = session.send_keys(keys)?;
            Ok(format!(
                "Sent keys [{}] ({} bytes) to session {}",
                keys, bytes_sent, id
            ))
        }

        "kill" => {
            let id = session_id.ok_or("Missing sessionId for kill action")?;

            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            session.kill()?;
            debug!(session_id = id, "Session killed");
            Ok(format!("Killed session {}", id))
        }

        "clear" => {
            mgr.clear_completed();
            debug!("Cleared completed sessions");
            Ok("Cleared completed sessions.".to_string())
        }

        "remove" => {
            let id = session_id.ok_or("Missing sessionId for remove action")?;

            if let Some(mut session) = mgr.remove(id) {
                if session.status == SessionStatus::Running {
                    let _ = session.kill();
                }
                debug!(session_id = id, "Session removed");
                Ok(format!("Removed session {}", id))
            } else {
                Err(format!("No session found: {}", id).into())
            }
        }

        _ => {
            warn!(action, "Unknown process action");
            Err(format!(
                "Unknown action: {}. Valid: list, poll, log, write, send_keys, kill, clear, remove",
                action
            )
            .into())
        }
    }
}

/// Sync wrapper for process tool.
#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_process(args: &Value, _workspace_dir: &Path) -> ToolResult {
    // Same as async version but sync - ProcessManager operations are quick
    exec_process_sync(args, _workspace_dir)
}

fn exec_process_sync(args: &Value, _workspace_dir: &Path) -> ToolResult {
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
        "send_keys" | "sendkeys" | "send-keys" => {
            let id = session_id.ok_or("Missing sessionId for send_keys action")?;
            let keys = args
                .get("keys")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'keys' for send_keys action")?;
            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;
            let bytes_sent = session.send_keys(keys)?;
            Ok(format!(
                "Sent keys [{}] ({} bytes) to session {}",
                keys, bytes_sent, id
            ))
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
                if session.status == SessionStatus::Running {
                    let _ = session.kill();
                }
                Ok(format!("Removed session {}", id))
            } else {
                Err(format!("No session found: {}", id).into())
            }
        }
        _ => Err(format!(
            "Unknown action: {}. Valid: list, poll, log, write, send_keys, kill, clear, remove",
            action
        )
        .into()),
    }
}

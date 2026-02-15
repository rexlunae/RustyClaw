//! Background process session management for RustyClaw.
//!
//! Provides a registry of background exec sessions that can be polled,
//! written to, and killed by the agent.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Unique identifier for a background session.
pub type SessionId = String;

/// Generate a short human-readable session ID.
fn generate_session_id() -> SessionId {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // Simple adjective-noun pattern for readability
    let adjectives = ["warm", "cool", "swift", "calm", "bold", "keen", "bright", "quick"];
    let nouns = ["rook", "hawk", "wolf", "bear", "fox", "owl", "lynx", "crow"];

    let adj_idx = (timestamp % adjectives.len() as u128) as usize;
    let noun_idx = ((timestamp / 8) % nouns.len() as u128) as usize;

    format!("{}-{}", adjectives[adj_idx], nouns[noun_idx])
}

/// Status of a background session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    /// Process is still running.
    Running,
    /// Process exited with the given code.
    Exited(i32),
    /// Process was killed by a signal.
    Killed,
    /// Process timed out and was killed.
    TimedOut,
}

/// A background exec session.
pub struct ExecSession {
    /// Session identifier.
    pub id: SessionId,
    /// The command that was executed.
    pub command: String,
    /// Working directory.
    pub working_dir: String,
    /// When the session started.
    pub started_at: Instant,
    /// Timeout duration (if set).
    pub timeout: Option<Duration>,
    /// Current status.
    pub status: SessionStatus,
    /// Accumulated stdout output.
    stdout_buffer: Vec<u8>,
    /// Accumulated stderr output.
    stderr_buffer: Vec<u8>,
    /// Combined output (interleaved stdout + stderr for display).
    combined_output: String,
    /// Last read position for polling.
    last_read_pos: usize,
    /// The child process handle.
    child: Option<Child>,
    /// Exit code (set when process exits).
    exit_code: Option<i32>,
}

impl ExecSession {
    /// Create a new session for a running process.
    pub fn new(
        command: String,
        working_dir: String,
        timeout: Option<Duration>,
        child: Child,
    ) -> Self {
        Self {
            id: generate_session_id(),
            command,
            working_dir,
            started_at: Instant::now(),
            timeout,
            status: SessionStatus::Running,
            stdout_buffer: Vec::new(),
            stderr_buffer: Vec::new(),
            combined_output: String::new(),
            last_read_pos: 0,
            child: Some(child),
            exit_code: None,
        }
    }

    /// Check if the process has exceeded its timeout.
    pub fn is_timed_out(&self) -> bool {
        if let Some(timeout) = self.timeout {
            self.started_at.elapsed() > timeout
        } else {
            false
        }
    }

    /// Get the elapsed time since the session started.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Append output to the combined buffer.
    pub fn append_output(&mut self, text: &str) {
        self.combined_output.push_str(text);
    }

    /// Get new output since the last poll.
    pub fn poll_output(&mut self) -> &str {
        let new_output = &self.combined_output[self.last_read_pos..];
        self.last_read_pos = self.combined_output.len();
        new_output
    }

    /// Get the full output log.
    pub fn full_output(&self) -> &str {
        &self.combined_output
    }

    /// Get output with line-based offset and limit.
    pub fn log_output(&self, offset: Option<usize>, limit: Option<usize>) -> String {
        let lines: Vec<&str> = self.combined_output.lines().collect();
        let total = lines.len();

        // If offset is None, grab the last `limit` lines
        let (start, end) = match (offset, limit) {
            (None, Some(lim)) => {
                let start = total.saturating_sub(lim);
                (start, total)
            }
            (Some(off), Some(lim)) => {
                let start = off.min(total);
                let end = (start + lim).min(total);
                (start, end)
            }
            (Some(off), None) => {
                let start = off.min(total);
                (start, total)
            }
            (None, None) => (0, total),
        };

        lines[start..end].join("\n")
    }

    /// Try to read any available output from the child process.
    /// Returns true if any output was read.
    pub fn try_read_output(&mut self) -> bool {
        let Some(ref mut child) = self.child else {
            return false;
        };

        let mut read_any = false;

        // Try to read from stdout
        if let Some(ref mut stdout) = child.stdout {
            let mut buf = [0u8; 4096];
            // Non-blocking read attempt
            if let Ok(n) = read_nonblocking(stdout, &mut buf) {
                if n > 0 {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    self.combined_output.push_str(&text);
                    self.stdout_buffer.extend_from_slice(&buf[..n]);
                    read_any = true;
                }
            }
        }

        // Try to read from stderr
        if let Some(ref mut stderr) = child.stderr {
            let mut buf = [0u8; 4096];
            if let Ok(n) = read_nonblocking(stderr, &mut buf) {
                if n > 0 {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    self.combined_output.push_str(&text);
                    self.stderr_buffer.extend_from_slice(&buf[..n]);
                    read_any = true;
                }
            }
        }

        read_any
    }

    /// Check if the process has exited and update status.
    pub fn check_exit(&mut self) -> bool {
        let Some(ref mut child) = self.child else {
            return true; // Already exited
        };

        match child.try_wait() {
            Ok(Some(status)) => {
                self.exit_code = status.code();
                self.status = if let Some(code) = status.code() {
                    SessionStatus::Exited(code)
                } else {
                    SessionStatus::Killed
                };

                // Read any remaining output
                self.try_read_output();

                true
            }
            Ok(None) => {
                // Still running, check timeout
                let timed_out = self.timeout
                    .map(|t| self.started_at.elapsed() > t)
                    .unwrap_or(false);
                if timed_out {
                    let _ = child.kill();
                    self.status = SessionStatus::TimedOut;
                    self.exit_code = None;
                    return true;
                }
                false
            }
            Err(_) => {
                self.status = SessionStatus::Killed;
                true
            }
        }
    }

    /// Write data to the process stdin.
    pub fn write_stdin(&mut self, data: &str) -> Result<(), String> {
        let Some(ref mut child) = self.child else {
            return Err("Process has exited".to_string());
        };

        let Some(ref mut stdin) = child.stdin else {
            return Err("Process stdin not available".to_string());
        };

        stdin
            .write_all(data.as_bytes())
            .map_err(|e| format!("Failed to write to stdin: {}", e))?;
        stdin
            .flush()
            .map_err(|e| format!("Failed to flush stdin: {}", e))?;

        Ok(())
    }

    /// Kill the process.
    pub fn kill(&mut self) -> Result<(), String> {
        let Some(ref mut child) = self.child else {
            return Ok(()); // Already gone
        };

        child
            .kill()
            .map_err(|e| format!("Failed to kill process: {}", e))?;

        self.status = SessionStatus::Killed;
        Ok(())
    }
}

/// Non-blocking read helper (Unix-specific for now).
#[cfg(unix)]
fn read_nonblocking<R: Read + std::os::unix::io::AsRawFd>(
    reader: &mut R,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    let fd = reader.as_raw_fd();

    // Set non-blocking
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    let result = reader.read(buf);

    // Restore blocking mode
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
    }

    match result {
        Ok(n) => Ok(n),
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(0),
        Err(e) => Err(e),
    }
}

#[cfg(not(unix))]
fn read_nonblocking<R: Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    // On non-Unix, just try a regular read with a short timeout
    // This is a simplified fallback
    Ok(0)
}

/// Global process session manager.
pub struct ProcessManager {
    sessions: HashMap<SessionId, ExecSession>,
}

impl ProcessManager {
    /// Create a new process manager.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Start a new background process.
    pub fn spawn(
        &mut self,
        command: &str,
        working_dir: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SessionId, String> {
        let timeout = timeout_secs.map(Duration::from_secs);

        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn process: {}", e))?;

        let session = ExecSession::new(
            command.to_string(),
            working_dir.to_string(),
            timeout,
            child,
        );

        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);

        Ok(id)
    }

    /// Insert an existing session into the manager.
    pub fn insert(&mut self, session: ExecSession) -> SessionId {
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        id
    }

    /// Get a session by ID.
    pub fn get(&self, id: &str) -> Option<&ExecSession> {
        self.sessions.get(id)
    }

    /// Get a mutable session by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut ExecSession> {
        self.sessions.get_mut(id)
    }

    /// List all sessions.
    pub fn list(&self) -> Vec<&ExecSession> {
        self.sessions.values().collect()
    }

    /// List active (running) sessions.
    pub fn list_active(&self) -> Vec<&ExecSession> {
        self.sessions
            .values()
            .filter(|s| s.status == SessionStatus::Running)
            .collect()
    }

    /// Remove a session.
    pub fn remove(&mut self, id: &str) -> Option<ExecSession> {
        self.sessions.remove(id)
    }

    /// Poll all sessions for updates.
    pub fn poll_all(&mut self) {
        for session in self.sessions.values_mut() {
            if session.status == SessionStatus::Running {
                session.try_read_output();
                session.check_exit();
            }
        }
    }

    /// Clear completed sessions.
    pub fn clear_completed(&mut self) {
        self.sessions.retain(|_, s| s.status == SessionStatus::Running);
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe process manager.
pub type SharedProcessManager = Arc<Mutex<ProcessManager>>;

/// Create a new shared process manager.
pub fn new_shared_manager() -> SharedProcessManager {
    Arc::new(Mutex::new(ProcessManager::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_id_generation() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();
        // IDs contain a hyphen
        assert!(id1.contains('-'));
        assert!(id2.contains('-'));
    }

    #[test]
    fn test_process_manager_creation() {
        let manager = ProcessManager::new();
        assert!(manager.list().is_empty());
    }

    #[test]
    fn test_log_output_with_limits() {
        let session = ExecSession {
            id: "test".to_string(),
            command: "echo test".to_string(),
            working_dir: "/tmp".to_string(),
            started_at: Instant::now(),
            timeout: None,
            status: SessionStatus::Running,
            stdout_buffer: Vec::new(),
            stderr_buffer: Vec::new(),
            combined_output: "line1\nline2\nline3\nline4\nline5\n".to_string(),
            last_read_pos: 0,
            child: None,
            exit_code: None,
        };

        // Get last 2 lines
        let output = session.log_output(None, Some(2));
        assert_eq!(output, "line4\nline5");

        // Get lines 1-3 (0-indexed offset)
        let output = session.log_output(Some(1), Some(2));
        assert_eq!(output, "line2\nline3");
    }
}

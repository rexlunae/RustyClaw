//! Background process session management for RustyClaw.
//!
//! Provides a registry of background exec sessions that can be polled,
//! written to, and killed by the agent.
//!
//! Uses Tokio's async process handling for cross-platform non-blocking I/O,
//! but exposes a sync API for compatibility with the sync tool execute interface.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Unique identifier for a background session.
pub type SessionId = String;

/// Errors produced by [`ExecSession`] stdin/kill operations.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// The process has already exited.
    #[error("Process has exited")]
    Exited,
    /// The process has no stdin handle.
    #[error("Process stdin not available")]
    StdinUnavailable,
    /// A key name passed to `send_keys` could not be translated.
    #[error("{0}")]
    InvalidKeys(String),
    /// An I/O operation on the process failed.
    #[error("Process I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Generate a short human-readable session ID.
fn generate_session_id() -> SessionId {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // Simple adjective-noun pattern for readability
    let adjectives = [
        "warm", "cool", "swift", "calm", "bold", "keen", "bright", "quick",
    ];
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

/// How much of one pipe's output an adopted session holds before the
/// oldest is dropped.
///
/// A ceiling is needed because nothing else provides one. While a command
/// runs in the foreground its output is bounded by how long that lasts;
/// once handed over, the readers keep going for as long as the command
/// does, and a log tail or dev server that nobody polls would otherwise
/// grow this without end.
pub const PIPE_BUFFER_MAX_BYTES: usize = 1 << 20;

/// Trimming copies what it keeps, so it is done once per [`PIPE_BUFFER_MAX_BYTES`]
/// written rather than on every read that crosses the line.
const PIPE_BUFFER_TRIM_AT: usize = PIPE_BUFFER_MAX_BYTES * 2;

/// One pipe's worth of output that an adopted child has produced and its
/// session has not taken yet, bounded.
#[derive(Default)]
pub struct PipeBuffer {
    bytes: Vec<u8>,
    /// Oldest bytes discarded to stay under the cap, not yet reported.
    /// Counted rather than silently forgotten: output vanishing without a
    /// word is the thing a cap must not do.
    dropped: u64,
}

impl PipeBuffer {
    /// Append what a reader just read, dropping the oldest if the cap is
    /// exceeded. The newest output is what a tail or a failing build is
    /// read for, so that is the end kept.
    pub fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > PIPE_BUFFER_TRIM_AT {
            let excess = self.bytes.len() - PIPE_BUFFER_MAX_BYTES;
            self.bytes.drain(..excess);
            self.dropped += excess as u64;
        }
    }

    /// Everything held, leaving it in place.
    pub fn peek(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    /// Take what is held, with the number of bytes dropped ahead of it.
    pub fn take(&mut self) -> (Vec<u8>, u64) {
        (
            std::mem::take(&mut self.bytes),
            std::mem::take(&mut self.dropped),
        )
    }

    /// Whether there is nothing to report — no bytes and no drops.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty() && self.dropped == 0
    }
}

/// A pipe buffer shared between a reader task and the session draining it.
pub type SharedPipe = Arc<Mutex<PipeBuffer>>;

/// A child that was already running elsewhere when the session took it on.
///
/// The foreground exec path spawns through `tokio::process`, which this
/// manager cannot poll synchronously. Rather than kill that child and run
/// the command again from the top — which threw away however much work it
/// had already done — the pipe readers already draining it keep running and
/// go on appending to [`Self::stdout`] and [`Self::stderr`], and a
/// supervisor task holds the child and records where it ended up in
/// [`Self::finished`]. This side reads those, so polling an adopted session
/// looks the same from outside as polling a spawned one.
pub struct AdoptedChild {
    /// Bytes the still-running stdout reader has produced but this session
    /// has not taken yet. Emptied on each poll — see `drain_adopted`.
    stdout: SharedPipe,
    /// The same, for stderr.
    stderr: SharedPipe,
    /// Set by the supervisor once the child is over.
    finished: Arc<Mutex<Option<SessionStatus>>>,
    /// Bytes to write to the child's stdin, handled by the supervisor
    /// because the handle is async and this API is not.
    stdin_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    /// Asks the supervisor to kill the child. Taken on first use — a kill
    /// is not repeatable.
    kill_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl AdoptedChild {
    /// Assemble the session side of an adoption. The caller spawns the
    /// supervisor that owns the child and fills these in.
    pub fn new(
        stdout: SharedPipe,
        stderr: SharedPipe,
        finished: Arc<Mutex<Option<SessionStatus>>>,
        stdin_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
        kill_tx: tokio::sync::oneshot::Sender<()>,
    ) -> Self {
        Self {
            stdout,
            stderr,
            finished,
            stdin_tx,
            kill_tx: Some(kill_tx),
        }
    }
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
    /// The child process handle, when this session spawned it itself.
    child: Option<Child>,
    /// The backing for a child adopted from the async exec path. Exactly
    /// one of this and [`Self::child`] is set.
    adopted: Option<AdoptedChild>,
    /// Exit code (set when process exits).
    exit_code: Option<i32>,
    /// Optional caller identity that owns this process session.
    /// Used for access control — only the owning session can poll/write/kill.
    owner_id: Option<String>,
}

impl ExecSession {
    /// Create a new session for a running process.
    pub fn new(
        command: String,
        working_dir: String,
        timeout: Option<Duration>,
        child: Child,
    ) -> Self {
        Self::with_owner(command, working_dir, timeout, child, None)
    }

    /// Create a new session with an owner identity for access control.
    pub fn with_owner(
        command: String,
        working_dir: String,
        timeout: Option<Duration>,
        child: Child,
        owner_id: Option<String>,
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
            adopted: None,
            exit_code: None,
            owner_id,
        }
    }

    /// Take on a child that is already running, keeping the elapsed time
    /// it has behind it.
    ///
    /// `started_at` is the moment the command actually began, not the
    /// moment of adoption: the point of handing a child over rather than
    /// re-running it is that its progress counts, and that includes its
    /// age. A timeout measured from the hand-off would give a command that
    /// had already been running for most of its budget a fresh one.
    pub fn adopt(
        command: String,
        working_dir: String,
        timeout: Option<Duration>,
        started_at: Instant,
        adopted: AdoptedChild,
        owner_id: Option<String>,
    ) -> Self {
        Self {
            id: generate_session_id(),
            command,
            working_dir,
            started_at,
            timeout,
            status: SessionStatus::Running,
            stdout_buffer: Vec::new(),
            stderr_buffer: Vec::new(),
            combined_output: String::new(),
            last_read_pos: 0,
            child: None,
            adopted: Some(adopted),
            exit_code: None,
            owner_id,
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
    ///
    /// Uses platform-specific non-blocking I/O.
    pub fn try_read_output(&mut self) -> bool {
        if self.adopted.is_some() {
            return self.drain_adopted();
        }
        let Some(ref mut child) = self.child else {
            return false;
        };

        let mut read_any = false;

        // Try to read from stdout
        if let Some(ref mut stdout) = child.stdout {
            let mut buf = [0u8; 4096];
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

    /// Copy whatever the adopted child's readers have appended since the
    /// last poll into this session's buffers.
    fn drain_adopted(&mut self) -> bool {
        let Self {
            adopted: Some(ad),
            combined_output,
            stdout_buffer,
            stderr_buffer,
            ..
        } = self
        else {
            return false;
        };

        let mut read_any = false;
        for (buf, own) in [
            (&ad.stdout, &mut *stdout_buffer),
            (&ad.stderr, &mut *stderr_buffer),
        ] {
            // Taken, not copied from. The readers only append and this is
            // the only place that consumes, so leaving the bytes behind
            // would mean the session and the buffer each held a complete
            // copy of everything the command had ever printed, growing for
            // as long as it ran.
            let (fresh, dropped) = {
                let Ok(mut shared) = buf.lock() else { continue };
                if shared.is_empty() {
                    continue;
                }
                shared.take()
            };
            // Said, not swallowed. Anything discarded was older than what
            // follows, so the note belongs ahead of it.
            if dropped > 0 {
                combined_output.push_str(&format!(
                    "\n[… {dropped} bytes of earlier output dropped, buffer limit reached …]\n"
                ));
            }
            combined_output.push_str(&String::from_utf8_lossy(&fresh));
            own.extend_from_slice(&fresh);
            read_any = true;
        }
        read_any
    }

    /// Check if the process has exited and update status.
    pub fn check_exit(&mut self) -> bool {
        if self.adopted.is_some() {
            return self.check_adopted_exit();
        }
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
                let timed_out = self
                    .timeout
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

    /// The adopted equivalent of [`Self::check_exit`]: the supervisor owns
    /// the child, so its verdict is what settles this.
    fn check_adopted_exit(&mut self) -> bool {
        let finished = self
            .adopted
            .as_ref()
            .and_then(|ad| ad.finished.lock().ok().and_then(|f| f.clone()));

        if let Some(status) = finished {
            self.exit_code = match status {
                SessionStatus::Exited(code) => Some(code),
                _ => None,
            };
            self.status = status;
            // Sweep up whatever the readers appended between the child
            // exiting and this poll noticing.
            self.try_read_output();
            return true;
        }

        if self.is_timed_out() {
            let _ = self.kill();
            self.status = SessionStatus::TimedOut;
            self.exit_code = None;
            return true;
        }
        false
    }

    /// Write data to the process stdin.
    pub fn write_stdin(&mut self, data: &str) -> Result<(), ProcessError> {
        self.write_stdin_bytes(data.as_bytes())
    }

    /// Put bytes on the child's stdin, whichever kind of child this is.
    ///
    /// Both stdin entry points go through here so neither can be taught
    /// about adopted sessions without the other: they differ only in how
    /// the bytes are arrived at, and routing them was where that
    /// difference had turned into two behaviours.
    fn write_stdin_bytes(&mut self, bytes: &[u8]) -> Result<(), ProcessError> {
        if let Some(ad) = self.adopted.as_ref() {
            if self.status != SessionStatus::Running {
                return Err(ProcessError::Exited);
            }
            return ad
                .stdin_tx
                .send(bytes.to_vec())
                .map_err(|_| ProcessError::Exited);
        }
        let Some(ref mut child) = self.child else {
            return Err(ProcessError::Exited);
        };

        let Some(ref mut stdin) = child.stdin else {
            return Err(ProcessError::StdinUnavailable);
        };

        stdin.write_all(bytes)?;
        stdin.flush()?;

        Ok(())
    }

    /// Translate named keys to escape sequences and write them to stdin.
    ///
    /// Supports key names: Enter, Tab, Escape, Space, Backspace,
    /// Up, Down, Left, Right, Home, End, PageUp, PageDown, Delete, Insert,
    /// Ctrl-A..Ctrl-Z, Ctrl-C, F1..F12, and plain text.
    ///
    /// Multiple keys can be separated by spaces:  `"Enter"`, `"Ctrl-C"`,
    /// `"Up Up Down Down Left Right"`.
    pub fn send_keys(&mut self, keys: &str) -> Result<usize, ProcessError> {
        let bytes = translate_keys(keys).map_err(ProcessError::InvalidKeys)?;
        let len = bytes.len();
        self.write_stdin_bytes(&bytes)?;
        Ok(len)
    }

    /// Kill the process.
    pub fn kill(&mut self) -> Result<(), ProcessError> {
        if let Some(ad) = self.adopted.as_mut() {
            // The supervisor holds the child, so a kill is a request to
            // it. Once sent there is nothing to repeat: the sender is
            // consumed, and a second call finds it gone.
            if let Some(tx) = ad.kill_tx.take() {
                let _ = tx.send(());
            }
            self.status = SessionStatus::Killed;
            return Ok(());
        }

        let Some(ref mut child) = self.child else {
            return Ok(()); // Already gone
        };

        child.kill()?;

        self.status = SessionStatus::Killed;
        Ok(())
    }
}

// ── Non-blocking read helpers ───────────────────────────────────────────────

/// Non-blocking read helper for Unix using poll().
#[cfg(unix)]
fn read_nonblocking<R: std::io::Read + std::os::unix::io::AsRawFd>(
    reader: &mut R,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    let fd = reader.as_raw_fd();

    // Use poll() to check if data is available (more portable than fcntl tricks)
    let mut poll_fd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    // Poll with 0 timeout (non-blocking check)
    let ready = unsafe { libc::poll(&mut poll_fd, 1, 0) };

    if ready > 0 && (poll_fd.revents & libc::POLLIN) != 0 {
        // Data available, do a regular read
        reader.read(buf)
    } else {
        // No data available
        Ok(0)
    }
}

/// Non-blocking read helper for Windows using PeekNamedPipe.
#[cfg(windows)]
fn read_nonblocking<R: std::io::Read + std::os::windows::io::AsRawHandle>(
    reader: &mut R,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;

    let handle = reader.as_raw_handle() as HANDLE;
    let mut available: u32 = 0;

    // Check if data is available without blocking
    let result = unsafe {
        PeekNamedPipe(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };

    if result != 0 && available > 0 {
        // Data available, do a regular read
        let to_read = (available as usize).min(buf.len());
        reader.read(&mut buf[..to_read])
    } else {
        // No data available or error (treat as no data)
        Ok(0)
    }
}

/// Fallback for platforms without specific implementation.
#[cfg(not(any(unix, windows)))]
fn read_nonblocking<R: std::io::Read>(_reader: &mut R, _buf: &mut [u8]) -> std::io::Result<usize> {
    // Can't do non-blocking reads on unknown platform
    Ok(0)
}

// ── Key translation ─────────────────────────────────────────────────────────

/// Translate a space-separated list of named keys into raw bytes.
///
/// Each token is matched case-insensitively against known key names.
/// Unrecognised tokens are sent as literal UTF-8 text.
pub fn translate_keys(keys: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();

    for token in keys.split_whitespace() {
        match token.to_lowercase().as_str() {
            // Basic control characters
            "enter" | "return" | "cr" => out.push(b'\n'),
            "tab" => out.push(b'\t'),
            "escape" | "esc" => out.push(0x1b),
            "space" => out.push(b' '),
            "backspace" | "bs" => out.push(0x7f),
            "delete" | "del" => out.extend_from_slice(b"\x1b[3~"),
            "insert" | "ins" => out.extend_from_slice(b"\x1b[2~"),

            // Arrow keys
            "up" => out.extend_from_slice(b"\x1b[A"),
            "down" => out.extend_from_slice(b"\x1b[B"),
            "right" => out.extend_from_slice(b"\x1b[C"),
            "left" => out.extend_from_slice(b"\x1b[D"),

            // Navigation
            "home" => out.extend_from_slice(b"\x1b[H"),
            "end" => out.extend_from_slice(b"\x1b[F"),
            "pageup" | "pgup" => out.extend_from_slice(b"\x1b[5~"),
            "pagedown" | "pgdn" => out.extend_from_slice(b"\x1b[6~"),

            // Function keys
            "f1" => out.extend_from_slice(b"\x1bOP"),
            "f2" => out.extend_from_slice(b"\x1bOQ"),
            "f3" => out.extend_from_slice(b"\x1bOR"),
            "f4" => out.extend_from_slice(b"\x1bOS"),
            "f5" => out.extend_from_slice(b"\x1b[15~"),
            "f6" => out.extend_from_slice(b"\x1b[17~"),
            "f7" => out.extend_from_slice(b"\x1b[18~"),
            "f8" => out.extend_from_slice(b"\x1b[19~"),
            "f9" => out.extend_from_slice(b"\x1b[20~"),
            "f10" => out.extend_from_slice(b"\x1b[21~"),
            "f11" => out.extend_from_slice(b"\x1b[23~"),
            "f12" => out.extend_from_slice(b"\x1b[24~"),

            // Ctrl- combinations
            other if other.starts_with("ctrl-") || other.starts_with("c-") => {
                let ch = other.rsplit('-').next().unwrap_or("");
                if ch.len() == 1 {
                    let b = ch.as_bytes()[0];
                    // Ctrl-A = 0x01, Ctrl-Z = 0x1A, Ctrl-[ = 0x1B, etc.
                    let ctrl = match b {
                        b'a'..=b'z' => b - b'a' + 1,
                        b'@' => 0,
                        b'[' => 0x1b,
                        b'\\' => 0x1c,
                        b']' => 0x1d,
                        b'^' => 0x1e,
                        b'_' => 0x1f,
                        _ => return Err(format!("Unknown Ctrl- key: {}", token)),
                    };
                    out.push(ctrl);
                } else {
                    return Err(format!("Invalid Ctrl- key: {}", token));
                }
            }

            // Literal text fallback
            _ => out.extend_from_slice(token.as_bytes()),
        }
    }

    Ok(out)
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
    ) -> Result<SessionId, ProcessError> {
        self.spawn_with_owner(command, working_dir, timeout_secs, None)
    }

    /// Spawn a background process with an optional owner identity.
    pub fn spawn_with_owner(
        &mut self,
        command: &str,
        working_dir: &str,
        timeout_secs: Option<u64>,
        owner_id: Option<String>,
    ) -> Result<SessionId, ProcessError> {
        let timeout = timeout_secs.map(Duration::from_secs);

        // Use platform-appropriate shell
        #[cfg(unix)]
        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        #[cfg(windows)]
        let child = Command::new("cmd")
            .arg("/C")
            .arg(command)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        #[cfg(not(any(unix, windows)))]
        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let session = ExecSession::with_owner(
            command.to_string(),
            working_dir.to_string(),
            timeout,
            child,
            owner_id,
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

    /// Get a session by ID, verifying the caller owns it (if owner tracking is enabled).
    /// Returns `None` if the session doesn't exist or ownership check fails.
    pub fn get_owned(&self, id: &str, caller_id: Option<&str>) -> Option<&ExecSession> {
        let session = self.sessions.get(id)?;
        if let Some(owner) = &session.owner_id {
            if let Some(caller) = caller_id {
                if owner != caller {
                    return None;
                }
            }
        }
        Some(session)
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
        self.sessions
            .retain(|_, s| s.status == SessionStatus::Running);
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

    /// A reader that nobody drains does not grow without end.
    ///
    /// The pipe is drained continuously once a command is handed over, so
    /// the kernel's pipe buffer no longer pushes back on a chatty writer.
    /// Without a ceiling here, a log tail or dev server that is started and
    /// then ignored grows this for as long as it runs.
    #[test]
    fn an_undrained_pipe_buffer_stays_bounded() {
        let mut buf = PipeBuffer::default();
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..200 {
            buf.push(&chunk);
        }
        assert!(
            buf.peek().len() <= PIPE_BUFFER_TRIM_AT,
            "12.5MB written with nobody reading should not all be held: {} bytes",
            buf.peek().len()
        );

        let (kept, dropped) = buf.take();
        assert!(dropped > 0, "the bytes that went missing should be counted");
        assert_eq!(
            kept.len() as u64 + dropped,
            200 * 64 * 1024,
            "every byte is either kept or accounted for as dropped"
        );
    }

    /// What a cap discards, it says it discarded.
    #[test]
    fn dropped_output_is_reported_not_silently_lost() {
        let stdout: SharedPipe = Arc::default();
        let finished: Arc<Mutex<Option<SessionStatus>>> = Arc::default();
        let (stdin_tx, _stdin_rx) = tokio::sync::mpsc::unbounded_channel();
        let (kill_tx, _kill_rx) = tokio::sync::oneshot::channel();

        {
            let mut buf = stdout.lock().unwrap();
            let chunk = vec![b'x'; 64 * 1024];
            for _ in 0..40 {
                buf.push(&chunk);
            }
        }

        let mut session = ExecSession::adopt(
            "chatty".to_string(),
            "/tmp".to_string(),
            None,
            Instant::now(),
            AdoptedChild::new(stdout, Arc::default(), finished, stdin_tx, kill_tx),
            None,
        );

        assert!(session.try_read_output());
        assert!(
            session
                .full_output()
                .contains("bytes of earlier output dropped"),
            "a reader that had to discard output should say so"
        );
    }

    /// Polling an adopted session takes the reader's bytes rather than
    /// copying them.
    ///
    /// The readers keep running for the whole background lifetime, so their
    /// buffer is the one thing here that grows for as long as the command
    /// does. The session is its only consumer: anything left behind after a
    /// poll is a second copy of the same output that nothing will free, and
    /// for a dev server or a log tail that is unbounded.
    #[test]
    fn polling_an_adopted_session_drains_the_readers_buffer() {
        let stdout: SharedPipe = Arc::default();
        let stderr: SharedPipe = Arc::default();
        let finished: Arc<Mutex<Option<SessionStatus>>> = Arc::default();
        let (stdin_tx, _stdin_rx) = tokio::sync::mpsc::unbounded_channel();
        let (kill_tx, _kill_rx) = tokio::sync::oneshot::channel();

        stdout.lock().unwrap().push(b"out-first ");
        stderr.lock().unwrap().push(b"err-first");

        let mut session = ExecSession::adopt(
            "chatty".to_string(),
            "/tmp".to_string(),
            None,
            Instant::now(),
            AdoptedChild::new(stdout.clone(), stderr.clone(), finished, stdin_tx, kill_tx),
            None,
        );

        assert!(session.try_read_output(), "there was output to collect");
        assert!(session.full_output().contains("out-first"));
        assert!(session.full_output().contains("err-first"));

        assert!(
            stdout.lock().unwrap().is_empty(),
            "the stdout the session took should not still be buffered"
        );
        assert!(
            stderr.lock().unwrap().is_empty(),
            "the stderr the session took should not still be buffered"
        );

        // Draining must not cost the session anything that arrives later.
        stdout.lock().unwrap().push(b" out-second");
        assert!(session.try_read_output(), "later output still arrives");
        assert!(session.full_output().contains("out-second"));
        assert!(
            session.full_output().contains("out-first"),
            "what was drained earlier is still in the session's own record"
        );
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
            adopted: None,
            exit_code: None,
            owner_id: None,
        };

        // Get last 2 lines
        let output = session.log_output(None, Some(2));
        assert_eq!(output, "line4\nline5");

        // Get lines 1-3 (0-indexed offset)
        let output = session.log_output(Some(1), Some(2));
        assert_eq!(output, "line2\nline3");
    }

    #[test]
    fn test_translate_keys_basic() {
        let keys = translate_keys("Enter").unwrap();
        assert_eq!(keys, vec![b'\n']);

        let keys = translate_keys("Ctrl-C").unwrap();
        assert_eq!(keys, vec![3]); // 0x03

        let keys = translate_keys("Up Down Left Right").unwrap();
        assert_eq!(keys, b"\x1b[A\x1b[B\x1b[D\x1b[C".to_vec());
    }

    #[test]
    fn test_translate_keys_literal() {
        let keys = translate_keys("hello").unwrap();
        assert_eq!(keys, b"hello".to_vec());
    }

    #[test]
    #[cfg(unix)]
    fn test_spawn_and_poll() {
        let mut manager = ProcessManager::new();
        let id = manager.spawn("echo hello", "/tmp", None).unwrap();

        // Give it a moment
        std::thread::sleep(Duration::from_millis(100));

        // Poll for updates
        manager.poll_all();

        let session = manager.get(&id).unwrap();
        assert!(session.full_output().contains("hello"));
    }
}

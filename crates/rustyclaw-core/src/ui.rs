//! Client-agnostic UI state types.
//!
//! These types are shared across all RustyClaw clients (desktop, TUI, CLI)
//! so that conversation models, thread info, and tool-call state have a
//! single canonical definition instead of living in each client crate.

use crate::types::MessageRole;
use crate::user_prompt_types::UserPrompt;

// ── Connection status ───────────────────────────────────────────────────────

/// Connection status to the gateway.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Authenticating,
    Authenticated,
    Error(String),
}

// ── Chat message types ──────────────────────────────────────────────────────

/// A chat message in the conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub tool_calls: Vec<ToolCallInfo>,
    pub is_streaming: bool,
    /// Wall-clock duration of the activity this message represents (ms).
    /// Set on Thinking messages when the reasoning block completes, so
    /// renderers can say "Thought for 4.2s".
    pub duration_ms: Option<u64>,
}

/// Information about a tool call within a message.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub result: Option<String>,
    pub is_error: bool,
    pub collapsed: bool,
    /// Wall-clock execution time in milliseconds, measured client-side
    /// between the ToolCall and ToolResult events (None while running
    /// or when replayed from history, which carries no timings).
    pub duration_ms: Option<u64>,
    /// Live execution status streamed by the gateway while the call is
    /// still running (None once the result arrives).
    pub live_status: Option<ToolLiveStatus>,
    /// Live output tail streamed while the tool runs (already processed
    /// by [`append_terminal_chunk`]: CR-overwrites applied, ANSI escapes
    /// stripped, bounded). Cleared when the final result arrives.
    pub live_output: String,
}

/// Live execution status for a tool call that is still running, streamed
/// by the gateway roughly once a second. Where the tool is waiting on a
/// child process the snapshot carries that process's stats; otherwise
/// only the elapsed time (and any tool-provided message) is present.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolLiveStatus {
    /// Time the tool call has been executing.
    pub elapsed_ms: u64,
    /// PID of the child process the tool is waiting on. Present ⇒ the
    /// process can be paused/resumed/stopped/killed by the user.
    pub pid: Option<u32>,
    /// CPU usage as a percentage of one core.
    pub cpu_percent: Option<f32>,
    /// Resident memory in bytes.
    pub memory_bytes: Option<u64>,
    /// Scheduler state ("running", "sleeping", "blocked on I/O",
    /// "paused", …).
    pub state: Option<String>,
    /// Tool-provided progress message, when available.
    pub message: Option<String>,
}

impl ToolLiveStatus {
    /// Whether the user paused the underlying process.
    pub fn is_paused(&self) -> bool {
        self.state.as_deref() == Some("paused")
    }
}

/// Maximum characters kept in a live-output tail.
pub const TERMINAL_TAIL_MAX_CHARS: usize = 4_000;
/// Maximum lines kept in a live-output tail.
pub const TERMINAL_TAIL_MAX_LINES: usize = 40;

/// Append a chunk of raw process output to a live tail the way a
/// terminal would render it:
///
/// - A carriage return followed by more text on the same line rewinds to
///   the start of the line, so progress bars that redraw with `\r`
///   overwrite in place instead of stacking new lines.
/// - CRLF is treated as a plain newline.
/// - A trailing `\r` is deferred (kept in the buffer) so the decision
///   can be made when the next chunk arrives.
/// - ANSI CSI/OSC escape sequences (colors, cursor movement) are
///   stripped: neither client renders them, and raw escapes would
///   corrupt the TUI's layout.
/// - The buffer is bounded to the last [`TERMINAL_TAIL_MAX_LINES`] lines
///   and [`TERMINAL_TAIL_MAX_CHARS`] characters.
pub fn append_terminal_chunk(buf: &mut String, chunk: &str) {
    // A '\r' deferred from the previous chunk is re-processed now that
    // its lookahead exists.
    let mut pending_cr = buf.ends_with('\r');
    if pending_cr {
        buf.pop();
    }
    let mut chars = chunk.chars().peekable();
    loop {
        let c = if pending_cr {
            pending_cr = false;
            '\r'
        } else {
            match chars.next() {
                Some(c) => c,
                None => break,
            }
        };
        match c {
            '\r' => match chars.peek() {
                // CRLF → the '\n' alone moves to the next line.
                Some(&'\n') => {}
                // Chunk ends in '\r' — defer until the next chunk.
                None => buf.push('\r'),
                // Text follows on the same line: rewind to overwrite it.
                Some(_) => match buf.rfind('\n') {
                    Some(i) => buf.truncate(i + 1),
                    None => buf.clear(),
                },
            },
            // Strip ANSI escape sequences.
            '\u{1b}' => match chars.peek() {
                // CSI: ESC [ params… final-byte (@ through ~).
                Some('[') => {
                    chars.next();
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] … terminated by BEL or ESC \.
                Some(']') => {
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '\u{07}' {
                            break;
                        }
                        if n == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                // Two-character escape (ESC c, ESC 7, …).
                _ => {
                    chars.next();
                }
            },
            _ => buf.push(c),
        }
    }
    cap_terminal_tail(buf);
}

/// Trim a live tail from the front to its line/char bounds.
fn cap_terminal_tail(buf: &mut String) {
    let excess_lines = buf.lines().count().saturating_sub(TERMINAL_TAIL_MAX_LINES);
    if excess_lines > 0 {
        let mut idx = 0;
        for _ in 0..excess_lines {
            match buf[idx..].find('\n') {
                Some(i) => idx += i + 1,
                None => break,
            }
        }
        buf.drain(..idx);
    }
    if buf.len() > TERMINAL_TAIL_MAX_CHARS {
        let mut cut = buf.len() - TERMINAL_TAIL_MAX_CHARS;
        while !buf.is_char_boundary(cut) {
            cut += 1;
        }
        buf.drain(..cut);
    }
}

// ── Thread / session types ──────────────────────────────────────────────────

/// Thread/session info for the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadInfo {
    pub id: u64,
    /// Project this thread belongs to (0 = the active project).
    pub project_id: u64,
    pub label: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
    pub message_count: usize,
}

/// Project info for the sidebar's top level.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ProjectInfo {
    pub id: u64,
    pub name: String,
    pub path: String,
}

// ── Dialog state helpers ────────────────────────────────────────────────────

/// Pending state for dialogs that the UI reads to show/hide overlays.
///
/// This is an intermediate representation: the client's signal-based or
/// channel-based state reacts to `GatewayEvent`s by pushing entries here,
/// and the UI rendering code reads these to display the relevant dialog.
#[derive(Clone, Debug, Default)]
pub struct DialogState {
    /// Pending tool approval request (id, name, arguments).
    pub pending_tool_approval: Option<(String, String, String)>,

    /// Pending user prompt from the agent.
    pub pending_user_prompt: Option<UserPrompt>,

    /// Pending credential request (id, provider, secret_name, message).
    pub pending_credential_request: Option<(String, String, String, String)>,

    /// Pending device flow (url, code, message).
    pub pending_device_flow: Option<(String, String, Option<String>)>,

    /// Number of streaming chunks received in the current response.
    pub streaming_chunks: u32,

    /// Total bytes received in the current streaming response.
    pub streaming_bytes: usize,
}

impl ChatMessage {
    /// Create a new user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: content.into(),
            timestamp: chrono::Utc::now(),
            tool_calls: Vec::new(),
            is_streaming: false,
            duration_ms: None,
        }
    }

    /// Start a new assistant message (streaming).
    pub fn start_assistant(id: String) -> Self {
        Self {
            id,
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: chrono::Utc::now(),
            tool_calls: Vec::new(),
            is_streaming: true,
            duration_ms: None,
        }
    }

    /// Start a new thinking block (streaming reasoning text).
    pub fn start_thinking() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::Thinking,
            content: String::new(),
            timestamp: chrono::Utc::now(),
            tool_calls: Vec::new(),
            is_streaming: true,
            duration_ms: None,
        }
    }

    /// Create an inline notice (Info, Success, Warning, or Error banner
    /// rendered in the transcript at the point it occurred).
    pub fn notice(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            content: content.into(),
            timestamp: chrono::Utc::now(),
            tool_calls: Vec::new(),
            is_streaming: false,
            duration_ms: None,
        }
    }

    /// Append text content to this message (used during streaming).
    pub fn append_content(&mut self, delta: &str) {
        if self.is_streaming {
            self.content.push_str(delta);
        }
    }

    /// Mark this message as finished streaming.
    pub fn finish(&mut self) {
        self.is_streaming = false;
    }

    /// Add a tool call to this message.
    pub fn add_tool_call(&mut self, id: String, name: String, arguments: String) {
        self.tool_calls.push(ToolCallInfo {
            id,
            name,
            arguments,
            result: None,
            is_error: false,
            collapsed: true,
            duration_ms: None,
            live_status: None,
            live_output: String::new(),
        });
    }

    /// Append a chunk of live output to a still-running tool call.
    /// Returns whether a matching (open) call was found.
    pub fn append_tool_output(&mut self, id: &str, chunk: &str) -> bool {
        for tool in &mut self.tool_calls {
            if tool.id == id && tool.result.is_none() {
                append_terminal_chunk(&mut tool.live_output, chunk);
                return true;
            }
        }
        false
    }

    /// Set the result for a tool call by ID, with the client-measured
    /// execution time (None when replaying history, which has no timings).
    pub fn set_tool_result(
        &mut self,
        id: &str,
        result: String,
        is_error: bool,
        duration_ms: Option<u64>,
    ) {
        for tool in &mut self.tool_calls {
            if tool.id == id {
                tool.result = Some(result);
                tool.is_error = is_error;
                tool.duration_ms = duration_ms;
                tool.live_status = None;
                // The final result supersedes the live tail.
                tool.live_output = String::new();
                return;
            }
        }
    }

    /// Update the live status of a still-running tool call. Returns true
    /// if a matching, unfinished call was found.
    pub fn set_tool_live_status(&mut self, id: &str, status: ToolLiveStatus) -> bool {
        for tool in &mut self.tool_calls {
            if tool.id == id && tool.result.is_none() {
                tool.live_status = Some(status);
                return true;
            }
        }
        false
    }
}

impl DialogState {
    /// Merge a [`super::gateway::client_types::GatewayEvent`] into the dialog state.
    ///
    /// Returns `true` if the event was consumed (set some dialog state),
    /// `false` if the event is unrelated to dialogs.
    pub fn handle_gateway_event(
        &mut self,
        event: &crate::gateway::client_types::GatewayEvent,
    ) -> bool {
        match event {
            crate::gateway::client_types::GatewayEvent::ToolApprovalRequest {
                id,
                name,
                arguments,
            } => {
                self.pending_tool_approval = Some((id.clone(), name.clone(), arguments.clone()));
                true
            }
            crate::gateway::client_types::GatewayEvent::UserPromptRequest { prompt, .. } => {
                self.pending_user_prompt = Some(prompt.clone());
                true
            }
            crate::gateway::client_types::GatewayEvent::CredentialRequest {
                id,
                provider,
                secret_name,
                message,
            } => {
                self.pending_credential_request = Some((
                    id.clone(),
                    provider.clone(),
                    secret_name.clone(),
                    message.clone(),
                ));
                true
            }
            crate::gateway::client_types::GatewayEvent::DeviceFlowStart { url, code, message } => {
                self.pending_device_flow = Some((url.clone(), code.clone(), message.clone()));
                true
            }
            crate::gateway::client_types::GatewayEvent::DeviceFlowComplete => {
                self.pending_device_flow = None;
                true
            }
            _ => false,
        }
    }
}

// ── Content formatting helpers ──────────────────────────────────────────────

/// Pretty-print a JSON string for display (tool call arguments, results).
///
/// Falls back to the raw string if JSON deserialisation fails.
pub fn pretty_print_json(input: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| input.to_string())
}

/// Truncate content to fit within display constraints.
///
/// - `max_chars`: hard character limit (applies before `max_lines`)
/// - `max_lines`: maximum number of lines to show (beyond this, `…` is appended)
///
/// When both limits apply, the shorter one wins.
pub fn truncate_content(content: &str, max_chars: usize, max_lines: usize) -> String {
    let mut result = String::with_capacity(content.len().min(max_chars));
    let mut line_count = 0usize;

    for (char_count, ch) in content.chars().enumerate() {
        if char_count >= max_chars {
            result.push('…');
            break;
        }
        if ch == '\n' {
            line_count += 1;
            if line_count >= max_lines {
                result.push('…');
                result.push('\n');
                break;
            }
        }
        result.push(ch);
    }

    result
}

/// Format a tool call name and arguments into a human-readable label.
///
/// Uses [`pretty_print_json`] on the arguments and truncates long content.
pub fn format_tool_call(name: &str, arguments: &str) -> String {
    let pretty = pretty_print_json(arguments);
    if pretty.len() > 200 {
        format!("🔧 {}\n{}\n…", name, truncate_content(&pretty, 200, 8))
    } else {
        format!("🔧 {}\n{}", name, pretty)
    }
}

/// Format a tool result for display.
///
/// Truncates long results and marks errors.
pub fn format_tool_result(result: &str, is_error: bool) -> String {
    let preview = if result.len() > 100 {
        truncate_content(&pretty_print_json(result), 500, 15)
    } else {
        result.to_string()
    };

    if is_error {
        format!("✕ Error:\n{}", preview)
    } else {
        format!("✓ Result:\n{}", preview)
    }
}

/// Format a UTC timestamp as a relative human-readable string.
///
/// "Just now" (< 10s ago), "12m ago", "2h ago", "3d ago", or the
/// date ("Jan 15") for older timestamps.
pub fn format_relative_time(timestamp: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(*timestamp);

    let secs = duration.num_seconds();
    if secs < 10 {
        "Just now".to_string()
    } else if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 604800 {
        format!("{}d ago", secs / 86400)
    } else {
        timestamp.format("%b %d").to_string()
    }
}

/// Format a timestamp as a short time string for chat bubbles
/// (e.g. "14:32" or "Yesterday 14:32").
pub fn format_chat_timestamp(timestamp: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(*timestamp);

    if duration.num_seconds() < 10 {
        "Just now".to_string()
    } else if duration.num_hours() < 24 {
        timestamp.format("%H:%M").to_string()
    } else if duration.num_hours() < 48 {
        format!("Yesterday {}", timestamp.format("%H:%M"))
    } else {
        timestamp.format("%b %d %H:%M").to_string()
    }
}

// ── Streaming state ─────────────────────────────────────────────────────────

use std::time::Instant;

/// Tracks the progress of an active streaming response.
///
/// Both the TUI and desktop clients need to track similar streaming
/// metrics. This struct consolidates that tracking in one place.
#[derive(Clone, Debug, Default)]
pub struct StreamingState {
    /// Whether we are currently receiving a streaming response.
    pub is_streaming: bool,

    /// Whether the model is currently in "thinking" mode.
    pub is_thinking: bool,

    /// Number of streaming chunks received so far.
    pub chunks: u32,

    /// Total bytes received across all chunks.
    pub bytes: usize,

    /// When the current stream started.
    pub start_time: Option<Instant>,
}

impl StreamingState {
    /// Start tracking a new streaming response.
    pub fn start_streaming(&mut self) {
        self.is_streaming = true;
        self.is_thinking = false;
        self.chunks = 0;
        self.bytes = 0;
        self.start_time = Some(Instant::now());
    }

    /// Start thinking mode (extended thinking models).
    pub fn start_thinking(&mut self) {
        self.is_thinking = true;
        self.is_streaming = false;
    }

    /// End thinking mode and begin actual streaming.
    pub fn end_thinking(&mut self) {
        self.is_thinking = false;
        self.is_streaming = true;
        self.start_time = Some(Instant::now());
    }

    /// Record one chunk of streaming data.
    pub fn record_chunk(&mut self, data: &str) {
        self.chunks += 1;
        self.bytes += data.len();
    }

    /// Finish the streaming response.
    pub fn finish(&mut self) {
        self.is_streaming = false;
        self.is_thinking = false;
        self.start_time = None;
    }

    /// Human-readable progress summary (e.g. "42 chunks, 12.3 KB").
    pub fn progress_summary(&self) -> String {
        if self.bytes >= 1024 {
            format!(
                "{} chunks, {:.1} KB",
                self.chunks,
                self.bytes as f64 / 1024.0,
            )
        } else if self.chunks > 0 {
            format!("{} chunks, {} B", self.chunks, self.bytes)
        } else {
            "Streaming…".to_string()
        }
    }
}

#[cfg(test)]
mod terminal_tail_tests {
    use super::*;

    fn tail(chunks: &[&str]) -> String {
        let mut buf = String::new();
        for c in chunks {
            append_terminal_chunk(&mut buf, c);
        }
        buf
    }

    #[test]
    fn plain_text_appends() {
        assert_eq!(
            tail(&["hello ", "world\n", "second"]),
            "hello world\nsecond"
        );
    }

    #[test]
    fn carriage_return_overwrites_the_line() {
        // A progress bar redrawing with \r keeps only the latest state.
        assert_eq!(tail(&["step 1/3\rstep 2/3\rstep 3/3"]), "step 3/3");
        // Earlier completed lines are untouched.
        assert_eq!(tail(&["done\n10%\r20%\r100%"]), "done\n100%");
    }

    #[test]
    fn crlf_is_a_plain_newline() {
        assert_eq!(tail(&["a\r\nb"]), "a\nb");
    }

    #[test]
    fn trailing_cr_defers_across_chunks() {
        // \r at a chunk boundary: followed by \n → CRLF (line survives);
        // followed by text → overwrite.
        assert_eq!(tail(&["progress 50%\r", "\ndone"]), "progress 50%\ndone");
        assert_eq!(tail(&["progress 50%\r", "progress 99%"]), "progress 99%");
    }

    #[test]
    fn ansi_escapes_are_stripped() {
        assert_eq!(tail(&["\u{1b}[32mgreen\u{1b}[0m ok"]), "green ok");
        // OSC title sequence, BEL-terminated.
        assert_eq!(tail(&["\u{1b}]0;title\u{07}text"]), "text");
        // CSI split across chunks is not supported (rare); but a full
        // sequence within one chunk never leaks.
        assert!(!tail(&["\u{1b}[1;31mred"]).contains('\u{1b}'));
    }

    #[test]
    fn tail_is_bounded() {
        let mut buf = String::new();
        for i in 0..200 {
            append_terminal_chunk(&mut buf, &format!("line {i}\n"));
        }
        assert!(buf.lines().count() <= TERMINAL_TAIL_MAX_LINES);
        assert!(buf.ends_with("line 199\n"));
        let mut big = String::new();
        append_terminal_chunk(&mut big, &"x".repeat(TERMINAL_TAIL_MAX_CHARS * 2));
        assert!(big.len() <= TERMINAL_TAIL_MAX_CHARS);
    }

    #[test]
    fn live_output_lifecycle_on_message() {
        let mut msg = ChatMessage::start_assistant("a".into());
        msg.add_tool_call("t1".into(), "execute_command".into(), "{}".into());
        assert!(msg.append_tool_output("t1", "building…\r"));
        assert!(msg.append_tool_output("t1", "built 10/100\rbuilt 100/100\n"));
        assert_eq!(msg.tool_calls[0].live_output, "built 100/100\n");
        // Unknown / already-finished calls report false.
        assert!(!msg.append_tool_output("nope", "x"));
        msg.set_tool_result("t1", "ok".into(), false, Some(10));
        assert!(msg.tool_calls[0].live_output.is_empty());
        assert!(!msg.append_tool_output("t1", "late chunk"));
    }
}

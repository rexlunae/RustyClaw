//! Component data for chat message bubbles and tool call panels.
//!
//! These types represent the exact slice of data that `MessageBubble`
//! and `ToolCallPanel` need to render — distinct from the canonical
//! [`ChatMessage`] / [`ToolCallInfo`] models in `rustyclaw_core::ui`.
//!
//! The key difference: a `ChatMessage` owns tool calls and carries
//! enough state for translation from `GatewayEvent`. A `MessageBubbleData`
//! is the *rendered* view of just the bubble — no tool calls, no
//! intermediate state, no back-reference to the canonical message.
//! Tool calls are a separate [`ToolCallData`] component rendered alongside.

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use rustyclaw_core::types::MessageRole;

use crate::tone::Tone;

// ── Message bubble ──────────────────────────────────────────────────────────

/// Everything a message-bubble component needs to render.
///
/// Used by both the desktop (Dioxus `MessageBubble`) and TUI
/// (iocraft `MessageBubble`) as the single source of truth for
/// rendering data.  Event handlers (click, long-press, etc.) are
/// provided by the framework-specific wrapper.
///
/// Methods on this struct centralise display logic so that both
/// clients derive the same labels, icons, and content transformations
/// without duplicating match arms.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageBubbleData {
    /// Who sent this message (User, Assistant, System, etc.).
    pub role: MessageRole,

    /// The message text content (plain or markdown, depending on role).
    pub content: String,

    /// When the message was created.
    ///
    /// Optional — the TUI does not track per-message timestamps.
    pub timestamp: Option<DateTime<Utc>>,

    /// Whether this message is still being streamed.
    pub is_streaming: bool,

    /// Display name override for assistant messages.
    pub agent_name: Option<String>,

    /// Whether this message has extended structured details
    /// (request URL, headers, body excerpt) accessible via a
    /// "show details" action.
    pub has_details: bool,
    pub collapsed: bool,

    /// Wall-clock duration of the activity this message represents,
    /// in milliseconds. Set for Thinking messages (measured client-side
    /// between ThinkingStart and ThinkingEnd) so the header can say
    /// "Thought for 4.2s".
    pub duration_ms: Option<u64>,
}

impl Default for MessageBubbleData {
    fn default() -> Self {
        Self {
            role: MessageRole::System,
            content: String::new(),
            timestamp: None,
            is_streaming: false,
            agent_name: None,
            has_details: false,
            collapsed: false,
            duration_ms: None,
        }
    }
}

impl MessageBubbleData {
    /// Build from a canonical [`rustyclaw_core::ui::ChatMessage`].
    ///
    /// Preserves role, content, timestamp, and streaming state.
    /// `agent_name` must be set by the caller (it depends on external
    /// state, not the message itself).
    pub fn from_chat_message(
        msg: &rustyclaw_core::ui::ChatMessage,
        agent_name: Option<String>,
    ) -> Self {
        Self {
            role: msg.role,
            content: msg.content.clone(),
            timestamp: Some(msg.timestamp),
            is_streaming: msg.is_streaming,
            agent_name,
            has_details: false,
            collapsed: false,
            duration_ms: msg.duration_ms,
        }
    }

    // ── Shared display logic ────────────────────────────────────────────

    /// The human-readable label for this message's role.
    ///
    /// Common values: "You", "Assistant", "System", "Thinking", etc.
    /// For assistant messages, [`agent_name`](Self::agent_name) takes
    /// precedence (with `"Assistant"` as fallback).
    pub fn display_name(&self) -> Cow<'_, str> {
        match self.role {
            MessageRole::User => "You".into(),
            MessageRole::Assistant => self
                .agent_name
                .as_deref()
                .filter(|n| !n.is_empty())
                .map(Cow::Borrowed)
                .unwrap_or(Cow::Borrowed("Assistant")),
            MessageRole::Info => "Info".into(),
            MessageRole::Success => "Success".into(),
            MessageRole::Warning => "Warning".into(),
            MessageRole::Error => "Error".into(),
            MessageRole::System => "System".into(),
            MessageRole::ToolCall => "Tool Call".into(),
            MessageRole::ToolResult => "Tool Result".into(),
            MessageRole::Thinking => "Thinking".into(),
        }
    }

    /// The icon/emoji associated with this message's role.
    ///
    /// Delegates to [`MessageRole::icon()`] which both clients already
    /// depend on.  Provided here for convenience so that a single method
    /// call replaces the manual match in each client.
    pub fn icon(&self) -> &'static str {
        self.role.icon()
    }

    /// The bubble header label. Same as [`display_name`](Self::display_name)
    /// except for Thinking messages, which get a summary that carries the
    /// measured duration: "Thought for 4.2s" (or "Thinking…" mid-stream).
    pub fn header_label(&self) -> Cow<'_, str> {
        if self.role == MessageRole::Thinking {
            self.thinking_summary().into()
        } else {
            self.display_name()
        }
    }

    /// One-line summary for a Thinking block, shown as its collapsed
    /// header in both clients.
    pub fn thinking_summary(&self) -> String {
        match self.duration_ms {
            Some(ms) => format!("Thought for {}", format_duration_ms(ms)),
            None if self.is_streaming => "Thinking…".to_string(),
            None => "Thought".to_string(),
        }
    }

    /// The avatar glyph for this message's role, as shown next to the
    /// bubble in graphical clients.
    pub fn avatar(&self) -> &'static str {
        match self.role {
            MessageRole::User => "🧑",
            MessageRole::Assistant | MessageRole::Thinking => "🦞",
            MessageRole::System => "⚙",
            _ => "ℹ️",
        }
    }

    /// CSS modifier identifying the role family of this bubble:
    /// `"is-user"`, `"is-assistant"`, or `"is-system"`.
    pub fn role_class(&self) -> &'static str {
        match self.role {
            MessageRole::User => "is-user",
            MessageRole::Assistant | MessageRole::Thinking => "is-assistant",
            _ => "is-system",
        }
    }

    /// Whether this message should be rendered as markdown.
    ///
    /// Assistant messages that aren't still streaming get markdown
    /// rendering.  All other roles (User, System, Error, etc.)
    /// display as plain text.
    pub fn should_render_markdown(&self) -> bool {
        self.role == MessageRole::Assistant && !self.is_streaming
    }

    /// The text to display, with role-specific transformations.
    ///
    /// - **Thinking** messages are truncated at `max_chars` (default 120)
    ///   to avoid overwhelming the chat area with raw reasoning.
    /// - All other roles return the raw content unchanged.
    ///
    /// Markdown rendering is **not** applied here — that's renderer-
    /// specific (the desktop renders HTML, the TUI renders ANSI).
    /// This method only applies plain-text transformations.
    pub fn display_content(&self) -> Cow<'_, str> {
        self.display_content_truncated(120)
    }

    /// Like [`display_content`](Self::display_content) but with a
    /// custom truncation limit for thinking messages. Truncates on
    /// character boundaries — reasoning text is arbitrary UTF-8, and a
    /// byte slice could split a multi-byte character and panic.
    pub fn display_content_truncated(&self, thinking_max_chars: usize) -> Cow<'_, str> {
        if self.role == MessageRole::Thinking && self.content.chars().count() > thinking_max_chars {
            let truncated: String = self.content.chars().take(thinking_max_chars).collect();
            format!("{truncated}…").into()
        } else {
            self.content.as_str().into()
        }
    }

    pub const AUTO_COLLAPSE_LINES: usize = 40;
    pub const AUTO_COLLAPSE_CHARS: usize = 2000;
    /// Lines to show when collapsed.
    pub const COLLAPSED_PREVIEW_LINES: usize = 8;

    /// Preview length for a collapsed Thinking block's one-line gist.
    pub const THINKING_PREVIEW_CHARS: usize = 120;

    /// Whether this message is long enough to be collapsible.
    ///
    /// Thinking blocks are always collapsible (they collapse to a
    /// one-line gist rather than an 8-line preview). Other roles check
    /// byte length first (O(1)) before counting lines (O(N)).
    pub fn is_collapsible(&self) -> bool {
        if self.role == MessageRole::Thinking {
            return !self.content.trim().is_empty();
        }
        self.content.len() > Self::AUTO_COLLAPSE_CHARS
            || self.content.lines().count() > Self::AUTO_COLLAPSE_LINES
    }

    /// Content to actually render — truncated when collapsed, full otherwise.
    ///
    /// Returns a borrow in the common (uncollapsed) case to avoid allocation.
    pub fn content_for_render(&self) -> Cow<'_, str> {
        if self.role == MessageRole::Thinking {
            if !self.collapsed {
                return Cow::Borrowed(&self.content);
            }
            // Collapsed reasoning: a single dim gist line keeps the
            // transcript compact while hinting at what was considered.
            let first = self
                .content
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let mut gist: String = first.chars().take(Self::THINKING_PREVIEW_CHARS).collect();
            if gist.len() < first.len() || self.content.lines().count() > 1 {
                gist.push('…');
            }
            return format!("{gist} (Ctrl+E to expand)").into();
        }
        if self.collapsed && self.is_collapsible() {
            let lines: Vec<&str> = self
                .content
                .lines()
                .take(Self::COLLAPSED_PREVIEW_LINES)
                .collect();
            let preview = lines.join("\n");
            let hidden = self
                .content
                .lines()
                .count()
                .saturating_sub(Self::COLLAPSED_PREVIEW_LINES);
            format!("{preview}\n\n… {hidden} lines hidden (Ctrl+E to expand)").into()
        } else {
            Cow::Borrowed(&self.content)
        }
    }
}

// ── Tool call panel ─────────────────────────────────────────────────────────

/// Everything a tool-call panel component needs to render.
///
/// Represented as a component separate from the message bubble —
/// each message may have zero or more tool calls, and in both the
/// desktop and TUI they render as distinct nested elements.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallData {
    /// Unique tool call identifier (matches approval flow).
    pub id: String,

    /// Tool name, shown as the panel header.
    pub name: String,

    /// Pretty-printed JSON arguments.
    pub arguments: String,

    /// Optional result returned by the tool.
    pub result: Option<String>,

    /// Whether the tool returned an error.
    pub is_error: bool,

    /// Whether the panel starts collapsed.
    pub collapsed: bool,

    /// Wall-clock execution time in milliseconds, measured client-side
    /// between the ToolCall and ToolResult events. None while running
    /// or for calls replayed from history (which carries no timings).
    pub duration_ms: Option<u64>,

    /// Live execution status streamed by the gateway while the call is
    /// still running (cleared when the result arrives).
    pub live_status: Option<rustyclaw_core::ui::ToolLiveStatus>,
    /// Live output tail streamed while the tool runs (CR-overwrites
    /// applied, ANSI stripped, bounded). Cleared when the result arrives.
    pub live_output: String,
}

impl Default for ToolCallData {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
            result: None,
            is_error: false,
            collapsed: true,
            duration_ms: None,
            live_status: None,
            live_output: String::new(),
        }
    }
}

impl ToolCallData {
    /// Whether the call is still executing (no result yet).
    pub fn is_running(&self) -> bool {
        self.result.is_none()
    }

    /// The last `max_lines` lines of the live output tail, for rendering
    /// under a running tool's header. None when nothing has streamed yet.
    pub fn live_tail(&self, max_lines: usize) -> Option<String> {
        let text = self.live_output.trim_end_matches(['\r', '\n']);
        if text.trim().is_empty() {
            return None;
        }
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(max_lines);
        Some(lines[start..].join("\n"))
    }

    /// A short summary line for this tool call.
    ///
    /// e.g. `"🔧 web_search"` or `"🔧 write_file (error)"`
    pub fn summary(&self) -> String {
        if self.is_error {
            format!("🔧 {} (error)", self.name)
        } else {
            format!("🔧 {}", self.name)
        }
    }

    /// Execution status as `(css_class, label, icon)`, shared by every client so
    /// the wording/icons stay consistent: running → `("is-running", "Running…",
    /// "⏳")`, completed → `("is-done", "Done", "✓")`, failed → `("is-error",
    /// "Failed", "✕")`.
    pub fn status_label(&self) -> (&'static str, &'static str, &'static str) {
        if self.result.is_some() {
            if self.is_error {
                ("is-error", "Failed", "✕")
            } else {
                ("is-done", "Done", "✓")
            }
        } else {
            ("is-running", "Running…", "⏳")
        }
    }

    /// Semantic tone for the status chip: running → Info, done → Success,
    /// failed → Danger.
    pub fn status_tone(&self) -> Tone {
        if self.result.is_some() {
            if self.is_error {
                Tone::Danger
            } else {
                Tone::Success
            }
        } else {
            Tone::Info
        }
    }

    /// The arguments string, truncated for display.
    ///
    /// Uses `rustyclaw_core::ui::truncate_content` to limit both
    /// character count and line count.
    pub fn arguments_preview(&self, max_chars: usize, max_lines: usize) -> String {
        rustyclaw_core::ui::truncate_content(&self.arguments, max_chars, max_lines)
    }

    /// The result string, truncated for display.
    ///
    /// Tool results can be arbitrarily large (e.g. shell output, file contents).
    /// Rendering unbounded content freezes the TUI layout engine, so we cap it.
    pub fn result_preview(&self, max_chars: usize, max_lines: usize) -> Option<String> {
        self.result
            .as_deref()
            .map(|r| rustyclaw_core::ui::truncate_content(r, max_chars, max_lines))
    }

    /// Wall-clock duration label, e.g. `"0.4s"` / `"12s"` / `"2m 03s"`.
    pub fn duration_label(&self) -> Option<String> {
        self.duration_ms.map(format_duration_ms)
    }

    /// A compact, human-readable description of what this call *does*,
    /// derived from the tool name and arguments — `read src/main.rs:10–80`
    /// rather than a raw JSON dump. Tool/argument names mirror the desktop
    /// hint mapping in `chat_transcript.rs` so both clients describe the
    /// same call the same way. Falls back to the tool name plus its most
    /// informative string argument.
    pub fn compact_action(&self) -> String {
        let args: serde_json::Value =
            serde_json::from_str(&self.arguments).unwrap_or(serde_json::Value::Null);
        let s = |key: &str| args.get(key).and_then(|v| v.as_str());
        let n = |key: &str| args.get(key).and_then(|v| v.as_u64());
        match self.name.as_str() {
            "read_file" => {
                let path = s("path").unwrap_or("?");
                match (n("start_line"), n("end_line")) {
                    (Some(a), Some(b)) => format!("read {path}:{a}–{b}"),
                    (Some(a), None) => format!("read {path}:{a}–"),
                    _ => format!("read {path}"),
                }
            }
            "write_file" => {
                let path = s("path").unwrap_or("?");
                match s("content") {
                    Some(c) => format!("write {path} ({} lines)", c.lines().count()),
                    None => format!("write {path}"),
                }
            }
            "edit_file" | "apply_patch" => format!("edit {}", s("path").unwrap_or("?")),
            "execute_command" => {
                format!("$ {}", one_line(s("command").unwrap_or("?"), 60))
            }
            "process" => {
                // Background-session management: "poll a1b2c3d4",
                // "list", "kill a1b2c3d4", …
                let action = s("action").unwrap_or("poll");
                match s("sessionId").or_else(|| s("session_id")) {
                    Some(sid) => format!("process {action} {}", short_id(sid)),
                    None => format!("process {action}"),
                }
            }
            "search_files" => match s("path") {
                Some(p) => format!(
                    "search \"{}\" in {p}",
                    one_line(s("pattern").unwrap_or("?"), 40)
                ),
                None => format!("search \"{}\"", one_line(s("pattern").unwrap_or("?"), 40)),
            },
            "find_files" | "list_directory" => {
                let what = s("pattern").or_else(|| s("path")).unwrap_or("?");
                format!("list {}", one_line(what, 50))
            }
            "web_search" => format!("web search \"{}\"", one_line(s("query").unwrap_or("?"), 50)),
            "web_fetch" | "browser" => format!("fetch {}", one_line(s("url").unwrap_or("?"), 60)),
            _ => {
                // Generic fallback: tool name plus its most informative
                // scalar argument, so even unknown tools say *something*.
                let detail = args.as_object().and_then(|o| {
                    o.values()
                        .find_map(|v| v.as_str().filter(|t| !t.trim().is_empty()))
                });
                match detail {
                    Some(d) => format!("{} {}", self.name, one_line(d, 40)),
                    None => self.name.clone(),
                }
            }
        }
    }

    /// A one-line live status readout for a still-running call, e.g.
    /// `"⏳ 12s · running · cpu 87% · mem 145 MB"`. None once the result
    /// has arrived or when no status has been received yet.
    pub fn live_status_line(&self) -> Option<String> {
        if self.result.is_some() {
            return None;
        }
        let st = self.live_status.as_ref()?;
        let mut parts = vec![format!("⏳ {}", format_duration_ms(st.elapsed_ms))];
        if let Some(msg) = st.message.as_deref().filter(|m| !m.trim().is_empty()) {
            parts.push(msg.to_string());
        }
        if let Some(state) = st.state.as_deref() {
            parts.push(state.to_string());
        }
        if let Some(cpu) = st.cpu_percent {
            parts.push(format!("cpu {cpu:.0}%"));
        }
        if let Some(mem) = st.memory_bytes {
            parts.push(format!("mem {}", format_bytes(mem)));
        }
        if let Some(pid) = st.pid {
            parts.push(format!("pid {pid}"));
        }
        Some(parts.join(" · "))
    }

    /// Whether the running call is waiting on a process the user can
    /// pause/resume/stop/kill (i.e. live status carries a PID).
    pub fn is_controllable(&self) -> bool {
        self.result.is_none() && self.live_status.as_ref().is_some_and(|s| s.pid.is_some())
    }

    /// Whether the user has paused the underlying process.
    pub fn is_process_paused(&self) -> bool {
        self.live_status.as_ref().is_some_and(|s| s.is_paused())
    }

    /// A one-line gist of what came back: the first line of an error,
    /// exit codes for shells, match/line counts for searches and reads.
    /// None while the call is still running.
    pub fn result_gist(&self) -> Option<String> {
        let r = self.result.as_deref()?;
        if self.is_error {
            return Some(one_line(r.trim(), 80));
        }
        let gist = match self.name.as_str() {
            "execute_command" => {
                // A command that yielded/backgrounded returns a JSON status
                // blob rather than shell output — report that plainly instead
                // of a misleading "1 lines".
                if let Some(g) = session_status_gist(r) {
                    return Some(g);
                }
                let exit = r.lines().rev().find_map(|line| {
                    let t = line.trim();
                    t.strip_prefix("Exit code: ")
                        .or_else(|| t.strip_prefix("exit code: "))
                        .and_then(|c| c.trim().parse::<i32>().ok())
                });
                match exit {
                    Some(c) if c != 0 => format!("exit {c}"),
                    _ => format!("{} lines", r.lines().count()),
                }
            }
            "process" => session_status_gist(r).unwrap_or_else(|| one_line(r.trim(), 60)),
            "search_files" => {
                format!(
                    "{} matches",
                    r.lines().filter(|l| !l.trim().is_empty()).count()
                )
            }
            "read_file" => format!("{} lines", r.lines().count()),
            _ => {
                let lines = r.lines().count();
                if lines > 1 {
                    format!("{lines} lines")
                } else {
                    one_line(r.trim(), 60)
                }
            }
        };
        Some(gist)
    }
}

/// Render a millisecond duration for humans: `"0.4s"`, `"12s"`, `"2m 03s"`.
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 10_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 120_000 {
        format!("{}s", ms / 1000)
    } else {
        format!("{}m {:02}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

/// Render a byte count for humans: `"512 KB"`, `"145 MB"`, `"2.3 GB"`.
pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}

/// A short, readable prefix of a session id (UUIDs are long and noisy).
fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// If a tool result is a background-session JSON status blob
/// (`{"status":"running","sessionId":"…"}` from a yielded/backgrounded
/// command or a `process` poll), summarise it — e.g. `"backgrounded a1b2c3d4"`
/// or `"exited (0) a1b2c3d4"`. Returns None for ordinary output.
///
/// A session response always carries a `sessionId`, so that field is
/// required: this deliberately does *not* match a command that merely
/// happens to emit JSON with a `status` (e.g. a health check returning
/// `{"status":"ok"}`), which should still gist by its line count.
fn session_status_gist(result: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(result.trim()).ok()?;
    let obj = v.as_object()?;
    let session = obj
        .get("sessionId")
        .or_else(|| obj.get("session_id"))
        .and_then(|s| s.as_str())?;
    let status = obj.get("status").and_then(|s| s.as_str())?;
    Some(if status == "running" {
        format!("backgrounded {}", short_id(session))
    } else {
        format!("{status} {}", short_id(session))
    })
}

/// First line of `s`, capped at `max_chars` characters, with an ellipsis
/// when anything was cut.
fn one_line(s: &str, max_chars: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    let mut out: String = first.chars().take(max_chars).collect();
    if out.chars().count() < first.chars().count() || s.lines().count() > 1 {
        out.push('…');
    }
    out
}

impl From<&rustyclaw_core::ui::ToolCallInfo> for ToolCallData {
    fn from(tc: &rustyclaw_core::ui::ToolCallInfo) -> Self {
        Self {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: rustyclaw_core::ui::pretty_print_json(&tc.arguments),
            result: tc.result.clone(),
            is_error: tc.is_error,
            collapsed: tc.collapsed,
            duration_ms: tc.duration_ms,
            live_status: tc.live_status.clone(),
            live_output: tc.live_output.clone(),
        }
    }
}

// ── Streaming indicator ─────────────────────────────────────────────────────

/// Data for the streaming progress indicator shown beneath a message
/// while the model is generating.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StreamingIndicatorData {
    /// Number of streaming chunks received so far.
    pub chunks: u32,

    /// Total bytes received across all chunks.
    pub bytes: usize,

    /// Whether the model is in thinking mode (extended reasoning).
    pub is_thinking: bool,
}

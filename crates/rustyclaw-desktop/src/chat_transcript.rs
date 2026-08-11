//! Convert RustyClaw's chat state into the `dioxus-genai-chat` data model.
//!
//! The desktop client keeps the conversation in `rustyclaw_core::ui::ChatMessage`
//! (one bubble per turn, with `tool_calls` nested and an `is_streaming` flag).
//! `ChatSurface` instead consumes a flat [`ChatTranscript`] of one-payload
//! messages. This module is the (render-time) bridge between the two; it lives in
//! the desktop crate because the crate's types pull in `dioxus`, while
//! `rustyclaw-view` stays framework-agnostic for the TUI.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use dioxus_genai_chat::{
    ChatMessage as GcChatMessage, ChatMessagePayload, ChatRole, ChatTranscript, ContextItem,
    ContextKind, Document, DocumentKind, MediaAudio, MediaImage, MessageAction, Reasoning,
    ReasoningStep, SearchMatch, StepStatus, ToolCall, ToolCallHint, ToolCallStatus, ToolResultHint,
};
use rustyclaw_core::gateway::protocol::types::MediaRef;
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::ChatMessage;
use rustyclaw_view::serde_json;
use rustyclaw_view::{ChatSurfaceData, PromptAttachment, PromptAttachmentKind};

/// Paths the desktop itself attached in this session, as canonical paths.
///
/// `MediaRef.local_path` is not client-controlled data: it is echoed back
/// from the gateway's thread history, so any party able to write a message
/// into a thread could set it. Inlining an arbitrary `local_path` would let
/// such a ref read any file on this machine into the webview. Only files the
/// user actually picked in this desktop session are ever read.
static TRUSTED_LOCAL_PATHS: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Record a file the desktop attached in this session so its media refs can
/// be rendered inline (and so its `local_path` is trusted).
pub fn trust_local_attachment(path: &str) {
    let canonical = Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    if let Ok(mut set) = TRUSTED_LOCAL_PATHS.lock() {
        set.insert(canonical);
    }
}

fn is_trusted_local_path(path: &str) -> bool {
    let canonical = Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    TRUSTED_LOCAL_PATHS
        .lock()
        .map(|set| set.contains(&canonical))
        .unwrap_or(false)
}

/// Cache of encoded data URIs keyed by (canonical path, mtime, length).
///
/// The transcript is rebuilt on every render — including every chunk of a
/// streaming reply — so reading and base64-encoding every attachment on each
/// repaint would stall the window. Files are read once per (mtime, length)
/// fingerprint; the budget bounds retained memory so a few large attachments
/// cannot pin gigabytes of base64 text.
#[derive(Default)]
struct DataUriCache {
    entries: HashMap<(PathBuf, u64, u64), String>,
    total_bytes: usize,
}

impl DataUriCache {
    const MAX_ENTRIES: usize = 64;
    /// Budget for the sum of stored URI lengths (base64 text). A 25 MB file
    /// becomes ~33 MB of text, so this keeps worst-case retention at the
    /// budget plus one entry rather than ~2 GB at 64 × 33 MB.
    const MAX_BYTES: usize = 64 * 1024 * 1024;

    fn get(&self, key: &(PathBuf, u64, u64)) -> Option<String> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: (PathBuf, u64, u64), uri: String) {
        let incoming = uri.len();
        // Crossing either bound resets the cache. A single over-budget entry
        // (impossible while the per-file cap holds) would still be kept alone
        // rather than dropped, so the read work is never wasted.
        if self.total_bytes + incoming > Self::MAX_BYTES && !self.entries.is_empty() {
            self.entries.clear();
            self.total_bytes = 0;
        }
        if self.entries.len() >= Self::MAX_ENTRIES {
            self.entries.clear();
            self.total_bytes = 0;
        }
        self.total_bytes += incoming;
        self.entries.insert(key, uri);
    }
}

static DATA_URI_CACHE: LazyLock<Mutex<DataUriCache>> =
    LazyLock::new(|| Mutex::new(DataUriCache::default()));

const INLINE_CAP_BYTES: u64 = 25 * 1024 * 1024;

fn mtime_nanos(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t: SystemTime| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Build the transcript shown by `ChatSurface` from the live message list and
/// the current busy state. `awaiting_user` is set while an `ask_user` prompt
/// is on screen, so the busy row reads as waiting rather than working.
pub fn to_transcript(
    messages: &[ChatMessage],
    surface: &ChatSurfaceData,
    awaiting_user: bool,
) -> ChatTranscript {
    let mut transcript = ChatTranscript::default();

    for msg in messages {
        push_message(&mut transcript, msg);
    }

    // A trailing busy line, mirroring the old StreamingProgress/Thinking row:
    // thinking before any tokens arrive, then a streaming/processing status.
    // While a live reasoning block is on screen its own "Thinking…" header
    // is the indicator, so don't stack a typing row underneath it.
    if awaiting_user {
        push_auxiliary(
            &mut transcript,
            ChatRole::Assistant,
            ChatMessagePayload::Status("Waiting for your answer…".to_string()),
        );
    } else if surface.is_thinking {
        let live_reasoning = messages
            .last()
            .map(|m| m.role == MessageRole::Thinking && m.is_streaming)
            .unwrap_or(false);
        if !live_reasoning {
            push_auxiliary(
                &mut transcript,
                ChatRole::Assistant,
                ChatMessagePayload::Typing,
            );
        }
    } else if surface.is_streaming {
        let label = surface
            .progress_summary()
            .unwrap_or_else(|| "Streaming…".to_string());
        push_auxiliary(
            &mut transcript,
            ChatRole::Assistant,
            ChatMessagePayload::Status(label),
        );
    } else if surface.is_processing {
        push_auxiliary(
            &mut transcript,
            ChatRole::Assistant,
            ChatMessagePayload::Status("Processing…".to_string()),
        );
    }

    transcript
}

/// Push an entry that is not backed by a thread message (busy rows, status
/// lines): copyable, but never deletable — there is no record to delete.
fn push_auxiliary(transcript: &mut ChatTranscript, role: ChatRole, payload: ChatMessagePayload) {
    transcript.messages.push(GcChatMessage {
        role,
        payload,
        id: None,
        actions: copy_only_actions(),
    });
}

/// Action set for entries with no deletable record: copy, nothing else.
fn copy_only_actions() -> Vec<MessageAction> {
    vec![MessageAction::copy()]
}

/// Resolve a media ref to a source the webview can load directly:
/// - a `url` (remote media, e.g. messenger CDN) is used as-is — this is the
///   only source for refs that carry no local path, which is exactly the
///   shape gateway/messenger-supplied remote media has,
/// - a `local_path` this desktop itself attached becomes a `data:` URI
///   (read once and cached; only files the user picked are trusted — a
///   `local_path` echoed from thread history is not client-controlled), and
///   any failure (untrusted, missing, unreadable, over-cap) falls back to
///   the URL rather than dropping the media,
/// - otherwise `None` — the caller falls back to a name-only document tile.
///
/// Files larger than the cap are not inlined: a multi-hundred-MB data URI
/// would freeze the webview. The cap is enforced against the file's actual
/// length (via metadata), never the optional recorded `size`. They still
/// render as a downloadable document.
fn media_src(media: &MediaRef) -> Option<String> {
    let Some(path) = media.local_path.as_deref() else {
        return media.url.clone();
    };
    if !is_trusted_local_path(path) {
        return media.url.clone();
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return media.url.clone();
    };
    if !meta.is_file() || meta.len() > INLINE_CAP_BYTES {
        return media.url.clone();
    }

    let key = (
        Path::new(path).to_path_buf(),
        mtime_nanos(&meta),
        meta.len(),
    );
    if let Some(uri) = DATA_URI_CACHE.lock().ok().and_then(|c| c.get(&key)) {
        return Some(uri);
    }

    let Ok(bytes) = std::fs::read(path) else {
        return media.url.clone();
    };
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let uri = format!("data:{};base64,{}", media.mime_type, encoded);

    if let Ok(mut cache) = DATA_URI_CACHE.lock() {
        cache.insert(key, uri.clone());
    }
    Some(uri)
}

/// Map one media ref onto a `ChatMessagePayload` for the transcript.
fn media_payload(media: &MediaRef, message_id: Option<&str>, index: usize) -> ChatMessagePayload {
    let name = media
        .filename
        .clone()
        .unwrap_or_else(|| format!("attachment-{}", index + 1));

    match media_src(media) {
        Some(src) if media.mime_type.starts_with("image/") => {
            return ChatMessagePayload::Image(MediaImage::new(src).with_alt(name));
        }
        Some(src) if media.mime_type.starts_with("audio/") => {
            return ChatMessagePayload::Audio(MediaAudio::new(src).with_title(name));
        }
        _ => {}
    }

    // Everything else (or media whose bytes aren't reachable) renders as a
    // document tile: icon + name, expanding to a link when a URL exists.
    // Untrusted local paths (echoed from thread history) are never used as
    // the link target — only URLs and this session's own attachments.
    let doc_url = media.url.clone().or_else(|| {
        media
            .local_path
            .as_deref()
            .filter(|p| is_trusted_local_path(p))
            .map(ToString::to_string)
    });
    let doc = Document {
        id: message_id
            .map(|id| format!("{id}-{index}"))
            .unwrap_or_else(|| format!("media-{index}")),
        name,
        kind: DocumentKind::Other,
        image: None,
        url: doc_url,
        text: None,
        data: None,
    };
    ChatMessagePayload::Documents(vec![doc])
}

/// Push one core message (text bubble + any tool calls/results) onto the transcript.
fn push_message(transcript: &mut ChatTranscript, msg: &ChatMessage) {
    // Roles whose bubbles map to a record in the thread's history get the
    // message's stable id attached, so the per-bubble actions (copy, delete)
    // can address the right record. Notices and inline status have no record
    // behind them — copy only.
    let (role, payload, deletable) = match msg.role {
        MessageRole::User => (
            ChatRole::User,
            ChatMessagePayload::Text(msg.content.clone()),
            true,
        ),
        // Assistant turns are markdown; an empty in-flight bubble that only
        // carries tool calls contributes no text payload.  Pre-sanitise the
        // source so raw-HTML attack vectors don't survive pulldown-cmark → webview.
        MessageRole::Assistant => (
            ChatRole::Assistant,
            ChatMessagePayload::Markdown(sanitize_markdown(&msg.content)),
            true,
        ),
        // Reasoning renders as a collapsible timeline: a one-line
        // "Thought for 4.2s" header that expands to the full trace,
        // one step per paragraph. Steps render as plain text, so no
        // markdown sanitisation is needed.
        MessageRole::Thinking => {
            let summary = match msg.duration_ms {
                Some(ms) => format!("Thought for {}", rustyclaw_view::format_duration_ms(ms)),
                None if msg.is_streaming => "Thinking…".to_string(),
                None => "Thought".to_string(),
            };
            (
                ChatRole::Assistant,
                ChatMessagePayload::Reasoning(Reasoning {
                    summary,
                    steps: reasoning_steps(&msg.content, msg.is_streaming),
                    collapsed: !msg.is_streaming,
                }),
                true,
            )
        }
        MessageRole::Error => (
            ChatRole::Assistant,
            ChatMessagePayload::Error(msg.content.clone()),
            false,
        ),
        // Inline notices keep their tone via an icon prefix; the crate
        // renders System rows as neutral lines, so the glyph carries the
        // severity. Full text is preserved (no truncation).
        MessageRole::Info => (
            ChatRole::System,
            ChatMessagePayload::Text(format!("ℹ️ {}", msg.content)),
            false,
        ),
        MessageRole::Success => (
            ChatRole::System,
            ChatMessagePayload::Text(format!("✅ {}", msg.content)),
            false,
        ),
        MessageRole::Warning => (
            ChatRole::System,
            ChatMessagePayload::Text(format!("⚠️ {}", msg.content)),
            false,
        ),
        // System and the (rare, usually folded) tool roles render as a
        // neutral system line.
        _ => (
            ChatRole::System,
            ChatMessagePayload::Text(msg.content.clone()),
            false,
        ),
    };

    let is_empty_text = matches!(
        &payload,
        ChatMessagePayload::Text(s) | ChatMessagePayload::Markdown(s) if s.is_empty()
    );
    if !is_empty_text {
        let mut gc = GcChatMessage {
            role: role.clone(),
            payload,
            id: None,
            actions: Vec::new(),
        };
        if deletable {
            gc = gc.with_id(msg.id.clone());
        } else {
            gc.actions = copy_only_actions();
        }
        transcript.messages.push(gc);
    }

    // Media attachments render as their own bubbles beside the text:
    // images inline (click to enlarge), audio as a player, everything else
    // as a document gallery. All share the message's id, so delete removes
    // the whole record from wherever the bubble sits.
    for (index, media) in msg.media.iter().enumerate() {
        let payload = media_payload(media, Some(msg.id.as_str()), index);
        let mut gc = GcChatMessage {
            role: role.clone(),
            payload,
            id: None,
            actions: Vec::new(),
        };
        if deletable {
            gc = gc.with_id(msg.id.clone());
        } else {
            gc.actions = copy_only_actions();
        }
        transcript.messages.push(gc);
    }

    for tc in &msg.tool_calls {
        let status = if tc.result.is_some() {
            if tc.is_error {
                ToolCallStatus::Failed
            } else {
                ToolCallStatus::Completed
            }
        } else if msg.is_streaming {
            ToolCallStatus::Running
        } else {
            ToolCallStatus::Pending
        };
        // Arguments are stored as a JSON string; surface them as structured
        // JSON when parseable, else as a bare string.
        let arguments: serde_json::Value = serde_json::from_str(&tc.arguments)
            .unwrap_or_else(|_| serde_json::Value::String(tc.arguments.clone()));

        // The agent's structured questions (`ask_user`) read as part of the
        // conversation, not as a tool-call panel dumping the raw JSON
        // arguments. While unanswered the interactive card at the bottom of
        // the stream shows the question; once a result exists the exchange
        // is rendered here as a question bubble plus the user's answer.
        if tc.name == "ask_user" {
            push_ask_user(
                transcript,
                &arguments,
                tc.result.as_deref(),
                tc.is_error,
                if deletable {
                    Some(msg.id.clone())
                } else {
                    None
                },
            );
            continue;
        }

        let hint = tool_call_hint(&tc.name, &arguments);
        // One panel per call: the invocation, live progress, and final
        // result all render inside the same ToolCall component (no
        // separate ToolResult bubble doubling the transcript).
        //
        // While running, the streamed output tail renders as a terminal
        // block with a "running" badge, updating in place. Once done,
        // the real result takes its place.
        let result_hint = match (tc.result.as_deref(), tc.live_output.is_empty()) {
            (Some(r), _) => Some(if tc.is_error {
                ToolResultHint::Plain(r.to_string())
            } else {
                tool_result_hint(&tc.name, &arguments, r)
            }),
            (None, false) => Some(ToolResultHint::Terminal {
                exit_code: None,
                output: tc.live_output.clone(),
            }),
            (None, true) => None,
        };

        // Header label plus the measured execution time, e.g.
        // "execute_command · 2.3s". Tools whose detail isn't carried by a
        // structured hint (e.g. `process`, which renders generically) get an
        // informative base label so the header isn't a bare tool name.
        let base = tool_call_base_label(&tc.name, &arguments);
        let name = match tc.duration_ms {
            Some(ms) => format!("{} · {}", base, rustyclaw_view::format_duration_ms(ms)),
            None => base,
        };
        transcript.messages.push(GcChatMessage {
            role: ChatRole::Assistant,
            payload: ChatMessagePayload::ToolCall(ToolCall {
                name,
                arguments,
                status,
                hint,
                result_hint,
            }),
            // The tool panel is part of its parent turn: actions address the
            // whole core message, not the panel.
            id: if deletable {
                Some(msg.id.clone())
            } else {
                None
            },
            actions: if deletable {
                Vec::new()
            } else {
                copy_only_actions()
            },
        });
        if tc.result.is_none() {
            // While the call runs, surface the gateway's live status
            // (elapsed, CPU, scheduler state) as a line under the panel.
            if let Some(line) = rustyclaw_view::ToolCallData::from(tc).live_status_line() {
                push_auxiliary(transcript, ChatRole::System, ChatMessagePayload::Text(line));
            }
        }
    }
}

/// The exact string the gateway returns when a prompt is dismissed
/// (see `execute_user_prompt` in rustyclaw-gateway); rendered as a muted
/// notice rather than as a user answer bubble.
const PROMPT_DISMISSED_RESULT: &str = "User dismissed the prompt without answering.";

/// Render an `ask_user` tool call as a chat exchange: the question as an
/// assistant bubble (title, description, options) and, when present, the
/// answer as a user bubble. Nothing is emitted while the answer is still
/// pending — the interactive inline card shows the question until then.
///
/// The bubbles carry their parent message's id, so deleting either removes
/// the whole turn from the record.
fn push_ask_user(
    transcript: &mut ChatTranscript,
    args: &serde_json::Value,
    result: Option<&str>,
    is_error: bool,
    parent_id: Option<String>,
) {
    let Some(result) = result else {
        return;
    };

    let title = str_field(args, "title").unwrap_or_else(|| "Question".to_string());
    let mut md = format!("💬 **{}**", title);
    if let Some(desc) = str_field(args, "description").filter(|d| !d.is_empty()) {
        md.push_str("\n\n");
        md.push_str(&desc);
    }
    if let Some(options) = args.get("options").and_then(|v| v.as_array()) {
        for opt in options {
            let label = opt
                .as_str()
                .map(String::from)
                .or_else(|| opt.get("label").and_then(|v| v.as_str()).map(String::from));
            if let Some(label) = label {
                md.push_str(&format!("\n- {}", label));
                if let Some(desc) = opt.get("description").and_then(|v| v.as_str()) {
                    md.push_str(&format!(" — {}", desc));
                }
            }
        }
    }
    let question = GcChatMessage {
        role: ChatRole::Assistant,
        payload: ChatMessagePayload::Markdown(sanitize_markdown(&md)),
        id: parent_id.clone(),
        actions: Vec::new(),
    };
    transcript.messages.push(question);

    let (role, text) = if is_error {
        (ChatRole::System, format!("⚠️ {}", result))
    } else if result == PROMPT_DISMISSED_RESULT {
        (ChatRole::System, format!("ℹ️ {}", result))
    } else {
        // The structured answer is the user's reply in the conversation.
        (ChatRole::User, result.to_string())
    };
    let is_user_answer = role == ChatRole::User;
    let mut answer = GcChatMessage {
        role,
        payload: ChatMessagePayload::Text(text),
        id: parent_id,
        actions: Vec::new(),
    };
    // A system note (dismissed/error) has no deletable record.
    if !is_user_answer {
        answer.id = None;
        answer.actions = copy_only_actions();
    }
    transcript.messages.push(answer);
}

/// Split accumulated reasoning text into timeline steps, one per paragraph:
/// the first line becomes the step title, the rest its detail. While the
/// block is still streaming the last step is marked Active.
fn reasoning_steps(content: &str, is_streaming: bool) -> Vec<ReasoningStep> {
    let mut steps: Vec<ReasoningStep> = content
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|para| {
            let mut lines = para.lines();
            let first = lines.next().unwrap_or("").trim();
            let title: String = if first.chars().count() > 100 {
                let mut t: String = first.chars().take(100).collect();
                t.push('…');
                t
            } else {
                first.to_string()
            };
            let detail = lines.collect::<Vec<_>>().join("\n");
            let step = ReasoningStep::new(title, StepStatus::Done);
            if detail.trim().is_empty() {
                step
            } else {
                step.with_detail(detail)
            }
        })
        .collect();
    if is_streaming && let Some(last) = steps.last_mut() {
        last.status = StepStatus::Active;
    }
    steps
}

// ── Markdown sanitisation ────────────────────────────────────────────────────
//
// `dioxus-genai-chat` renders Markdown via pulldown-cmark straight into
// `dangerous_inner_html`.  pulldown-cmark passes raw HTML through verbatim,
// so an adversarial or hallucinated LLM response could inject `<script>`,
// `<iframe>`, event-handler attributes, or `javascript:` links into the
// webview.
//
// We use `ammonia` (a DOM-aware allowlist HTML sanitiser) on the raw markdown
// source.  This handles nested-tag bypasses, HTML-entity-encoding evasion,
// and attribute-level attacks that regex-based approaches cannot cover.
// Markdown syntax (headings, bold, code fences, etc.) passes through
// unmodified because it is not HTML.  Raw HTML *outside* code fences is
// cleaned to the ammonia default allowlist (safe inline elements only).

// `ammonia` alone is not sufficient, because it runs on the markdown *source*
// and therefore never sees the `<a>` / `<img>` that pulldown-cmark builds from
// markdown link syntax — a `[x](javascript:…)` link reached the DOM untouched.
// `markdown_prep::prepare` closes that gap and also autolinks bare URLs; see
// that module for the details.
fn sanitize_markdown(src: &str) -> String {
    crate::markdown_prep::prepare(&ammonia::clean(src))
}

// ── Tool call hints ──────────────────────────────────────────────────────────
//
// Extract semantic metadata from tool name + arguments so `dioxus-genai-chat`
// can render tool-specific panels (collapsed file headers, terminal blocks,
// search match lists, etc.) instead of raw JSON dumps.

fn str_field(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn u32_field(args: &serde_json::Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

/// The header label for a tool call. Most tools use their name (their
/// arguments render on the hint line beneath), but tools that fall back to
/// the generic `Other` hint — which carries no detail — get an informative
/// label here so the header isn't just a bare name like "process".
fn tool_call_base_label(name: &str, args: &serde_json::Value) -> String {
    match name {
        "process" => {
            // Background-session management: "poll a1b2c3d4", "list", …
            let action = str_field(args, "action").unwrap_or_else(|| "poll".into());
            match str_field(args, "sessionId").or_else(|| str_field(args, "session_id")) {
                Some(sid) => format!("process {action} {}", short_session(&sid)),
                None => format!("process {action}"),
            }
        }
        _ => name.to_string(),
    }
}

/// A short, readable prefix of a session id (UUIDs are long and noisy).
fn short_session(id: &str) -> String {
    id.chars().take(8).collect()
}

fn tool_call_hint(name: &str, args: &serde_json::Value) -> ToolCallHint {
    match name {
        "read_file" => ToolCallHint::FileRead {
            path: str_field(args, "path").unwrap_or_default(),
            start_line: u32_field(args, "start_line"),
            end_line: u32_field(args, "end_line"),
        },
        "write_file" => ToolCallHint::FileWrite {
            path: str_field(args, "path").unwrap_or_default(),
            lines: str_field(args, "content").map(|c| c.lines().count() as u32),
        },
        "edit_file" | "apply_patch" => ToolCallHint::FileEdit {
            path: str_field(args, "path").unwrap_or_default(),
        },
        "execute_command" => {
            // An empty command (e.g. a backgrounded call replayed from
            // history) would render as a dangling "execute_command · " with
            // nothing after the separator, so fall back to the generic hint.
            match str_field(args, "command").filter(|c| !c.trim().is_empty()) {
                Some(command) => ToolCallHint::Shell {
                    command,
                    working_dir: str_field(args, "working_dir"),
                },
                None => ToolCallHint::Other,
            }
        }
        "search_files" => ToolCallHint::Search {
            pattern: str_field(args, "pattern").unwrap_or_default(),
            path: str_field(args, "path"),
        },
        "find_files" | "list_directory" => ToolCallHint::FindFiles {
            query: str_field(args, "pattern").unwrap_or_default(),
            path: str_field(args, "path"),
        },
        "web_search" => ToolCallHint::WebSearch {
            query: str_field(args, "query").unwrap_or_default(),
        },
        "web_fetch" | "browser" => ToolCallHint::WebFetch {
            url: str_field(args, "url").unwrap_or_default(),
        },
        "memory_search" | "memory_get" | "save_memory" | "add_memory" | "search_history" => {
            ToolCallHint::Memory {
                action: name.to_string(),
            }
        }
        _ => ToolCallHint::Other,
    }
}

fn tool_result_hint(name: &str, args: &serde_json::Value, result: &str) -> ToolResultHint {
    match name {
        "read_file" => {
            let path = str_field(args, "path").unwrap_or_default();
            let language = path.rsplit('.').next().map(String::from);
            ToolResultHint::Code {
                path,
                content: result.to_string(),
                language,
            }
        }
        "execute_command" => {
            let exit_code = result.lines().rev().find_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("Exit code: ")
                    .or_else(|| trimmed.strip_prefix("exit code: "))
                    .and_then(|s| s.trim().parse::<i32>().ok())
            });
            ToolResultHint::Terminal {
                exit_code,
                output: result.to_string(),
            }
        }
        "search_files" => {
            let matches: Vec<SearchMatch> = result
                .lines()
                .filter_map(parse_search_match)
                .take(50)
                .collect();
            if matches.is_empty() {
                ToolResultHint::Plain(result.to_string())
            } else {
                ToolResultHint::SearchMatches(matches)
            }
        }
        _ => ToolResultHint::Plain(result.to_string()),
    }
}

/// Parse a grep-style match line: `path:line:content`
fn parse_search_match(line: &str) -> Option<SearchMatch> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    // Find the first colon after a path component (skip Windows drive letters like C:)
    let after_drive = if line.len() >= 2 && line.as_bytes()[1] == b':' {
        2
    } else {
        0
    };
    let first_colon = line[after_drive..].find(':')? + after_drive;
    let rest = &line[first_colon + 1..];
    let second_colon = rest.find(':')?;
    let line_no: u32 = rest[..second_colon].parse().ok()?;
    let content = rest[second_colon + 1..].to_string();
    let path = line[..first_colon].to_string();
    Some(SearchMatch {
        path,
        line: line_no,
        content,
    })
}

/// Map prompt attachments to the chat surface's context-item model. The
/// attachment path is the stable id used when the user removes a chip.
pub fn to_context_items(attachments: &[PromptAttachment]) -> Vec<ContextItem> {
    attachments
        .iter()
        .map(|att| ContextItem {
            id: att.path.clone(),
            label: att.display_name.clone(),
            kind: match att.kind {
                PromptAttachmentKind::File => ContextKind::File,
                PromptAttachmentKind::Directory => ContextKind::Directory,
            },
        })
        .collect()
}

#[cfg(test)]
mod ask_user_tests {
    use super::*;
    use rustyclaw_core::ui::ToolCallInfo;

    /// Bubbles backed by a thread record carry their stable id and let the
    /// surface defaults (copy + delete) apply; notices and auxiliary rows
    /// carry no id and are explicitly copy-only, so delete cannot be fired
    /// against a record that does not exist.
    #[test]
    fn deletable_bubbles_carry_ids_notices_are_copy_only() {
        let mut messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::start_assistant("a-1".to_string()),
        ];
        messages[0].id = "u-1".to_string();
        messages[1].content = "hi".to_string();
        messages.push(ChatMessage::notice(
            rustyclaw_core::types::MessageRole::Warning,
            "careful".to_string(),
        ));
        messages[0].is_streaming = false;
        messages[1].is_streaming = false;

        let mut transcript = ChatTranscript::default();
        for msg in &messages {
            push_message(&mut transcript, msg);
        }
        // A busy row (no record behind it) — copy only.
        push_auxiliary(
            &mut transcript,
            ChatRole::Assistant,
            ChatMessagePayload::Typing,
        );

        assert_eq!(transcript.messages.len(), 4);
        let (user, assistant, notice, busy) = (
            &transcript.messages[0],
            &transcript.messages[1],
            &transcript.messages[2],
            &transcript.messages[3],
        );
        assert_eq!(user.id.as_deref(), Some("u-1"));
        assert!(user.actions.is_empty(), "defaults (copy+delete) apply");
        assert_eq!(assistant.id.as_deref(), Some("a-1"));
        assert!(assistant.actions.is_empty());
        assert_eq!(notice.id, None);
        assert_eq!(
            notice.actions.iter().map(|a| a.id()).collect::<Vec<_>>(),
            vec!["copy"],
            "notices are copy-only"
        );
        assert_eq!(busy.id, None);
        assert_eq!(
            busy.actions.iter().map(|a| a.id()).collect::<Vec<_>>(),
            vec!["copy"]
        );
    }

    fn ask_user_message(result: Option<&str>, is_error: bool) -> ChatMessage {
        let mut msg = ChatMessage::start_assistant("m1".to_string());
        msg.is_streaming = false;
        msg.tool_calls.push(ToolCallInfo {
            id: "call_1".to_string(),
            name: "ask_user".to_string(),
            arguments: r#"{"prompt_type":"select","title":"Pick a colour",
                "description":"Used for the theme.",
                "options":[{"label":"Red"},{"label":"Blue","description":"calm"}]}"#
                .to_string(),
            result: result.map(String::from),
            is_error,
            collapsed: false,
            duration_ms: None,
            live_status: None,
            live_output: String::new(),
        });
        msg
    }

    fn transcript_for(msg: &ChatMessage) -> ChatTranscript {
        let mut transcript = ChatTranscript::default();
        push_message(&mut transcript, msg);
        transcript
    }

    #[test]
    fn pending_question_emits_nothing() {
        // While unanswered the inline card shows the question; the
        // transcript must not render the tool call (raw JSON) at all.
        let transcript = transcript_for(&ask_user_message(None, false));
        assert!(transcript.messages.is_empty());
    }

    #[test]
    fn answered_question_renders_as_conversation() {
        let transcript = transcript_for(&ask_user_message(Some("Blue"), false));
        assert_eq!(transcript.messages.len(), 2);

        match &transcript.messages[0].payload {
            ChatMessagePayload::Markdown(md) => {
                assert!(md.contains("Pick a colour"));
                assert!(md.contains("Used for the theme."));
                assert!(md.contains("Blue — calm"));
                assert!(!md.contains("prompt_type"));
            }
            other => panic!("expected question markdown, got {other:?}"),
        }
        assert_eq!(transcript.messages[1].role, ChatRole::User);
        match &transcript.messages[1].payload {
            ChatMessagePayload::Text(answer) => assert_eq!(answer, "Blue"),
            other => panic!("expected answer text, got {other:?}"),
        }
    }

    #[test]
    fn dismissed_and_errored_prompts_render_as_notices() {
        let dismissed = transcript_for(&ask_user_message(Some(PROMPT_DISMISSED_RESULT), false));
        assert_eq!(dismissed.messages[1].role, ChatRole::System);

        let errored = transcript_for(&ask_user_message(
            Some("User prompt timed out after 5 minutes."),
            true,
        ));
        assert_eq!(errored.messages[1].role, ChatRole::System);
        match &errored.messages[1].payload {
            ChatMessagePayload::Text(text) => assert!(text.starts_with("⚠️")),
            other => panic!("expected notice text, got {other:?}"),
        }
    }

    fn tool_call_message(name: &str, arguments: &str) -> ChatMessage {
        let mut msg = ChatMessage::start_assistant("m1".to_string());
        msg.is_streaming = false;
        msg.tool_calls.push(ToolCallInfo {
            id: "call_1".to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
            result: Some("ok".to_string()),
            is_error: false,
            collapsed: false,
            duration_ms: Some(0),
            live_status: None,
            live_output: String::new(),
        });
        msg
    }

    fn first_tool_call(transcript: &ChatTranscript) -> &ToolCall {
        transcript
            .messages
            .iter()
            .find_map(|m| match &m.payload {
                ChatMessagePayload::ToolCall(tc) => Some(tc),
                _ => None,
            })
            .expect("a tool call payload")
    }

    #[test]
    fn process_tool_gets_an_informative_header() {
        let transcript = transcript_for(&tool_call_message(
            "process",
            r#"{"action":"poll","sessionId":"a1b2c3d4e5f6"}"#,
        ));
        let tc = first_tool_call(&transcript);
        // Header carries the action + short session, not a bare "process".
        assert!(
            tc.name.starts_with("process poll a1b2c3d4"),
            "got header {:?}",
            tc.name
        );
        assert!(matches!(tc.hint, ToolCallHint::Other));
    }

    #[test]
    fn execute_command_with_empty_command_has_no_dangling_separator() {
        let transcript = transcript_for(&tool_call_message("execute_command", "{}"));
        let tc = first_tool_call(&transcript);
        // An empty command falls back to the generic hint, so the crate
        // renders a clean "execute_command · <dur>" with no trailing " · ".
        assert!(matches!(tc.hint, ToolCallHint::Other));
        assert!(tc.name.starts_with("execute_command"));
    }

    #[test]
    fn execute_command_with_command_keeps_the_shell_hint() {
        let transcript = transcript_for(&tool_call_message(
            "execute_command",
            r#"{"command":"ls -la"}"#,
        ));
        let tc = first_tool_call(&transcript);
        match &tc.hint {
            ToolCallHint::Shell { command, .. } => assert_eq!(command, "ls -la"),
            other => panic!("expected Shell hint, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::gateway::protocol::types::MediaRef;

    fn user_message_with_media(media: Vec<MediaRef>) -> ChatMessage {
        let mut msg = ChatMessage::user("see attached");
        msg.id = "msg-1".to_string();
        msg.media = media;
        msg
    }

    /// A local image the desktop itself attached becomes an inline `Image`
    /// bubble with a data URI (the transcript can read the file back).
    #[test]
    fn a_local_image_becomes_an_inline_image_bubble() {
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw-media-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chart.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nfake").unwrap();

        let mut media = MediaRef::new("image/png".into());
        media.filename = Some("chart.png".into());
        media.size = Some(12);
        media.local_path = Some(path.display().to_string());
        // The desktop itself attached this file, so its local_path is trusted.
        trust_local_attachment(&path.display().to_string());

        let mut transcript = ChatTranscript::default();
        push_message(&mut transcript, &user_message_with_media(vec![media]));
        std::fs::remove_dir_all(&dir).ok();

        let bubbles: Vec<_> = transcript
            .messages
            .iter()
            .filter(|m| matches!(m.payload, ChatMessagePayload::Image(_)))
            .collect();
        assert_eq!(bubbles.len(), 1, "one image bubble");
        match &bubbles[0].payload {
            ChatMessagePayload::Image(image) => {
                assert!(image.url.starts_with("data:image/png;base64,"));
                assert_eq!(image.alt, "chart.png");
            }
            other => panic!("expected Image payload, got {other:?}"),
        }
        assert_eq!(bubbles[0].id.as_deref(), Some("msg-1"));
    }

    /// A local audio clip becomes an inline `Audio` player bubble.
    #[test]
    fn a_local_audio_clip_becomes_an_audio_player_bubble() {
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw-media-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.ogg");
        std::fs::write(&path, b"OggS fake").unwrap();

        let mut media = MediaRef::new("audio/ogg".into());
        media.filename = Some("note.ogg".into());
        media.size = Some(9);
        media.local_path = Some(path.display().to_string());
        trust_local_attachment(&path.display().to_string());

        let mut transcript = ChatTranscript::default();
        push_message(&mut transcript, &user_message_with_media(vec![media]));
        std::fs::remove_dir_all(&dir).ok();

        let bubbles: Vec<_> = transcript
            .messages
            .iter()
            .filter(|m| matches!(m.payload, ChatMessagePayload::Audio(_)))
            .collect();
        assert_eq!(bubbles.len(), 1, "one audio bubble");
        match &bubbles[0].payload {
            ChatMessagePayload::Audio(audio) => {
                assert!(audio.url.starts_with("data:audio/ogg;base64,"));
                assert_eq!(audio.title, "note.ogg");
            }
            other => panic!("expected Audio payload, got {other:?}"),
        }
    }

    /// Non-image/audio media (or media whose bytes aren't reachable) renders
    /// as a document tile instead of being dropped.
    #[test]
    fn non_inline_media_renders_as_a_document_tile() {
        let mut media = MediaRef::new("application/pdf".into());
        media.filename = Some("report.pdf".into());
        media.url = Some("https://cdn.example/report.pdf".into());

        let mut transcript = ChatTranscript::default();
        push_message(&mut transcript, &user_message_with_media(vec![media]));
        transcript
            .messages
            .iter()
            .find(|m| matches!(m.payload, ChatMessagePayload::Documents(_)))
            .expect("a document gallery bubble");
    }

    /// A gateway-side cache path the desktop cannot read falls back to the
    /// URL when one exists — messenger media keeps rendering.
    #[test]
    fn unreachable_local_path_falls_back_to_the_url() {
        let mut media = MediaRef::new("image/jpeg".into());
        media.filename = Some("photo.jpg".into());
        media.local_path = Some("/nonexistent/gateway-cache/photo.jpg".into());
        media.url = Some("https://cdn.example/photo.jpg".into());

        let src = media_src(&media).expect("url fallback");
        assert_eq!(src, "https://cdn.example/photo.jpg");
    }

    /// A `local_path` echoed back from thread history is not client
    /// controlled: the desktop must never read a file it did not itself
    /// attach, even if the path exists on this machine (SEC).
    #[test]
    fn untrusted_local_path_is_never_inlined() {
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw-media-untrusted-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secret.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nsecret").unwrap();

        // Deliberately NOT trust_local_attachment — this ref is forged.
        let mut media = MediaRef::new("image/png".into());
        media.filename = Some("secret.png".into());
        media.size = Some(14);
        media.local_path = Some(path.display().to_string());

        let src = media_src(&media);
        assert_eq!(src, None, "untrusted local path must not become a data URI");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The inline cap is enforced against the file's real length, so a ref
    /// with no recorded `size` (metadata failure, foreign client) cannot
    /// smuggle a multi-hundred-MB data URI into the webview.
    #[test]
    fn size_cap_uses_real_file_length_even_when_size_is_missing() {
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw-media-cap-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.webp");
        std::fs::write(&path, vec![0u8; (INLINE_CAP_BYTES + 1) as usize]).unwrap();
        trust_local_attachment(&path.display().to_string());

        // size is None: the old code would have inlined the whole file.
        let mut media = MediaRef::new("image/webp".into());
        media.filename = Some("big.webp".into());
        media.local_path = Some(path.display().to_string());

        let src = media_src(&media);
        assert_eq!(
            src, None,
            "over-cap file must not be inlined even with size=None"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A trusted, under-cap file whose recorded `size` is wrong still
    /// inlines — the real length is what matters.
    #[test]
    fn trusted_small_file_inlines_despite_wrong_recorded_size() {
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw-media-size-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("small.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nsmall").unwrap();
        trust_local_attachment(&path.display().to_string());

        let mut media = MediaRef::new("image/png".into());
        media.filename = Some("small.png".into());
        media.size = Some(9_999_999); // lies; real file is tiny
        media.local_path = Some(path.display().to_string());

        let src = media_src(&media).expect("small trusted file inlines");
        assert!(src.starts_with("data:image/png;base64,"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Remote media that only has a URL (messenger CDN, gateway-supplied —
    /// `MediaRef::new` leaves `local_path: None`) still renders inline as an
    /// image/audio bubble; it must not degrade to a document tile.
    #[test]
    fn url_only_remote_media_renders_inline() {
        let mut media = MediaRef::new("image/jpeg".into());
        media.filename = Some("photo.jpg".into());
        media.url = Some("https://cdn.example/photo.jpg".into());

        let src = media_src(&media).expect("url-only ref resolves to its URL");
        assert_eq!(src, "https://cdn.example/photo.jpg");

        let mut transcript = ChatTranscript::default();
        push_message(&mut transcript, &user_message_with_media(vec![media]));
        let bubbles: Vec<_> = transcript
            .messages
            .iter()
            .filter(|m| matches!(m.payload, ChatMessagePayload::Image(_)))
            .collect();
        assert_eq!(
            bubbles.len(),
            1,
            "url-only image renders as an image bubble"
        );
    }

    /// The data-URI cache bounds retained memory by total bytes, not just
    /// entry count: a handful of large attachments cannot pin gigabytes.
    #[test]
    fn data_uri_cache_bounds_total_bytes() {
        let mut cache = DataUriCache::default();
        // 4 MiB of encoded text per entry; 20 entries would be 80 MiB raw.
        let chunk = "a".repeat(4 * 1024 * 1024);
        for i in 0..20 {
            cache.insert(
                (PathBuf::from(format!("/tmp/img-{i}")), 0, 0),
                chunk.clone(),
            );
        }
        assert!(
            cache.total_bytes <= DataUriCache::MAX_BYTES + chunk.len(),
            "cache exceeded the byte budget: {} bytes",
            cache.total_bytes
        );
        assert!(
            cache.entries.len() < 20,
            "byte budget should have evicted entries"
        );
        // A fresh small insert after a reset still works (read work never wasted).
        cache.insert((PathBuf::from("/tmp/small"), 0, 0), "tiny".to_string());
        assert_eq!(
            cache.get(&(PathBuf::from("/tmp/small"), 0, 0)),
            Some("tiny".to_string())
        );
    }
}

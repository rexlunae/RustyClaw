//! Convert RustyClaw's chat state into the `dioxus-genai-chat` data model.
//!
//! The desktop client keeps the conversation in `rustyclaw_core::ui::ChatMessage`
//! (one bubble per turn, with `tool_calls` nested and an `is_streaming` flag).
//! `ChatSurface` instead consumes a flat [`ChatTranscript`] of one-payload
//! messages. This module is the (render-time) bridge between the two; it lives in
//! the desktop crate because the crate's types pull in `dioxus`, while
//! `rustyclaw-view` stays framework-agnostic for the TUI.

use dioxus_genai_chat::{
    ChatMessagePayload, ChatRole, ChatTranscript, ContextItem, ContextKind, Reasoning,
    ReasoningStep, SearchMatch, StepStatus, ToolCall, ToolCallHint, ToolCallStatus, ToolResultHint,
};
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::ChatMessage;
use rustyclaw_view::serde_json;
use rustyclaw_view::{ChatSurfaceData, PromptAttachment, PromptAttachmentKind};

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
        transcript.push(
            ChatRole::Assistant,
            ChatMessagePayload::Status("Waiting for your answer…".to_string()),
        );
    } else if surface.is_thinking {
        let live_reasoning = messages
            .last()
            .map(|m| m.role == MessageRole::Thinking && m.is_streaming)
            .unwrap_or(false);
        if !live_reasoning {
            transcript.push(ChatRole::Assistant, ChatMessagePayload::Typing);
        }
    } else if surface.is_streaming {
        let label = surface
            .progress_summary()
            .unwrap_or_else(|| "Streaming…".to_string());
        transcript.push(ChatRole::Assistant, ChatMessagePayload::Status(label));
    } else if surface.is_processing {
        transcript.push(
            ChatRole::Assistant,
            ChatMessagePayload::Status("Processing…".to_string()),
        );
    }

    transcript
}

/// Push one core message (text bubble + any tool calls/results) onto the transcript.
fn push_message(transcript: &mut ChatTranscript, msg: &ChatMessage) {
    let (role, payload) = match msg.role {
        MessageRole::User => (
            ChatRole::User,
            ChatMessagePayload::Text(msg.content.clone()),
        ),
        // Assistant turns are markdown; an empty in-flight bubble that only
        // carries tool calls contributes no text payload.  Pre-sanitise the
        // source so raw-HTML attack vectors don't survive pulldown-cmark → webview.
        MessageRole::Assistant => (
            ChatRole::Assistant,
            ChatMessagePayload::Markdown(sanitize_markdown(&msg.content)),
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
            )
        }
        MessageRole::Error => (
            ChatRole::Assistant,
            ChatMessagePayload::Error(msg.content.clone()),
        ),
        // Inline notices keep their tone via an icon prefix; the crate
        // renders System rows as neutral lines, so the glyph carries the
        // severity. Full text is preserved (no truncation).
        MessageRole::Info => (
            ChatRole::System,
            ChatMessagePayload::Text(format!("ℹ️ {}", msg.content)),
        ),
        MessageRole::Success => (
            ChatRole::System,
            ChatMessagePayload::Text(format!("✅ {}", msg.content)),
        ),
        MessageRole::Warning => (
            ChatRole::System,
            ChatMessagePayload::Text(format!("⚠️ {}", msg.content)),
        ),
        // System and the (rare, usually folded) tool roles render as a
        // neutral system line.
        _ => (
            ChatRole::System,
            ChatMessagePayload::Text(msg.content.clone()),
        ),
    };

    let is_empty_text = matches!(
        &payload,
        ChatMessagePayload::Text(s) | ChatMessagePayload::Markdown(s) if s.is_empty()
    );
    if !is_empty_text {
        transcript.push(role, payload);
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
            push_ask_user(transcript, &arguments, tc.result.as_deref(), tc.is_error);
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
        transcript.push(
            ChatRole::Assistant,
            ChatMessagePayload::ToolCall(ToolCall {
                name,
                arguments,
                status,
                hint,
                result_hint,
            }),
        );
        if tc.result.is_none() {
            // While the call runs, surface the gateway's live status
            // (elapsed, CPU, scheduler state) as a line under the panel.
            if let Some(line) = rustyclaw_view::ToolCallData::from(tc).live_status_line() {
                transcript.push(ChatRole::System, ChatMessagePayload::Text(line));
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
fn push_ask_user(
    transcript: &mut ChatTranscript,
    args: &serde_json::Value,
    result: Option<&str>,
    is_error: bool,
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
    transcript.push(
        ChatRole::Assistant,
        ChatMessagePayload::Markdown(sanitize_markdown(&md)),
    );

    if is_error {
        transcript.push(
            ChatRole::System,
            ChatMessagePayload::Text(format!("⚠️ {}", result)),
        );
    } else if result == PROMPT_DISMISSED_RESULT {
        transcript.push(
            ChatRole::System,
            ChatMessagePayload::Text(format!("ℹ️ {}", result)),
        );
    } else {
        // The structured answer is the user's reply in the conversation.
        transcript.push(ChatRole::User, ChatMessagePayload::Text(result.to_string()));
    }
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

fn sanitize_markdown(src: &str) -> String {
    ammonia::clean(src)
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

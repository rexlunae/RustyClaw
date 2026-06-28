//! Convert RustyClaw's chat state into the `dioxus-genai-chat` data model.
//!
//! The desktop client keeps the conversation in `rustyclaw_core::ui::ChatMessage`
//! (one bubble per turn, with `tool_calls` nested and an `is_streaming` flag).
//! `ChatSurface` instead consumes a flat [`ChatTranscript`] of one-payload
//! messages. This module is the (render-time) bridge between the two; it lives in
//! the desktop crate because the crate's types pull in `dioxus`, while
//! `rustyclaw-view` stays framework-agnostic for the TUI.

use dioxus_genai_chat::{
    ChatMessagePayload, ChatRole, ChatTranscript, ContextItem, ContextKind, SearchMatch, ToolCall,
    ToolCallHint, ToolCallStatus, ToolResultHint,
};
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::ChatMessage;
use rustyclaw_view::serde_json;
use rustyclaw_view::{ChatSurfaceData, PromptAttachment, PromptAttachmentKind};

/// Build the transcript shown by `ChatSurface` from the live message list and
/// the current busy state.
pub fn to_transcript(messages: &[ChatMessage], surface: &ChatSurfaceData) -> ChatTranscript {
    let mut transcript = ChatTranscript::default();

    for msg in messages {
        push_message(&mut transcript, msg);
    }

    // A trailing busy line, mirroring the old StreamingProgress/Thinking row:
    // thinking before any tokens arrive, then a streaming/processing status.
    if surface.is_thinking {
        transcript.push(ChatRole::Assistant, ChatMessagePayload::Typing);
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
        MessageRole::Assistant | MessageRole::Thinking => (
            ChatRole::Assistant,
            ChatMessagePayload::Markdown(sanitize_markdown(&msg.content)),
        ),
        MessageRole::Error => (
            ChatRole::Assistant,
            ChatMessagePayload::Error(msg.content.clone()),
        ),
        // Info / Success / Warning / System and the (rare, usually folded)
        // tool roles all render as a neutral system line.
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

        let hint = tool_call_hint(&tc.name, &arguments);
        let result_hint = tc.result.as_deref().map(|r| {
            if tc.is_error {
                ToolResultHint::Plain(r.to_string())
            } else {
                tool_result_hint(&tc.name, &arguments, r)
            }
        });

        transcript.push(
            ChatRole::Assistant,
            ChatMessagePayload::ToolCall(ToolCall {
                name: tc.name.clone(),
                arguments,
                status,
                hint,
                result_hint,
            }),
        );
        if let Some(result) = &tc.result {
            transcript.push(
                ChatRole::Tool,
                ChatMessagePayload::ToolResult {
                    name: tc.name.clone(),
                    content: result.clone(),
                },
            );
        }
    }
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
        "execute_command" => ToolCallHint::Shell {
            command: str_field(args, "command").unwrap_or_default(),
            working_dir: str_field(args, "working_dir"),
        },
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

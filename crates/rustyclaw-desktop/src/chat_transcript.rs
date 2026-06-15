//! Convert RustyClaw's chat state into the `dioxus-genai-chat` data model.
//!
//! The desktop client keeps the conversation in `rustyclaw_core::ui::ChatMessage`
//! (one bubble per turn, with `tool_calls` nested and an `is_streaming` flag).
//! `ChatSurface` instead consumes a flat [`ChatTranscript`] of one-payload
//! messages. This module is the (render-time) bridge between the two; it lives in
//! the desktop crate because the crate's types pull in `dioxus`, while
//! `rustyclaw-view` stays framework-agnostic for the TUI.

use dioxus_genai_chat::{
    ChatMessagePayload, ChatRole, ChatTranscript, ContextItem, ContextKind, ToolCall,
    ToolCallStatus,
};
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::ChatMessage;
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
<<<<<<< HEAD
        // carries tool calls contributes no text payload.
        MessageRole::Assistant | MessageRole::Thinking => (
            ChatRole::Assistant,
            ChatMessagePayload::Markdown(msg.content.clone()),
=======
        // carries tool calls contributes no text payload.  Pre-sanitise the
        // source so raw-HTML attack vectors don't survive pulldown-cmark → webview.
        MessageRole::Assistant | MessageRole::Thinking => (
            ChatRole::Assistant,
            ChatMessagePayload::Markdown(sanitize_markdown(&msg.content)),
>>>>>>> origin/main
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
        let arguments = serde_json::from_str(&tc.arguments)
            .unwrap_or_else(|_| serde_json::Value::String(tc.arguments.clone()));
        transcript.push(
            ChatRole::Assistant,
            ChatMessagePayload::ToolCall(ToolCall {
                name: tc.name.clone(),
                arguments,
                status,
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

<<<<<<< HEAD
=======
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

>>>>>>> origin/main
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

//! Message list: the scrolled region showing messages, empty state,
//! and streaming/thinking/processing indicators.
//!
//! This component is the messages-area analogue of the TUI's
//! `components/messages.rs`, refactored out of `chat.rs` during the
//! Phase D structural alignment.  Sub-components:
//!   - [`Messages`] — the public composite
//!   - [`EmptyState`] — starter prompts when no messages exist
//!   - [`ThinkingIndicator`] — animated "Thinking…" row
//!   - [`ProcessingIndicator`] — hourglass shown before streaming starts
//!   - [`StreamingProgress`] — live chunk/byte counter during streaming

use dioxus::prelude::*;
use rustyclaw_core::ui::{ChatMessage, StreamingState};

use super::message::MessageBubble;
use super::tool_call::ToolCallPanel;

/// (provider_id, model_id) pair emitted when the user changes model.
pub type ModelSelection = (String, String);

/// Suggested starter prompts shown on the empty state.
const STARTERS: &[(&str, &str, &str)] = &[
    (
        "🔍",
        "Explore the system",
        "What can you do? List your available tools and capabilities.",
    ),
    (
        "📚",
        "Summarise a topic",
        "Give me a concise overview of how RustyClaw secures agent runtimes.",
    ),
    (
        "🛠️",
        "Run a quick task",
        "Help me draft a TOML config for connecting to a local Ollama provider.",
    ),
    (
        "🧠",
        "Think out loud",
        "Walk me through how you'd debug a failing tool call step by step.",
    ),
];

/// Props for [`Messages`].
#[derive(Props, Clone, PartialEq)]
pub struct MessagesProps {
    pub messages: Vec<ChatMessage>,
    pub is_processing: bool,
    pub is_thinking: bool,
    pub is_streaming: bool,
    pub streaming_chunks: u32,
    pub streaming_bytes: usize,
    pub agent_name: Option<String>,
    pub on_starter_pick: EventHandler<String>,
}

/// Message list region with empty state, indicators, and auto-scroll.
#[component]
pub fn Messages(props: MessagesProps) -> Element {
    let has_messages = !props.messages.is_empty();

    rsx! {
        if !has_messages && !props.is_thinking {
            EmptyState {
                agent_name: props.agent_name.clone(),
                on_pick: props.on_starter_pick,
            }
        } else {
            div { class: "messages",
                div { class: "messages-inner",
                    for msg in props.messages.iter() {
                        div { key: "{msg.id}",
                            MessageBubble {
                                data: rustyclaw_view::MessageBubbleData::from_chat_message(
                                    msg,
                                    props.agent_name.clone(),
                                ),
                            }
                            for tool in msg.tool_calls.iter() {
                                ToolCallPanel {
                                    key: "{tool.id}",
                                    id: tool.id.clone(),
                                    name: tool.name.clone(),
                                    arguments: tool.arguments.clone(),
                                    result: tool.result.clone(),
                                    is_error: tool.is_error,
                                    collapsed: tool.collapsed,
                                }
                            }
                        }
                    }

                    if props.is_thinking {
                        ThinkingIndicator {
                            agent_name: props.agent_name.clone(),
                        }
                    }

                    if props.is_streaming {
                        StreamingProgress {
                            chunks: props.streaming_chunks,
                            bytes: props.streaming_bytes,
                        }
                    } else if props.is_processing {
                        ProcessingIndicator {}
                    }
                }
            }
        }
    }
}

// ── Empty state ─────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct EmptyStateProps {
    agent_name: Option<String>,
    on_pick: EventHandler<String>,
}

#[component]
fn EmptyState(props: EmptyStateProps) -> Element {
    let greeting = match props.agent_name.as_deref() {
        Some(name) if !name.is_empty() => format!("How can {name} help you today?"),
        _ => "How can I help you today?".to_string(),
    };

    rsx! {
        div { class: "empty-state",
            div { class: "empty-state-card",
                div { class: "empty-state-mark", "🦞" }
                h2 { "{greeting}" }
                p { "Pick a starter or just type below to begin." }
                div { class: "starter-grid",
                    for (icon, title, prompt) in STARTERS.iter() {
                        button {
                            key: "{title}",
                            class: "starter-card",
                            onclick: {
                                let p = (*prompt).to_string();
                                let cb = props.on_pick;
                                move |_| cb.call(p.clone())
                            },
                            div { class: "starter-title",
                                span { "{icon}" }
                                span { "{title}" }
                            }
                            div { class: "starter-sub", "{prompt}" }
                        }
                    }
                }
            }
        }
    }
}

// ── Thinking indicator ──────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct ThinkingIndicatorProps {
    agent_name: Option<String>,
}

#[component]
fn ThinkingIndicator(props: ThinkingIndicatorProps) -> Element {
    rsx! {
        div { class: "msg-row is-assistant",
            div { class: "msg-avatar", "🦞" }
            div { class: "msg-body",
                div { class: "msg-header",
                    span { class: "msg-name",
                        {props.agent_name.as_deref().unwrap_or("Assistant")}
                    }
                }
                div { class: "thinking",
                    span { "Thinking" }
                    span { class: "thinking-dots",
                        span {} span {} span {}
                    }
                }
            }
        }
    }
}

// ── Processing indicator ───────────────────────────────────────────────────

#[component]
fn ProcessingIndicator() -> Element {
    rsx! {
        div { class: "streaming-progress",
            span { class: "streaming-progress-icon", "⏳" }
            span { class: "streaming-progress-text", "Processing…" }
        }
    }
}

// ── Streaming progress ──────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct StreamingProgressProps {
    chunks: u32,
    bytes: usize,
}

#[component]
fn StreamingProgress(props: StreamingProgressProps) -> Element {
    let state = StreamingState {
        is_streaming: true,
        is_thinking: false,
        chunks: props.chunks,
        bytes: props.bytes,
        start_time: None,
    };
    let label = state.progress_summary();

    rsx! {
        div { class: "streaming-progress",
            span { class: "streaming-progress-icon streaming-pulse" }
            span { class: "streaming-progress-text", "{label}" }
        }
    }
}

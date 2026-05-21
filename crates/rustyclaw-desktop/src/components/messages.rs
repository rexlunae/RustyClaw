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
use rustyclaw_core::ui::ChatMessage;

use super::message::MessageBubble;
use super::tool_call::ToolCallPanel;

/// (provider_id, model_id) pair emitted when the user changes model.
pub type ModelSelection = (String, String);

/// Props for [`Messages`].
#[derive(Props, Clone, PartialEq)]
pub struct MessagesProps {
    pub messages: Vec<ChatMessage>,
    pub surface: rustyclaw_view::ChatSurfaceData,
    pub agent_name: Option<String>,
    pub on_starter_pick: EventHandler<String>,
}

/// Message list region with empty state, indicators, and auto-scroll.
#[component]
pub fn Messages(props: MessagesProps) -> Element {
    let has_messages = !props.messages.is_empty();

    rsx! {
        if !has_messages && !props.surface.is_thinking {
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
                                    data: rustyclaw_view::ToolCallData::from(tool),
                                }
                            }
                        }
                    }

                    if props.surface.is_thinking {
                        ThinkingIndicator {
                            agent_name: props.agent_name.clone(),
                        }
                    }

                    if props.surface.is_streaming {
                        StreamingProgress {
                            surface: props.surface.clone(),
                        }
                    } else if props.surface.is_processing {
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
    let data = rustyclaw_view::EmptyStateData {
        agent_name: props.agent_name,
    };

    rsx! {
        div { class: "empty-state",
            div { class: "empty-state-card",
                div { class: "empty-state-mark", "🦞" }
                h2 { "{data.greeting()}" }
                p { "{data.subtitle()}" }
                div { class: "starter-grid",
                    for starter in data.starters().iter() {
                        button {
                            key: "{starter.title}",
                            class: "starter-card",
                            onclick: {
                                let p = starter.prompt.to_string();
                                let cb = props.on_pick;
                                move |_| cb.call(p.clone())
                            },
                            div { class: "starter-title",
                                span { "{starter.icon}" }
                                span { "{starter.title}" }
                            }
                            div { class: "starter-sub", "{starter.prompt}" }
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
    surface: rustyclaw_view::ChatSurfaceData,
}

#[component]
fn StreamingProgress(props: StreamingProgressProps) -> Element {
    let label = props
        .surface
        .progress_summary()
        .unwrap_or_else(|| "Processing…".to_string());

    rsx! {
        div { class: "streaming-progress",
            span { class: "streaming-progress-icon streaming-pulse" }
            span { class: "streaming-progress-text", "{label}" }
        }
    }
}

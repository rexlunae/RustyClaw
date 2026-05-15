//! Single chat message row (avatar + role header + content).

use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use crate::markdown;
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::format_chat_timestamp;

/// Props for [`MessageBubble`].
#[derive(Props, Clone, PartialEq)]
pub struct MessageBubbleProps {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[props(default = false)]
    pub is_streaming: bool,
    /// Display name for the agent (shown on assistant messages).
    pub agent_name: Option<String>,
}

#[component]
pub fn MessageBubble(props: MessageBubbleProps) -> Element {
    let (row_class, name, avatar) = match props.role {
        MessageRole::User => ("msg-row is-user", "You".to_string(), "🧑"),
        MessageRole::Assistant => {
            let label = props
                .agent_name
                .as_deref()
                .filter(|n| !n.is_empty())
                .unwrap_or("Assistant")
                .to_string();
            ("msg-row is-assistant", label, "🦞")
        }
        MessageRole::System => ("msg-row is-system", "System".to_string(), "⚙"),
        // Catch-all for the remaining role variants (Info, Success, Warning,
        // Error, ToolCall, ToolResult, Thinking). These aren't currently
        // emitted by the desktop gateway client, but we handle them gracefully.
        _ => ("msg-row is-system", "System".to_string(), "ℹ️"),
    };

    let time_str = format_chat_timestamp(&props.timestamp);
    let time_full = props.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();

    let render_markdown = matches!(props.role, MessageRole::Assistant)
        // Plaintext while streaming: markdown re-parsing on every chunk
        // (100+ per second) overwhelms the webview and backs up the event
        // channel.  Markdown renders once when ResponseDone arrives.
        && !props.is_streaming;

    let content_html = if render_markdown {
        Some(markdown::render(&props.content))
    } else {
        None
    };

    rsx! {
        div { class: "{row_class}",
            div { class: "msg-avatar", "{avatar}" }
            div { class: "msg-body",
                div { class: "msg-header",
                    span { class: "msg-name", "{name}" }
                    span { class: "msg-time", title: "{time_full}", "{time_str}" }
                }

                if let Some(html) = content_html {
                    div {
                        class: "msg-content",
                        dangerous_inner_html: "{html}",
                    }
                } else {
                    div { class: "msg-content is-plain",
                        "{props.content}"
                        if props.is_streaming {
                            span { class: "streaming-cursor" }
                        }
                    }
                }
            }
        }
    }
}

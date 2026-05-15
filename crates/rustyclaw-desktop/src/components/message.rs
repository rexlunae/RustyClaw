//! Single chat message row (avatar + role header + content).

use dioxus::prelude::*;

use crate::markdown;
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::format_chat_timestamp;

/// Props for [`MessageBubble`].
///
/// Wraps [`MessageBubbleData`] from `rustyclaw-view` with the Dioxus-specific
/// event handlers that this component needs.
#[derive(Props, Clone, PartialEq)]
pub struct MessageBubbleProps {
    /// Shared component data (role, content, timestamp, streaming, agent name).
    pub data: rustyclaw_view::MessageBubbleData,
    /// Optional callback when the user clicks on the message.
    pub onclick: Option<EventHandler<()>>,
}

#[component]
pub fn MessageBubble(props: MessageBubbleProps) -> Element {
    let (row_class, name, avatar) = match props.data.role {
        MessageRole::User => ("msg-row is-user", "You".to_string(), "🧑"),
        MessageRole::Assistant => {
            let label = props
                .data
                .agent_name
                .as_deref()
                .filter(|n| !n.is_empty())
                .unwrap_or("Assistant")
                .to_string();
            ("msg-row is-assistant", label, "🦞")
        }
        MessageRole::System => ("msg-row is-system", "System".to_string(), "⚙"),
        _ => ("msg-row is-system", "System".to_string(), "ℹ️"),
    };

    // Timestamp is optional in MessageBubbleData. The desktop always sets it
    // (via from_chat_message), but we still handle None gracefully.
    let time_str = props
        .data
        .timestamp
        .as_ref()
        .map(format_chat_timestamp)
        .unwrap_or_default();
    let time_full = props
        .data
        .timestamp
        .as_ref()
        .map(|ts| ts.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default();

    let render_markdown = matches!(props.data.role, MessageRole::Assistant)
        && !props.data.is_streaming;

    let content_html = if render_markdown {
        Some(markdown::render(&props.data.content))
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
                        "{props.data.content}"
                        if props.data.is_streaming {
                            span { class: "streaming-cursor" }
                        }
                    }
                }
            }
        }
    }
}

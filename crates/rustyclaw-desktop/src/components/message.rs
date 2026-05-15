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
    // CSS class and avatar emoji are desktop-specific.  The display name,
    // markdown decision, and content transformations come from shared methods.
    let (row_class, avatar) = match props.data.role {
        MessageRole::User => ("msg-row is-user", "🧑"),
        MessageRole::Assistant => ("msg-row is-assistant", "🦞"),
        MessageRole::System => ("msg-row is-system", "⚙"),
        _ => ("msg-row is-system", "ℹ️"),
    };
    let name = props.data.display_name().to_string();

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

    // Shared markdown decision.  Display content already accounts for
    // thinking-truncation; markdown rendering is renderer-specific.
    let rendered = if props.data.should_render_markdown() {
        Some(markdown::render(&props.data.content))
    } else {
        None
    };
    let display = props.data.display_content();

    rsx! {
        div { class: "{row_class}",
            div { class: "msg-avatar", "{avatar}" }
            div { class: "msg-body",
                div { class: "msg-header",
                    span { class: "msg-name", "{name}" }
                    span { class: "msg-time", title: "{time_full}", "{time_str}" }
                }

                if let Some(html) = rendered {
                    div {
                        class: "msg-content",
                        dangerous_inner_html: "{html}",
                    }
                } else {
                    div { class: "msg-content is-plain",
                        "{display}"
                        if props.data.is_streaming {
                            span { class: "streaming-cursor" }
                        }
                    }
                }
            }
        }
    }
}

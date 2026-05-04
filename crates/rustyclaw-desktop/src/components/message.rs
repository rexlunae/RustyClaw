//! Message bubble component.

use dioxus::prelude::*;
use dioxus_bulma::prelude::*;

use crate::state::MessageRole;

/// Props for MessageBubble.
#[derive(Props, Clone, PartialEq)]
pub struct MessageBubbleProps {
    /// Message role (user/assistant/system)
    pub role: MessageRole,
    /// Message content
    pub content: String,
    /// Whether the message is still streaming
    #[props(default = false)]
    pub is_streaming: bool,
}

/// A single message bubble in the chat.
#[component]
pub fn MessageBubble(props: MessageBubbleProps) -> Element {
    let (icon_class, bg_color) = match props.role {
        MessageRole::User => ("fa-user", "#3273dc"),
        MessageRole::Assistant => ("fa-robot", "#209cee"),
        MessageRole::System => ("fa-cog", "#ffdd57"),
    };

    let is_user = props.role == MessageRole::User;
    let align = if is_user { "flex-end" } else { "flex-start" };

    rsx! {
        div {
            class: "message-bubble",
            style: "display: flex; justify-content: {align}; margin-bottom: 0.75rem;",

            BulmaBox {
                style: "max-width: 80%; background-color: {bg_color}; color: white; padding: 0.75rem 1rem;",

                // Header with icon
                div {
                    class: "message-header",
                    style: "display: flex; align-items: center; margin-bottom: 0.25rem; opacity: 0.8; font-size: 0.85rem;",

                    Icon { size: BulmaSize::Small,
                        i { class: "fas {icon_class}" }
                    }
                    span { style: "margin-left: 0.25rem;",
                        match props.role {
                            MessageRole::User => "You",
                            MessageRole::Assistant => "Assistant",
                            MessageRole::System => "System",
                        }
                    }
                }

                // Content
                div { class: "message-content",
                    style: "white-space: pre-wrap; word-break: break-word;",

                    "{props.content}"

                    // Streaming cursor
                    if props.is_streaming {
                        span {
                            class: "streaming-cursor",
                            style: "animation: blink 1s infinite;",
                            "▊"
                        }
                    }
                }
            }
        }
    }
}

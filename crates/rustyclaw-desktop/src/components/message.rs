//! Single chat message row (avatar + role header + content).

use chrono::{DateTime, Local, Utc};
use dioxus::prelude::*;

use crate::markdown;
use crate::state::MessageRole;

/// Props for [`MessageBubble`].
#[derive(Props, Clone, PartialEq)]
pub struct MessageBubbleProps {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[props(default = false)]
    pub is_streaming: bool,
}

#[component]
pub fn MessageBubble(props: MessageBubbleProps) -> Element {
    let (row_class, name, avatar) = match props.role {
        MessageRole::User => ("msg-row is-user", "You", "🧑"),
        MessageRole::Assistant => ("msg-row is-assistant", "Assistant", "🦞"),
        MessageRole::System => ("msg-row is-system", "System", "⚙"),
    };

    let local: DateTime<Local> = props.timestamp.with_timezone(&Local);
    let time_str = local.format("%H:%M").to_string();
    let time_full = local.format("%Y-%m-%d %H:%M:%S").to_string();

    let is_assistant = matches!(props.role, MessageRole::Assistant);

    let content_html = if is_assistant && props.is_streaming {
        Some(markdown::render_streaming(&props.content))
    } else if is_assistant {
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
                    }
                }
            }
        }
    }
}

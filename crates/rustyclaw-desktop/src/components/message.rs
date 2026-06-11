//! Single chat message row (avatar + role header + content).

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button, Buttons};

use crate::markdown;
use rustyclaw_core::ui::format_chat_timestamp;

/// Props for [`MessageBubble`].
#[derive(Props, Clone, PartialEq)]
pub struct MessageBubbleProps {
    /// Shared component data (role, content, timestamp, streaming, agent name).
    pub data: rustyclaw_view::MessageBubbleData,
    /// Optional callback when the user clicks on the message.
    pub onclick: Option<EventHandler<()>>,
}

#[component]
pub fn MessageBubble(props: MessageBubbleProps) -> Element {
    let mut collapsed = use_signal(|| props.data.collapsed);

    let row_class = format!("msg-row {}", props.data.role_class());
    let avatar = props.data.avatar();
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

    // Build a temporary view model with local collapsed state for rendering
    let mut render_data = props.data.clone();
    render_data.collapsed = collapsed();

    let rendered = if render_data.should_render_markdown() {
        Some(markdown::render(&render_data.content_for_render()))
    } else {
        None
    };
    let display = render_data.content_for_render();

    let can_collapse = props.data.is_collapsible();
    let is_collapsed = collapsed();
    let content_to_copy = props.data.content.clone();
    let content_to_save = props.data.content.clone();
    let is_streaming = props.data.is_streaming;

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
                        class: "content msg-content",
                        dangerous_inner_html: "{html}",
                    }
                } else {
                    div { class: "msg-content is-plain",
                        "{display}"
                        if is_streaming {
                            span { class: "streaming-cursor" }
                        }
                    }
                }

                if !is_streaming {
                    Buttons { class: "msg-actions", size: BulmaSize::Small,
                        if can_collapse {
                            Button {
                                color: BulmaColor::Ghost,
                                size: BulmaSize::Small,
                                class: "msg-action-btn",
                                onclick: move |_| {
                                    let current = *collapsed.read();
                                    collapsed.set(!current);
                                },
                                if is_collapsed { "⊞ Expand" } else { "⊟ Collapse" }
                            }
                        }
                        Button {
                            color: BulmaColor::Ghost,
                            size: BulmaSize::Small,
                            class: "msg-action-btn",
                            onclick: move |_| {
                                let text = content_to_copy.clone();
                                spawn(async move {
                                    let js = format!("navigator.clipboard.writeText({:?})", text);
                                    let _ = document::eval(&js).await;
                                });
                            },
                            "⎘ Copy"
                        }
                        Button {
                            color: BulmaColor::Ghost,
                            size: BulmaSize::Small,
                            class: "msg-action-btn",
                            onclick: move |_| {
                                let text = content_to_save.clone();
                                spawn(async move {
                                    if let Some(dir) = dirs::home_dir() {
                                        let dir = dir.join(".rustyclaw").join("messages");
                                        let _ = tokio::fs::create_dir_all(&dir).await;
                                        let filename = format!(
                                            "{}.md",
                                            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
                                        );
                                        if let Err(e) = tokio::fs::write(dir.join(&filename), &text).await { tracing::warn!("failed to save message to file: {e}"); }
                                    }
                                });
                            },
                            "↓ Save"
                        }
                    }
                }
            }
        }
    }
}

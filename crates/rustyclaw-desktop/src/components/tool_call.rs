//! Collapsible panel showing tool-call arguments and result.
//!
//! Rendered as a Bulma `Message` whose colour follows the call status
//! (running → info, done → success, failed → danger).  Status wording
//! and tone both come from the shared view layer.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaSize, Message, MessageBody, MessageHeader, Tag};
use rustyclaw_view::ToolCallData;

use super::tone_color;

/// Props for [`ToolCallPanel`].
#[derive(Props, Clone, PartialEq)]
pub struct ToolCallPanelProps {
    pub data: ToolCallData,
}

#[component]
pub fn ToolCallPanel(props: ToolCallPanelProps) -> Element {
    let mut is_collapsed = use_signal(|| props.data.collapsed);

    let (_, status_label, status_icon) = props.data.status_label();
    let status_tone = props.data.status_tone();
    let is_running = props.data.result.is_none();

    let panel_class = if *is_collapsed.read() {
        "tool-call"
    } else {
        "tool-call is-open"
    };
    let chip_class = if is_running {
        "rc-chip is-pulse"
    } else {
        "rc-chip"
    };

    let pretty_args = props.data.arguments_preview(100_000, 10_000);

    rsx! {
        Message {
            color: tone_color(status_tone),
            size: BulmaSize::Small,
            class: panel_class,
            MessageHeader { class: "tool-head",
                div {
                    class: "tool-head-row",
                    onclick: move |_| {
                        let v = *is_collapsed.read();
                        is_collapsed.set(!v);
                    },
                    span { class: "tool-name", "🔧 {props.data.name}" }
                    span { class: "tool-spacer" }
                    Tag {
                        color: tone_color(status_tone),
                        light: true,
                        rounded: true,
                        class: chip_class,
                        "{status_icon} {status_label}"
                    }
                    span { class: "tool-chevron", "⌄" }
                }
            }

            if !*is_collapsed.read() {
                MessageBody { class: "tool-body",
                    div { class: "tool-section",
                        div { class: "tool-section-label", "Arguments" }
                        pre { class: "tool-pre",
                            code { "{pretty_args}" }
                        }
                    }
                    if let Some(result) = props.data.result.as_ref() {
                        div { class: "tool-section",
                            div { class: "tool-section-label",
                                if props.data.is_error { "Error" } else { "Result" }
                            }
                            pre {
                                class: if props.data.is_error { "tool-pre is-error" } else { "tool-pre" },
                                code { "{result}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

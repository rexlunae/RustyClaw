//! Tool call display panel.
//!
//! Renders a Bulma `Message` whose header is clickable to expand/collapse the
//! arguments and result. Uses dioxus-bulma's `Message` family of components.

use dioxus::prelude::*;
use dioxus_bulma::prelude::*;

/// Props for ToolCallPanel.
#[derive(Props, Clone, PartialEq)]
pub struct ToolCallPanelProps {
    /// Tool call ID
    pub id: String,
    /// Tool name
    pub name: String,
    /// Tool arguments (JSON)
    pub arguments: String,
    /// Tool result
    pub result: Option<String>,
    /// Whether the result is an error
    #[props(default = false)]
    pub is_error: bool,
    /// Whether the panel is collapsed
    #[props(default = true)]
    pub collapsed: bool,
}

/// Collapsible panel showing tool call details.
#[component]
pub fn ToolCallPanel(props: ToolCallPanelProps) -> Element {
    let mut is_collapsed = use_signal(|| props.collapsed);

    let status_color = if props.result.is_some() {
        if props.is_error {
            BulmaColor::Danger
        } else {
            BulmaColor::Success
        }
    } else {
        BulmaColor::Info
    };

    let status_icon = if props.result.is_some() {
        if props.is_error {
            "fa-times-circle"
        } else {
            "fa-check-circle"
        }
    } else {
        "fa-spinner fa-spin"
    };

    let panel_id = format!("tool-call-{}", props.id);

    rsx! {
        Message {
            id: panel_id,
            color: status_color,
            class: "tool-call-panel",
            style: "margin: 0.5rem 0; font-size: 0.9rem;",

            // Header (clickable to toggle expand/collapse). `MessageHeader`
            // renders a `<div class="message-header"><p>...</p></div>`; we
            // attach the onclick to the inner content so the whole header
            // surface toggles the panel.
            MessageHeader {
                style: "cursor: pointer; padding: 0.5rem 0.75rem;",

                span {
                    style: "display: flex; align-items: center; width: 100%;",
                    onclick: move |_| {
                        let val = *is_collapsed.read();
                        is_collapsed.set(!val);
                    },

                    Icon { size: BulmaSize::Small,
                        i { class: "fas fa-wrench" }
                    }
                    span { style: "margin-left: 0.25rem; font-weight: 600;",
                        "{props.name}"
                    }

                    span { style: "margin-left: auto;",
                        Icon { size: BulmaSize::Small,
                            i { class: "fas {status_icon}" }
                        }
                        Icon { size: BulmaSize::Small,
                            i { class: if *is_collapsed.read() { "fas fa-chevron-down" } else { "fas fa-chevron-up" } }
                        }
                    }
                }
            }

            // Body (collapsible)
            if !*is_collapsed.read() {
                MessageBody {
                    style: "padding: 0.75rem;",

                    // Arguments
                    div { class: "tool-arguments",
                        style: "margin-bottom: 0.5rem;",

                        strong { "Arguments:" }
                        pre {
                            style: "background: rgba(0,0,0,0.1); padding: 0.5rem; border-radius: 4px; overflow-x: auto; margin-top: 0.25rem;",
                            code {
                                // Pretty print JSON if possible
                                {
                                    serde_json::from_str::<serde_json::Value>(&props.arguments)
                                        .map(|v| serde_json::to_string_pretty(&v).unwrap_or(props.arguments.clone()))
                                        .unwrap_or(props.arguments.clone())
                                }
                            }
                        }
                    }

                    // Result (if available)
                    if let Some(result) = &props.result {
                        div { class: "tool-result",
                            strong {
                                if props.is_error { "Error:" } else { "Result:" }
                            }
                            pre {
                                style: "background: rgba(0,0,0,0.1); padding: 0.5rem; border-radius: 4px; overflow-x: auto; margin-top: 0.25rem; max-height: 200px;",
                                code { "{result}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

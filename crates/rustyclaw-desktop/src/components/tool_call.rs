//! Tool call display panel.

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

    let status_class = if props.result.is_some() {
        if props.is_error {
            "is-danger"
        } else {
            "is-success"
        }
    } else {
        "is-info"
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

    rsx! {
        div {
            class: "tool-call-panel message {status_class}",
            style: "margin: 0.5rem 0; font-size: 0.9rem;",

            // Header (clickable)
            div {
                class: "message-header",
                style: "cursor: pointer; padding: 0.5rem 0.75rem;",
                onclick: move |_| {
                    let val = *is_collapsed.read();
                    is_collapsed.set(!val);
                },

                span { class: "icon is-small",
                    i { class: "fas fa-wrench" }
                }
                span { style: "margin-left: 0.25rem; font-weight: 600;",
                    "{props.name}"
                }

                span { style: "margin-left: auto;",
                    span { class: "icon is-small",
                        i { class: "fas {status_icon}" }
                    }
                    span { class: "icon is-small",
                        i { class: if *is_collapsed.read() { "fas fa-chevron-down" } else { "fas fa-chevron-up" } }
                    }
                }
            }

            // Body (collapsible)
            if !*is_collapsed.read() {
                div { class: "message-body",
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

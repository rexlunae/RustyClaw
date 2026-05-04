//! Collapsible panel showing tool-call arguments and result.

use dioxus::prelude::*;

/// Props for [`ToolCallPanel`].
#[derive(Props, Clone, PartialEq)]
pub struct ToolCallPanelProps {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub result: Option<String>,
    #[props(default = false)]
    pub is_error: bool,
    #[props(default = true)]
    pub collapsed: bool,
}

#[component]
pub fn ToolCallPanel(props: ToolCallPanelProps) -> Element {
    let mut is_collapsed = use_signal(|| props.collapsed);

    let (status_class, status_label, status_icon) = if props.result.is_some() {
        if props.is_error {
            ("is-error", "Failed", "✕")
        } else {
            ("is-done", "Done", "✓")
        }
    } else {
        ("is-running", "Running…", "⏳")
    };

    let panel_class = format!(
        "tool-call {} {}",
        status_class,
        if *is_collapsed.read() { "" } else { "is-open" }
    );
    let panel_class = panel_class.trim().to_string();

    let pretty_args = serde_json::from_str::<serde_json::Value>(&props.arguments)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| props.arguments.clone());

    let chip_class = format!(
        "chip {}",
        match status_class {
            "is-error" => "is-danger",
            "is-running" => "is-info is-pulse",
            _ => "is-success",
        }
    );

    rsx! {
        div { class: "{panel_class}",
            div { class: "tool-head",
                onclick: move |_| {
                    let v = *is_collapsed.read();
                    is_collapsed.set(!v);
                },
                span { class: "tool-name", "🔧 {props.name}" }
                span { class: "tool-spacer" }
                span { class: "{chip_class}",
                    span { class: "dot" }
                    span { "{status_icon} {status_label}" }
                }
                span { class: "tool-chevron", "⌄" }
            }

            if !*is_collapsed.read() {
                div { class: "tool-body",
                    div { class: "tool-section",
                        div { class: "tool-section-label", "Arguments" }
                        pre { class: "tool-pre",
                            code { "{pretty_args}" }
                        }
                    }
                    if let Some(result) = props.result.as_ref() {
                        div { class: "tool-section",
                            div { class: "tool-section-label",
                                if props.is_error { "Error" } else { "Result" }
                            }
                            pre {
                                class: if props.is_error { "tool-pre is-error" } else { "tool-pre" },
                                code { "{result}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

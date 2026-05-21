//! Tool approval dialog: approve or deny tool execution in Ask mode.

use dioxus::prelude::*;
use rustyclaw_view::ToolApprovalData;

#[derive(Props, Clone, PartialEq)]
pub struct ToolApprovalDialogProps {
    pub visible: bool,
    pub data: ToolApprovalData,
    pub on_approve: EventHandler<String>,
    pub on_deny: EventHandler<String>,
}

#[component]
pub fn ToolApprovalDialog(props: ToolApprovalDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let truncated_args = props.data.arguments_preview(500, 20);

    let id_approve = props.data.id.clone();
    let id_deny = props.data.id.clone();

    rsx! {
        div { class: "modal-backdrop",
            div {
                class: "modal",
                style: "max-width: 560px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "🔒 Tool Approval Required" }
                }

                div { class: "modal-body",
                    div { class: "settings-section",
                        div { class: "settings-section-title", "The agent wants to run:" }
                        div {
                            style: "margin-top: 8px; padding: 8px 12px; background: var(--bg-surface); border-radius: 6px; font-family: monospace;",
                            span {
                                style: "color: var(--accent-bright); font-weight: bold;",
                                "{props.data.name}"
                            }
                        }
                    }

                    if !truncated_args.is_empty() {
                        div { class: "settings-section",
                            div { class: "settings-section-title", "Arguments:" }
                            pre {
                                style: "margin-top: 8px; padding: 8px 12px; background: var(--bg-surface); border-radius: 6px; font-size: 0.85em; max-height: 200px; overflow: auto; white-space: pre-wrap; word-break: break-all;",
                                "{truncated_args}"
                            }
                        }
                    }
                }

                div { class: "modal-foot",
                    button {
                        class: "btn btn-subtle",
                        onclick: move |_| props.on_deny.call(id_deny.clone()),
                        "✕ Deny"
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| props.on_approve.call(id_approve.clone()),
                        "✓ Approve"
                    }
                }
            }
        }
    }
}

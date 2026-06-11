//! Tool approval dialog: approve or deny tool execution in Ask mode.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, FieldLabel, Tag};
use rustyclaw_view::ToolApprovalData;

use super::RcModal;

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
    let id_close = props.data.id.clone();

    rsx! {
        RcModal {
            active: true,
            title: "🔒 Tool Approval Required",
            width: 560,
            onclose: move |_| props.on_deny.call(id_close.clone()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| props.on_deny.call(id_deny.clone()),
                        "✕ Deny"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_approve.call(id_approve.clone()),
                        "✓ Approve"
                    }
                }
            },

            div { class: "settings-section",
                FieldLabel { "The agent wants to run:" }
                Tag {
                    color: BulmaColor::Warning,
                    light: true,
                    class: "tool-approval-name",
                    "{props.data.name}"
                }
            }

            if !truncated_args.is_empty() {
                div { class: "settings-section",
                    FieldLabel { "Arguments:" }
                    pre { class: "tool-pre tool-approval-args", "{truncated_args}" }
                }
            }
        }
    }
}

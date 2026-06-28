//! Approvals panel — pending approvals queue dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct ApprovalsDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::ApprovalsPanelData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn ApprovalsDialog(props: ApprovalsDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Pending Approvals",
            width: 700,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                dioxus_bulma::prelude::Buttons {
                    dioxus_bulma::prelude::Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Close"
                    }
                }
            },

            if let Some(ref data) = props.data {
                if data.approvals.is_empty() {
                    p { class: "has-text-grey", "No pending approvals." }
                } else {
                    div { class: "mb-3",
                        span { class: "tag is-warning is-light",
                            "{data.count()} pending"
                        }
                        if data.selected_count() > 0 {
                            span { class: "tag is-info is-light ml-2",
                                "{data.selected_count()} selected"
                            }
                        }
                    }

                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Tool" }
                                th { "Arguments" }
                                th { "Requested" }
                            }
                        }
                        tbody {
                            for approval in data.approvals.iter() {
                                tr { key: "{approval.id}",
                                    td { strong { "{approval.tool_name}" } }
                                    td {
                                        code { class: "is-size-7",
                                            "{approval.arguments_preview(60)}"
                                        }
                                    }
                                    td { "{approval.requested_at}" }
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "Approvals system not available." }
            }
        }
    }
}

use dioxus::prelude::*;
use dioxus_bulma::prelude::*;
use crate::state::{AppState, ToolApprovalRequest};

#[component]
pub fn ToolApprovalDialog(
    visible: Signal<<boolbool>,
    request: Option<<ToolToolApprovalRequest>,
    on_approve: EventHandler<<StringString>,
    on_deny: EventHandler<<StringString>,
) -> Element {
    let Some(req) = request else {
        return rsx! { div { } };
    };

    let call_id = req.call_id.clone();

    rsx! {
        div { 
            class: "modal",
            style: if *visible.read() { "display: block" } else { "display: none" },
            div { class: "modal-background", onclick: move |_| on_deny.call(call_id.clone()) }
            div { 
                class: "modal-card",
                div { 
                    class: "modal-card-head",
                    p { class: "modal-card-title", "Tool Call Approval" }
                }
                div { 
                    class: "modal-card-body",
                    div { 
                        class: "content",
                        p { 
                            b { "Tool: " } 
                            "{req.tool_name}" 
                        }
                        div { 
                            class: "notification is-light",
                            pre { 
                                style: "white-space: pre-wrap; word-break: break-all;",
                                "{req.arguments}" 
                            }
                        }
                        p { 
                            "The agent wants to execute this tool. Do you approve?" 
                        }
                    }
                }
                div { 
                    class: "modal-card-foot",
                    button { 
                        class: "button is-danger", 
                        onclick: move |_| on_deny.call(call_id.clone()), 
                        "Deny" 
                    }
                    button { 
                        class: "button is-success", 
                        onclick: move |_| on_approve.call(call_id.clone()), 
                        "Approve" 
                    }
                }
            }
        }
    }
}

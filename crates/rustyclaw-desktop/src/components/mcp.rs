//! MCP panel — MCP server management dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct McpDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::McpPanelData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn McpDialog(props: McpDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "MCP Servers",
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
                if data.servers.is_empty() {
                    p { class: "has-text-grey", "No MCP servers configured." }
                } else {
                    div { class: "mb-3",
                        span { class: "tag is-info is-light",
                            "{data.connected_count()} connected / {data.total_count()} total"
                        }
                    }

                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Name" }
                                th { "Status" }
                                th { "Target" }
                                th { "Tools" }
                                th { "Health" }
                            }
                        }
                        tbody {
                            for server in data.servers.iter() {
                                tr { key: "{server.name}",
                                    td { strong { "{server.name}" } }
                                    td {
                                        span { class: "tag",
                                            "{server.status}"
                                        }
                                    }
                                    td { code { class: "is-size-7", "{server.target()}" } }
                                    td { "{server.tool_count()}" }
                                    td { "{server.health_label()}" }
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "MCP system not initialised." }
            }
        }
    }
}

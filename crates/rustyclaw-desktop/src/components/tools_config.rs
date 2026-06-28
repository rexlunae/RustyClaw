//! Tools config panel — tool enable/disable dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct ToolsConfigDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::ToolConfigPanelData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn ToolsConfigDialog(props: ToolsConfigDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Tool Configuration",
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
                if data.tools.is_empty() {
                    p { class: "has-text-grey", "No tools registered." }
                } else {
                    div { class: "mb-3",
                        span { class: "tag is-info is-light",
                            "{data.enabled_count()} enabled / {data.total_count()} total"
                        }
                    }

                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Tool" }
                                th { "Category" }
                                th { "Enabled" }
                                th { "Policy" }
                                th { "Description" }
                            }
                        }
                        tbody {
                            for tool in data.tools.iter() {
                                tr { key: "{tool.name}",
                                    td { strong { "{tool.name}" } }
                                    td {
                                        span { class: "tag is-light",
                                            "{tool.category}"
                                        }
                                    }
                                    td {
                                        if tool.enabled {
                                            span { class: "tag is-success is-light", "ON" }
                                        } else {
                                            span { class: "tag is-light", "OFF" }
                                        }
                                    }
                                    td {
                                        span { class: "tag is-light",
                                            "{tool.policy}"
                                        }
                                    }
                                    td { class: "is-size-7", "{tool.description}" }
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "Tool registry not available." }
            }
        }
    }
}

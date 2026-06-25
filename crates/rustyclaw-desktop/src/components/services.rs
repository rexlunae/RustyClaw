//! Services panel — managed backend services dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct ServicesDialogProps {
    pub visible: bool,
    pub services: Option<rustyclaw_view::ServiceListData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn ServicesDialog(props: ServicesDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Managed Services",
            width: 640,
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

            if let Some(ref data) = props.services {
                if data.services.is_empty() {
                    p { class: "has-text-grey", "No services configured." }
                } else {
                    div { class: "mb-3",
                        span { class: "tag is-info is-light",
                            "{data.running_count()} running / {data.total_count()} total"
                        }
                    }

                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Name" }
                                th { "Type" }
                                th { "Status" }
                                th { "PID" }
                                th { "Uptime" }
                                th { "Restarts" }
                                th { "Health" }
                                th { "MCP Tools" }
                            }
                        }
                        tbody {
                            for svc in data.services.iter() {
                                tr { key: "{svc.name}",
                                    td { strong { "{svc.name}" } }
                                    td { "{svc.service_type}" }
                                    td {
                                        span { class: "tag {svc.status_class()}",
                                            "{svc.status}"
                                        }
                                    }
                                    td {
                                        if let Some(pid) = svc.pid {
                                            "{pid}"
                                        } else {
                                            "—"
                                        }
                                    }
                                    td { "{svc.uptime_display()}" }
                                    td { "{svc.restart_count}" }
                                    td {
                                        match svc.health_ok {
                                            Some(true) => rsx! { span { class: "tag is-success is-light", "OK" } },
                                            Some(false) => rsx! { span { class: "tag is-danger is-light", "Fail" } },
                                            None => rsx! { span { class: "has-text-grey", "—" } },
                                        }
                                    }
                                    td { "{svc.mcp_tools}" }
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "Service manager not initialised." }
            }
        }
    }
}

//! System information panel — host capabilities + load status.

use dioxus::prelude::*;
use dioxus_bulma::components::{Title, TitleSize};
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct SystemInfoDialogProps {
    pub visible: bool,
    pub host: Option<rustyclaw_view::HostInfoData>,
    pub load: Option<rustyclaw_view::LoadStatusData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn SystemInfoDialog(props: SystemInfoDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let load_color = props
        .load
        .as_ref()
        .map(|l| match l.load_class() {
            "is-success" => "has-text-success",
            "is-info" => "has-text-info",
            "is-warn" => "has-text-warning",
            "is-danger" => "has-text-danger",
            _ => "",
        })
        .unwrap_or("");

    rsx! {
        RcModal {
            active: true,
            title: "System Information",
            width: 560,
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

            // Host info section
            div { class: "system-info-section",
                Title { size: TitleSize::Is6, class: "system-info-title", "Host Hardware" }
                if let Some(ref h) = props.host {
                    table { class: "table is-narrow is-fullwidth",
                        tbody {
                            tr {
                                td { strong { "Hostname" } }
                                td { "{h.hostname}" }
                            }
                            tr {
                                td { strong { "OS / Arch" } }
                                td { "{h.os} ({h.arch})" }
                            }
                            tr {
                                td { strong { "CPU" } }
                                td { "{h.cpu_brand} ({h.cpu_cores_physical}/{h.cpu_cores_logical}c @ {h.cpu_frequency_mhz}MHz)" }
                            }
                            tr {
                                td { strong { "RAM" } }
                                td { "{h.total_memory_gib:.1} GiB" }
                            }
                            if h.total_swap_gib > 0.0 {
                                tr {
                                    td { strong { "Swap" } }
                                    td { "{h.total_swap_gib:.1} GiB" }
                                }
                            }
                            tr {
                                td { strong { "Disk" } }
                                td { "{h.disk_available_gib:.1} / {h.disk_total_gib:.1} GiB free ({h.disk_used_percent()})" }
                            }
                            for (i, gpu) in h.gpus.iter().enumerate() {
                                tr { key: "{i}",
                                    td { strong { "GPU {i}" } }
                                    td { "{gpu.name} ({gpu.vendor}, {gpu.vram_gib:.1} GiB VRAM)" }
                                }
                            }
                        }
                    }
                } else {
                    p { class: "has-text-grey", "Host capabilities not yet detected." }
                }
            }

            // Load status section
            div { class: "system-info-section mt-4",
                Title { size: TitleSize::Is6, class: "system-info-title", "System Load" }
                if let Some(ref l) = props.load {
                    table { class: "table is-narrow is-fullwidth",
                        tbody {
                            tr {
                                td { strong { "Load Score" } }
                                td {
                                    span { class: "{load_color}",
                                        "{l.load_score:.2} [{l.load_label()}]"
                                    }
                                }
                            }
                            tr {
                                td { strong { "Avg Load" } }
                                td { "{l.avg_load_score:.2}" }
                            }
                            tr {
                                td { strong { "CPU Usage" } }
                                td { "{l.cpu_percent:.1}%" }
                            }
                            tr {
                                td { strong { "Memory" } }
                                td { "{l.memory_percent:.1}%" }
                            }
                        }
                    }
                    // Visual load bar
                    div { class: "system-load-bar",
                        progress {
                            class: "progress {load_color}",
                            value: "{l.load_score}",
                            max: "1.0",
                        }
                    }
                } else {
                    p { class: "has-text-grey", "Load data not yet sampled." }
                }
            }
        }
    }
}

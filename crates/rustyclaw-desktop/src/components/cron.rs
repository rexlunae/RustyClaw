//! Cron panel — scheduled jobs management dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct CronDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::CronPanelData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn CronDialog(props: CronDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Scheduled Jobs",
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
                if data.jobs.is_empty() {
                    p { class: "has-text-grey", "No scheduled jobs." }
                } else {
                    div { class: "mb-3",
                        span { class: "tag is-info is-light",
                            "{data.active_count()} active / {data.total_count()} total"
                        }
                    }

                    table { class: "table is-narrow is-fullwidth is-hoverable",
                        thead {
                            tr {
                                th { "Name" }
                                th { "Expression" }
                                th { "Status" }
                                th { "Next Run" }
                                th { "Last Run" }
                                th { "Runs" }
                            }
                        }
                        tbody {
                            for job in data.jobs.iter() {
                                tr { key: "{job.id}",
                                    td { strong { "{job.name}" } }
                                    td { code { "{job.expr}" } }
                                    td {
                                        span { class: "tag",
                                            "{job.status_label()}"
                                        }
                                    }
                                    td {
                                        if let Some(ref next) = job.next_run {
                                            "{next}"
                                        } else {
                                            "—"
                                        }
                                    }
                                    td {
                                        if let Some(ref last) = job.last_run {
                                            "{last}"
                                        } else {
                                            "—"
                                        }
                                    }
                                    td { "{job.run_count}" }
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "Cron system not initialised." }
            }
        }
    }
}

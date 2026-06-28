//! Logs panel — general log viewer dialog.

use dioxus::prelude::*;
use dioxus_bulma::prelude::BulmaColor;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct LogsDialogProps {
    pub visible: bool,
    pub data: Option<rustyclaw_view::LogsPanelData>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn LogsDialog(props: LogsDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "Logs",
            width: 800,
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
                div { class: "mb-3",
                    span { class: "tag is-info is-light mr-2",
                        "Source: {data.source.label()}"
                    }
                    span { class: "tag is-light mr-2",
                        "{data.line_count()} lines"
                    }
                    if data.following {
                        span { class: "tag is-success is-light",
                            "Following"
                        }
                    }
                }

                if data.lines.is_empty() {
                    p { class: "has-text-grey", "No log entries." }
                } else {
                    pre { class: "is-size-7",
                        style: "max-height: 400px; overflow-y: auto; background: #1a1a1a; color: #e0e0e0; padding: 1rem; border-radius: 4px;",
                        for line in data.lines.iter() {
                            "{line}\n"
                        }
                    }
                }
            } else {
                p { class: "has-text-grey", "Logs not available." }
            }
        }
    }
}

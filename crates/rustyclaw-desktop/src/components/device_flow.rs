//! Device flow dialog: OAuth device flow showing URL and code.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, FieldLabel, Notification};
use rustyclaw_view::DeviceFlowData;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct DeviceFlowDialogProps {
    pub visible: bool,
    pub data: DeviceFlowData,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn DeviceFlowDialog(props: DeviceFlowDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        RcModal {
            active: true,
            title: "🔗 Device Authentication",
            width: 480,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| props.on_close.call(()),
                        "Cancel"
                    }
                }
            },

            if let Some(ref msg) = props.data.message {
                Notification {
                    color: BulmaColor::Warning,
                    light: true,
                    class: "device-flow-provider",
                    "Provider: {msg}"
                }
            }

            p { class: "rc-dialog-lead",
                "Visit the URL below and enter the code to authenticate."
            }

            div { class: "settings-section",
                FieldLabel { "Verification URL" }
                div { class: "device-flow-url",
                    a {
                        href: "{props.data.url}",
                        target: "_blank",
                        "{props.data.url}"
                    }
                }
            }

            div { class: "settings-section",
                FieldLabel { "Code" }
                div { class: "device-flow-code", "{props.data.code}" }
            }

            p { class: "rc-dialog-wait", "⏳ Waiting for authentication to complete…" }
        }
    }
}

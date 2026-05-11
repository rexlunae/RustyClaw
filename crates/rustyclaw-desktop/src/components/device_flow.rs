//! Device flow dialog: OAuth device flow showing URL and code.

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct DeviceFlowDialogProps {
    pub visible: bool,
    pub url: String,
    pub code: String,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn DeviceFlowDialog(props: DeviceFlowDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        div { class: "modal-backdrop",
            div {
                class: "modal",
                style: "max-width: 480px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "🔗 Device Authentication" }
                }

                div { class: "modal-body",
                    p {
                        style: "color: var(--text-dim); margin-bottom: 16px;",
                        "Visit the URL below and enter the code to authenticate."
                    }

                    div { class: "settings-section",
                        div { class: "settings-section-title", "Verification URL" }
                        div {
                            style: "margin-top: 8px; padding: 10px 14px; background: var(--bg-surface); border-radius: 6px; font-family: monospace; word-break: break-all;",
                            a {
                                href: "{props.url}",
                                target: "_blank",
                                style: "color: var(--accent-bright);",
                                "{props.url}"
                            }
                        }
                    }

                    div { class: "settings-section",
                        div { class: "settings-section-title", "Code" }
                        div {
                            style: "margin-top: 8px; padding: 12px 14px; background: var(--bg-surface); border-radius: 6px; font-family: monospace; font-size: 1.4em; font-weight: bold; text-align: center; letter-spacing: 0.15em; color: var(--accent-bright);",
                            "{props.code}"
                        }
                    }

                    p {
                        style: "color: var(--text-dim); margin-top: 16px; font-size: 0.9em;",
                        "⏳ Waiting for authentication to complete…"
                    }
                }

                div { class: "modal-foot",
                    button {
                        class: "btn btn-subtle",
                        onclick: move |_| props.on_close.call(()),
                        "Cancel"
                    }
                }
            }
        }
    }
}

//! First-run "hatching" wizard for naming and personality setup.

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct HatchingDialogProps {
    pub visible: bool,
    pub on_complete: EventHandler<HatchingResult>,
    pub on_cancel: EventHandler<()>,
}

/// Result of the hatching process.
#[derive(Clone, Debug)]
pub struct HatchingResult {
    pub name: String,
    pub personality: Option<String>,
}

#[component]
pub fn HatchingDialog(props: HatchingDialogProps) -> Element {
    let mut name = use_signal(String::new);
    let mut personality = use_signal(String::new);
    let mut step = use_signal(|| 1u8);

    if !props.visible {
        return rsx! {};
    }

    let current_step = *step.read();
    let is_next_disabled = current_step == 1 && name.read().trim().is_empty();
    let on_complete = props.on_complete;

    let go_next = move || {
        let s = *step.read();
        if s == 1 {
            if !name.read().trim().is_empty() {
                step.set(2);
            }
        } else if s == 2 {
            on_complete.call(HatchingResult {
                name: name.read().trim().to_string(),
                personality: {
                    let p = personality.read().trim().to_string();
                    if p.is_empty() { None } else { Some(p) }
                },
            });
        }
    };

    let go_back = move || {
        let s = *step.read();
        if s > 1 {
            step.set(s - 1);
        }
    };

    rsx! {
        div { class: "modal-backdrop",
            onclick: move |evt| {
                // Click outside the panel cancels.
                evt.stop_propagation();
                props.on_cancel.call(());
            },

            div {
                class: "modal",
                style: "max-width: 480px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "🥚 Hatching" }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| props.on_cancel.call(()),
                        "✕"
                    }
                }

                div { class: "modal-body",
                    div { class: "steps",
                        span { class: if current_step >= 1 { "step is-active" } else { "step" }, "1" }
                        span { class: "step-bar" }
                        span { class: if current_step >= 2 { "step is-active" } else { "step" }, "2" }
                    }

                    if current_step == 1 {
                        div { class: "field",
                            span { class: "field-label", "Agent name" }
                            input {
                                class: "input",
                                r#type: "text",
                                placeholder: "Give your agent a name",
                                value: "{name}",
                                autofocus: true,
                                oninput: move |evt| name.set(evt.value()),
                                onkeydown: move |evt: KeyboardEvent| {
                                    if evt.key() == Key::Enter && !name.read().trim().is_empty() {
                                        evt.prevent_default();
                                        let mut g = go_next;
                                        g();
                                    }
                                }
                            }
                            span { class: "field-help",
                                "This is how RustyClaw will refer to your agent in the UI."
                            }
                        }
                    } else if current_step == 2 {
                        div { class: "field",
                            span { class: "field-label", "Personality (optional)" }
                            textarea {
                                class: "textarea",
                                placeholder: "e.g. Friendly and curious, with a dry sense of humour",
                                rows: "4",
                                value: "{personality}",
                                oninput: move |evt| personality.set(evt.value()),
                            }
                            span { class: "field-help",
                                "Leave blank to use the default. You can change this later."
                            }
                        }
                    }
                }

                div { class: "modal-foot is-split",
                    if current_step > 1 {
                        button {
                            class: "btn btn-ghost",
                            onclick: move |_| {
                                let mut g = go_back;
                                g();
                            },
                            "‹ Back"
                        }
                    } else {
                        button {
                            class: "btn btn-ghost",
                            onclick: move |_| props.on_cancel.call(()),
                            "Cancel"
                        }
                    }

                    button {
                        class: "btn btn-primary",
                        disabled: is_next_disabled,
                        onclick: move |_| {
                            let mut g = go_next;
                            g();
                        },
                        if current_step == 2 { "✓ Complete" } else { "Next ›" }
                    }
                }
            }
        }
    }
}

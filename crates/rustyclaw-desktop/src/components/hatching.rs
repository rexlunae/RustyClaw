//! Hatching dialog for first-run identity setup.

use dioxus::prelude::*;
use dioxus_bulma::prelude::*;

/// Props for HatchingDialog.
#[derive(Props, Clone, PartialEq)]
pub struct HatchingDialogProps {
    /// Whether the dialog is visible
    pub visible: bool,
    /// Callback when hatching is complete
    pub on_complete: EventHandler<HatchingResult>,
    /// Callback to cancel
    pub on_cancel: EventHandler<()>,
}

/// Result of the hatching process.
#[derive(Clone, Debug)]
pub struct HatchingResult {
    pub name: String,
    pub personality: Option<String>,
}

/// Hatching dialog component.
#[component]
pub fn HatchingDialog(props: HatchingDialogProps) -> Element {
    let mut name = use_signal(String::new);
    let mut personality = use_signal(String::new);
    let mut step = use_signal(|| 1u32);

    let on_complete = props.on_complete;

    let handle_next = move |_| {
        let current_step = *step.read();
        if current_step == 1 && !name.read().trim().is_empty() {
            step.set(2);
        } else if current_step == 2 {
            on_complete.call(HatchingResult {
                name: name.read().trim().to_string(),
                personality: if personality.read().trim().is_empty() {
                    None
                } else {
                    Some(personality.read().trim().to_string())
                },
            });
        }
    };

    let handle_back = move |_| {
        let current_step = *step.read();
        if current_step > 1 {
            step.set(current_step - 1);
        }
    };

    if !props.visible {
        return rsx! {};
    }

    let current_step = *step.read();
    let is_next_disabled = current_step == 1 && name.read().trim().is_empty();

    rsx! {
        Modal {
            active: true,
            onclose: move |_| props.on_cancel.call(()),

            ModalCard { style: "max-width: 500px;",

                ModalCardHead {
                    onclose: move |_| props.on_cancel.call(()),
                    p { class: "modal-card-title",
                        Icon { i { class: "fas fa-egg" } }
                        " Hatching"
                    }
                }

                ModalCardBody {
                    // Progress indicator built from Bulma Tags.
                    div {
                        style: "display: flex; justify-content: center; align-items: center; margin-bottom: 1.5rem;",

                        Tag {
                            color: if current_step >= 1 { BulmaColor::Primary } else { BulmaColor::Light },
                            size: BulmaSize::Medium,
                            "1"
                        }
                        span { style: "width: 50px; height: 2px; background: #dbdbdb; margin: 0 0.5rem;" }
                        Tag {
                            color: if current_step >= 2 { BulmaColor::Primary } else { BulmaColor::Light },
                            size: BulmaSize::Medium,
                            "2"
                        }
                    }

                    match current_step {
                        1 => rsx! {
                            Content {
                                h4 { "What's your name?" }
                                p { class: "has-text-grey",
                                    "This will be used to identify your agent."
                                }

                                Field {
                                    Control {
                                        class: "has-icons-left".to_string(),
                                        // Bulma's Input doesn't expose `onkeypress`,
                                        // and we want Enter-to-advance here, so we
                                        // use a raw <input> with the `input` class
                                        // (still styled as a Bulma input).
                                        input {
                                            class: "input is-medium",
                                            r#type: "text",
                                            placeholder: "Enter agent name",
                                            value: "{name}",
                                            autofocus: true,
                                            oninput: move |evt| name.set(evt.value()),
                                            onkeypress: move |evt: KeyboardEvent| {
                                                if evt.key() == Key::Enter && !name.read().trim().is_empty() {
                                                    step.set(2);
                                                }
                                            },
                                        }
                                        Icon {
                                            class: "is-small is-left".to_string(),
                                            i { class: "fas fa-robot" }
                                        }
                                    }
                                }
                            }
                        },
                        2 => rsx! {
                            Content {
                                h4 { "Personality (optional)" }
                                p { class: "has-text-grey",
                                    "Describe your agent's personality or leave blank for default."
                                }

                                Field {
                                    Control {
                                        Textarea {
                                            placeholder: "e.g., Friendly and helpful, with a dry sense of humor".to_string(),
                                            rows: 4u32,
                                            value: personality.read().clone(),
                                            oninput: move |evt: FormEvent| personality.set(evt.value()),
                                        }
                                    }
                                }
                            }
                        },
                        _ => rsx! {}
                    }
                }

                ModalCardFoot { style: "justify-content: space-between;",
                    if current_step > 1 {
                        Button {
                            color: BulmaColor::Light,
                            onclick: handle_back,
                            Icon { i { class: "fas fa-arrow-left" } }
                            span { "Back" }
                        }
                    } else {
                        Button {
                            color: BulmaColor::Light,
                            onclick: move |_| props.on_cancel.call(()),
                            "Cancel"
                        }
                    }

                    Button {
                        color: BulmaColor::Primary,
                        disabled: is_next_disabled,
                        onclick: handle_next,

                        if current_step == 2 {
                            Icon { i { class: "fas fa-check" } }
                            span { "Complete" }
                        } else {
                            span { "Next" }
                            Icon { i { class: "fas fa-arrow-right" } }
                        }
                    }
                }
            }
        }
    }
}

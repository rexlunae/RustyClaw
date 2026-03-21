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
    let mut step = use_signal(|| 1);
    
    let on_complete = props.on_complete.clone();
    
    let handle_next = move |_| {
        if *step.read() == 1 && !name.read().trim().is_empty() {
            step.set(2);
        } else if *step.read() == 2 {
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
        if *step.read() > 1 {
            step.set(*step.read() - 1);
        }
    };
    
    if !props.visible {
        return rsx! {};
    }
    
    rsx! {
        div { class: "modal is-active",
            div { class: "modal-background",
                onclick: move |_| props.on_cancel.call(()),
            }
            
            div { class: "modal-card",
                style: "max-width: 500px;",
                
                header { class: "modal-card-head",
                    p { class: "modal-card-title",
                        span { class: "icon",
                            i { class: "fas fa-egg" }
                        }
                        " Hatching"
                    }
                }
                
                section { class: "modal-card-body",
                    // Progress indicator
                    div { class: "steps",
                        style: "display: flex; justify-content: center; margin-bottom: 1.5rem;",
                        
                        span { 
                            class: if *step.read() >= 1 { "tag is-primary is-medium" } else { "tag is-light is-medium" },
                            "1"
                        }
                        span { style: "width: 50px; height: 2px; background: #dbdbdb; align-self: center;" }
                        span { 
                            class: if *step.read() >= 2 { "tag is-primary is-medium" } else { "tag is-light is-medium" },
                            "2"
                        }
                    }
                    
                    match *step.read() {
                        1 => rsx! {
                            div { class: "content",
                                h4 { "What's your name?" }
                                p { class: "has-text-grey",
                                    "This will be used to identify your agent."
                                }
                                
                                Field {
                                    Control { class: "has-icons-left",
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
                                        span { class: "icon is-left",
                                            i { class: "fas fa-robot" }
                                        }
                                    }
                                }
                            }
                        },
                        2 => rsx! {
                            div { class: "content",
                                h4 { "Personality (optional)" }
                                p { class: "has-text-grey",
                                    "Describe your agent's personality or leave blank for default."
                                }
                                
                                Field {
                                    Control {
                                        textarea {
                                            class: "textarea",
                                            placeholder: "e.g., Friendly and helpful, with a dry sense of humor",
                                            rows: 4,
                                            value: "{personality}",
                                            oninput: move |evt| personality.set(evt.value()),
                                        }
                                    }
                                }
                            }
                        },
                        _ => rsx! {}
                    }
                }
                
                footer { class: "modal-card-foot",
                    style: "justify-content: space-between;",
                    
                    if *step.read() > 1 {
                        Button {
                            color: Color::Light,
                            onclick: handle_back,
                            
                            span { class: "icon",
                                i { class: "fas fa-arrow-left" }
                            }
                            span { "Back" }
                        }
                    } else {
                        Button {
                            color: Color::Light,
                            onclick: move |_| props.on_cancel.call(()),
                            "Cancel"
                        }
                    }
                    
                    Button {
                        color: Color::Primary,
                        disabled: *step.read() == 1 && name.read().trim().is_empty(),
                        onclick: handle_next,
                        
                        if *step.read() == 2 {
                            span { class: "icon",
                                i { class: "fas fa-check" }
                            }
                            span { "Complete" }
                        } else {
                            span { "Next" }
                            span { class: "icon",
                                i { class: "fas fa-arrow-right" }
                            }
                        }
                    }
                }
            }
        }
    }
}

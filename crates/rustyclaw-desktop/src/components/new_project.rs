//! New-project dialog: collect a name and a working directory.
//!
//! A project is a working directory that groups threads; creating one points
//! the agent's tools at that directory for the project's threads.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, FieldLabel, Help};

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct NewProjectDialogProps {
    /// Whether the dialog is rendered.
    pub visible: bool,
    /// User confirmed: `(name, path)`.
    pub on_create: EventHandler<(String, String)>,
    /// User dismissed the dialog.
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn NewProjectDialog(props: NewProjectDialogProps) -> Element {
    let mut name = use_signal(String::new);
    let mut path = use_signal(String::new);

    if !props.visible {
        return rsx! {};
    }

    let can_create = !name.read().trim().is_empty() && !path.read().trim().is_empty();
    let on_create = props.on_create;
    let on_cancel = props.on_cancel;
    let submit = move |_| {
        let n = name.read().trim().to_string();
        let p = path.read().trim().to_string();
        if !n.is_empty() && !p.is_empty() {
            on_create.call((n, p));
        }
    };

    rsx! {
        RcModal {
            active: true,
            title: "New project",
            width: 460,
            onclose: move |_| on_cancel.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        disabled: !can_create,
                        onclick: submit,
                        "Create project"
                    }
                }
            },

            Field {
                FieldLabel { "Name" }
                Control {
                    input {
                        class: "input",
                        r#type: "text",
                        value: "{name}",
                        placeholder: "My project",
                        autofocus: true,
                        oninput: move |evt| name.set(evt.value()),
                    }
                }
            }
            Field {
                FieldLabel { "Working directory" }
                Control {
                    input {
                        class: "input",
                        r#type: "text",
                        value: "{path}",
                        placeholder: "/home/you/code/my-project",
                        oninput: move |evt| path.set(evt.value()),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() == Key::Enter && can_create {
                                let n = name.read().trim().to_string();
                                let p = path.read().trim().to_string();
                                if !n.is_empty() && !p.is_empty() {
                                    on_create.call((n, p));
                                }
                            }
                        },
                    }
                }
                Help {
                    "The agent's tools run in this directory for this project's threads. It's created if it doesn't exist."
                }
            }
        }
    }
}

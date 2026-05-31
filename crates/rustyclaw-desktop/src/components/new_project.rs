//! New-project dialog: collect a name and a working directory.
//!
//! A project is a working directory that groups threads; creating one points
//! the agent's tools at that directory for the project's threads.

use dioxus::prelude::*;

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
        div { class: "modal-backdrop",
            onclick: move |_| on_cancel.call(()),
            div {
                class: "modal",
                style: "max-width: 460px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "New project" }
                }

                div { class: "modal-body",
                    div { class: "settings-section",
                        div { class: "field",
                            span { class: "field-label", "Name" }
                            input {
                                class: "input",
                                r#type: "text",
                                value: "{name}",
                                placeholder: "My project",
                                autofocus: true,
                                oninput: move |evt| name.set(evt.value()),
                            }
                        }
                        div { class: "field",
                            span { class: "field-label", "Working directory" }
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
                            span { class: "field-help",
                                "The agent's tools run in this directory for this project's threads. It's created if it doesn't exist."
                            }
                        }
                    }
                }

                div { class: "modal-foot",
                    button {
                        class: "btn btn-subtle",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: "btn btn-primary",
                        disabled: !can_create,
                        onclick: submit,
                        "Create project"
                    }
                }
            }
        }
    }
}

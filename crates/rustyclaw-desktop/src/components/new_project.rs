//! New-project dialog: collect a name and a working directory.
//!
//! A project is a working directory that groups threads; creating one points
//! the agent's tools at that directory for the project's threads.
//!
//! The directory field starts at the user's home directory and offers the
//! native folder picker, so the common case is "browse, confirm" rather than
//! typing a path from memory — and a picked folder also fills in the name.

use std::path::PathBuf;

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, FieldLabel, Help};

use super::RcModal;
use super::dir_picker::{example_path, home_path_text, name_from_path, pick_folder};

#[derive(Props, Clone, PartialEq)]
pub struct NewProjectDialogProps {
    /// Whether the dialog is rendered.
    pub visible: bool,
    /// User confirmed: `(name, path)`.
    pub on_create: EventHandler<(String, PathBuf)>,
    /// User dismissed the dialog.
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn NewProjectDialog(props: NewProjectDialogProps) -> Element {
    let mut name = use_signal(String::new);
    // Seeded with the home directory: a project usually lives under it, and an
    // absolute starting point means the path is never resolved against
    // whatever directory the gateway happens to be running in.
    let mut path = use_signal(home_path_text);

    if !props.visible {
        return rsx! {};
    }

    let can_create = !name.read().trim().is_empty() && !path.read().trim().is_empty();
    let on_create = props.on_create;
    let on_cancel = props.on_cancel;
    let submit = move || {
        let n = name.read().trim().to_string();
        let p = path.read().trim().to_string();
        if !n.is_empty() && !p.is_empty() {
            on_create.call((n, PathBuf::from(p)));
        }
    };
    let browse = move |_| {
        let start = path.read().clone();
        spawn(async move {
            if let Some(picked) = pick_folder(start).await {
                // Fill in the name from the folder, but never overwrite one the
                // user has already typed.
                if name.read().trim().is_empty()
                    && let Some(leaf) = name_from_path(&picked)
                {
                    name.set(leaf);
                }
                path.set(picked);
            }
        });
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
                        onclick: move |_| submit(),
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
                div { class: "field has-addons",
                    Control {
                        input {
                            class: "input",
                            r#type: "text",
                            value: "{path}",
                            placeholder: example_path("my-project"),
                            oninput: move |evt| path.set(evt.value()),
                            onkeydown: move |evt: KeyboardEvent| {
                                if evt.key() == Key::Enter && can_create {
                                    submit();
                                }
                            },
                        }
                    }
                    Control {
                        button {
                            class: "button",
                            r#type: "button",
                            onclick: browse,
                            "Browse…"
                        }
                    }
                }
                Help {
                    "The agent's tools run in this directory for this project's threads. It's created if it doesn't exist."
                }
            }
        }
    }
}

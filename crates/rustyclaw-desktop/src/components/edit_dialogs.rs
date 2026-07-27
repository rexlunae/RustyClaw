//! Edit dialogs for projects and threads.
//!
//! A project owns a working directory; its threads run there by default. A
//! thread may pin its own directory instead, which these dialogs make explicit
//! rather than implicit: the thread dialog shows the inherited directory and
//! requires a deliberate toggle to override it.
//!
//! Both dialogs are mounted only while open (the caller renders them from an
//! `Option`), so their fields initialise from the current values every time.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, FieldLabel, Help};

use super::RcModal;

/// Open a native folder picker starting at `start`, returning the choice.
async fn pick_folder(start: String) -> Option<String> {
    let start = if start.trim().is_empty() {
        ".".to_string()
    } else {
        start
    };
    rfd::AsyncFileDialog::new()
        .set_directory(start)
        .pick_folder()
        .await
        .map(|folder| folder.path().display().to_string())
}

// ── Project ─────────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct EditProjectDialogProps {
    pub project_id: u64,
    /// Current project name.
    pub name: String,
    /// Current working directory.
    pub path: String,
    /// User confirmed: `(project_id, name, path)`.
    pub on_save: EventHandler<(u64, String, String)>,
    /// User dismissed the dialog.
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn EditProjectDialog(props: EditProjectDialogProps) -> Element {
    let mut name = use_signal(|| props.name.clone());
    let mut path = use_signal(|| props.path.clone());

    let project_id = props.project_id;
    let on_save = props.on_save;
    let on_cancel = props.on_cancel;

    let can_save = !name.read().trim().is_empty() && !path.read().trim().is_empty();
    let submit = move || {
        let n = name.read().trim().to_string();
        let p = path.read().trim().to_string();
        if !n.is_empty() && !p.is_empty() {
            on_save.call((project_id, n, p));
        }
    };

    rsx! {
        RcModal {
            active: true,
            title: "Edit project",
            width: 480,
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
                        disabled: !can_save,
                        onclick: move |_| submit(),
                        "Save changes"
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
                            placeholder: "/home/you/code/my-project",
                            oninput: move |evt| path.set(evt.value()),
                            onkeydown: move |evt: KeyboardEvent| {
                                if evt.key() == Key::Enter {
                                    submit();
                                }
                            },
                        }
                    }
                    Control {
                        button {
                            class: "button",
                            r#type: "button",
                            onclick: move |_| {
                                let start = path.read().clone();
                                spawn(async move {
                                    if let Some(picked) = pick_folder(start).await {
                                        path.set(picked);
                                    }
                                });
                            },
                            "Browse…"
                        }
                    }
                }
                Help {
                    "Threads in this project run here unless they set their own directory. It's created if it doesn't exist."
                }
            }
        }
    }
}

// ── Thread ──────────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct EditThreadDialogProps {
    pub thread_id: u64,
    /// Current caption.
    pub label: String,
    /// Current working-directory override, or `None` when the thread inherits
    /// its project's directory.
    pub working_dir: Option<String>,
    /// The project directory this thread inherits when it has no override.
    /// Shown so the effective directory is never a mystery.
    pub project_path: String,
    /// User confirmed: `(thread_id, label, working_dir)`. `None` clears the
    /// override.
    pub on_save: EventHandler<(u64, String, Option<String>)>,
    /// User dismissed the dialog.
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn EditThreadDialog(props: EditThreadDialogProps) -> Element {
    let mut label = use_signal(|| props.label.clone());
    // Seed the path field with the inherited directory so switching the
    // override on gives you somewhere to edit from rather than a blank box.
    let mut path = use_signal(|| {
        props
            .working_dir
            .clone()
            .unwrap_or_else(|| props.project_path.clone())
    });
    let mut overridden = use_signal(|| props.working_dir.is_some());

    let thread_id = props.thread_id;
    let inherited = props.project_path.clone();
    let inherited_display = if inherited.is_empty() {
        "the project's directory".to_string()
    } else {
        inherited
    };
    let on_save = props.on_save;
    let on_cancel = props.on_cancel;

    let is_overridden = *overridden.read();
    let can_save =
        !label.read().trim().is_empty() && (!is_overridden || !path.read().trim().is_empty());
    let submit = move || {
        let l = label.read().trim().to_string();
        if l.is_empty() {
            return;
        }
        let dir = if *overridden.read() {
            let p = path.read().trim().to_string();
            if p.is_empty() {
                return;
            }
            Some(p)
        } else {
            None
        };
        on_save.call((thread_id, l, dir));
    };

    rsx! {
        RcModal {
            active: true,
            title: "Edit thread",
            width: 480,
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
                        disabled: !can_save,
                        onclick: move |_| submit(),
                        "Save changes"
                    }
                }
            },

            Field {
                FieldLabel { "Caption" }
                Control {
                    input {
                        class: "input",
                        r#type: "text",
                        value: "{label}",
                        placeholder: "What this thread is about",
                        autofocus: true,
                        oninput: move |evt| label.set(evt.value()),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() == Key::Enter {
                                submit();
                            }
                        },
                    }
                }
            }
            Field {
                FieldLabel { "Working directory" }
                Control {
                    label { class: "checkbox",
                        input {
                            r#type: "checkbox",
                            checked: is_overridden,
                            onchange: move |evt| overridden.set(evt.checked()),
                        }
                        " Use a different directory for this thread"
                    }
                }
                if is_overridden {
                    div { class: "field has-addons",
                        Control {
                            input {
                                class: "input",
                                r#type: "text",
                                value: "{path}",
                                placeholder: "/home/you/code/some-worktree",
                                oninput: move |evt| path.set(evt.value()),
                            }
                        }
                        Control {
                            button {
                                class: "button",
                                r#type: "button",
                                onclick: move |_| {
                                    let start = path.read().clone();
                                    spawn(async move {
                                        if let Some(picked) = pick_folder(start).await {
                                            path.set(picked);
                                        }
                                    });
                                },
                                "Browse…"
                            }
                        }
                    }
                    Help { "Created if it doesn't exist. Clear the checkbox to go back to the project's directory." }
                } else {
                    Help { "Inherits {inherited_display}" }
                }
            }
        }
    }
}

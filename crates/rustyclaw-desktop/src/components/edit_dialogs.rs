//! Edit dialogs for projects and threads.
//!
//! A project owns a working directory; its threads run there by default. A
//! thread may pin its own directory instead, which these dialogs make explicit
//! rather than implicit: the thread dialog shows the inherited directory and
//! requires a deliberate toggle to override it.
//!
//! Both dialogs are mounted only while open (the caller renders them from an
//! `Option`), so their fields initialise from the current values every time.

use std::path::PathBuf;

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, FieldLabel, Help};

use super::RcModal;
use super::dir_picker::{example_path, pick_folder};

/// A path as editable text.
///
/// Lossy for a path that isn't valid UTF-8, and unavoidably so — a text input
/// holds text. Confining the conversion to the dialogs means a path the user
/// never edits is carried as a `PathBuf` end to end and never laundered.
fn path_field(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The text field back to a path. Trimmed, because surrounding whitespace in
/// a text box is a typo rather than part of a directory name; empty means the
/// user cleared it.
fn field_path(text: &str) -> Option<PathBuf> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

// ── Project ─────────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct EditProjectDialogProps {
    pub project_id: u64,
    /// Current project name.
    pub name: String,
    /// Current working directory.
    pub path: PathBuf,
    /// User confirmed: `(project_id, name, path)`.
    pub on_save: EventHandler<(u64, String, PathBuf)>,
    /// User dismissed the dialog.
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn EditProjectDialog(props: EditProjectDialogProps) -> Element {
    let mut name = use_signal(|| props.name.clone());
    let mut path = use_signal(|| path_field(&props.path));

    let project_id = props.project_id;
    let on_save = props.on_save;
    let on_cancel = props.on_cancel;

    let can_save = !name.read().trim().is_empty() && !path.read().trim().is_empty();
    let submit = move || {
        let n = name.read().trim().to_string();
        let p = field_path(&path.read());
        if let (false, Some(p)) = (n.is_empty(), p) {
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
                            placeholder: example_path("my-project"),
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
    pub working_dir: Option<PathBuf>,
    /// The project directory this thread inherits when it has no override.
    /// Shown so the effective directory is never a mystery.
    pub project_path: PathBuf,
    /// The thread's current project id (its own, or the active project's when
    /// the thread belongs to the implicit default).
    pub project_id: u64,
    /// Every project, `(id, name)`, for the move-to-project selector.
    pub projects: Vec<(u64, String)>,
    /// User confirmed: `(thread_id, label, working_dir, project_id)`. The
    /// caller moves the thread when the project changed. `None` clears the
    /// directory override.
    pub on_save: EventHandler<(u64, String, Option<PathBuf>, u64)>,
    /// User asked to download a copy of the transcript. Carries the thread id.
    pub on_download: EventHandler<u64>,
    /// User dismissed the dialog.
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn EditThreadDialog(props: EditThreadDialogProps) -> Element {
    let mut label = use_signal(|| props.label.clone());
    // Seed the path field with the inherited directory so switching the
    // override on gives you somewhere to edit from rather than a blank box.
    let mut path = use_signal(|| {
        path_field(
            props
                .working_dir
                .as_deref()
                .unwrap_or(props.project_path.as_path()),
        )
    });
    let mut overridden = use_signal(|| props.working_dir.is_some());
    let mut project = use_signal(|| props.project_id);

    let thread_id = props.thread_id;
    let inherited_display = if props.project_path.as_os_str().is_empty() {
        "the project's directory".to_string()
    } else {
        path_field(&props.project_path)
    };
    let on_save = props.on_save;
    let on_download = props.on_download;
    let on_cancel = props.on_cancel;
    let projects = props.projects;

    let is_overridden = *overridden.read();
    let can_save =
        !label.read().trim().is_empty() && (!is_overridden || !path.read().trim().is_empty());
    let submit = move || {
        let l = label.read().trim().to_string();
        if l.is_empty() {
            return;
        }
        let dir = if *overridden.read() {
            match field_path(&path.read()) {
                Some(p) => Some(p),
                // The override is on but the box is empty — nothing to save.
                None => return,
            }
        } else {
            None
        };
        on_save.call((thread_id, l, dir, *project.read()));
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
                        onclick: move |_| on_download.call(thread_id),
                        "Download copy…"
                    }
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
                FieldLabel { "Project" }
                Control {
                    div { class: "select",
                        select {
                            onchange: move |evt| {
                                if let Ok(id) = evt.value().parse::<u64>() {
                                    project.set(id);
                                }
                            },
                            for (id, name) in &projects {
                                option {
                                    value: "{id}",
                                    selected: *id == *project.read(),
                                    "{name}"
                                }
                            }
                        }
                    }
                }
                Help { "Moves this thread to the chosen project. Its conversation and directory override, if any, stay with it." }
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
                                placeholder: example_path("some-worktree"),
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

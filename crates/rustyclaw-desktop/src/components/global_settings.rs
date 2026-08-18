//! Global settings dialog: agent name, system-prompt override, and the
//! workspace files that get injected into system prompts (SOUL.md, AGENTS.md,
//! MEMORY.md, …), editable in place.
//!
//! The caller mounts this dialog only while it is open AND the view has been
//! loaded (rendering a "Loading…" modal itself while the fetch is in flight),
//! and passes a `key` derived from the loaded data so a re-fetch after a save
//! remounts the dialog with fresh values. The edit signals therefore always
//! seed from current data at mount, and local edits never touch app state
//! until Save.

use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, FieldLabel, Help};

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct GlobalSettingsDialogProps {
    pub visible: bool,
    /// The view loaded from the gateway. Always `Some` when mounted by the
    /// caller; `None` renders a loading placeholder as a safety net.
    pub data: Option<rustyclaw_view::GlobalSettingsData>,
    pub on_close: EventHandler<()>,
    /// User saved: send the new config values plus every workspace file whose
    /// content changed.
    pub on_save: EventHandler<rustyclaw_view::GlobalSettingsSaveData>,
}

#[component]
pub fn GlobalSettingsDialog(props: GlobalSettingsDialogProps) -> Element {
    let mut agent_name = use_signal(|| {
        props
            .data
            .as_ref()
            .map(|d| d.agent_name.clone())
            .unwrap_or_default()
    });
    let mut system_prompt = use_signal(|| {
        props
            .data
            .as_ref()
            .and_then(|d| d.system_prompt.clone())
            .unwrap_or_default()
    });
    let mut selected = use_signal(|| 0usize);
    // Per-file edits keyed by file name; a missing entry means "unchanged".
    let mut edits: Signal<HashMap<String, String>> = use_signal(HashMap::new);

    if !props.visible {
        return rsx! {};
    }

    let Some(data) = props.data.clone() else {
        return rsx! {
            RcModal {
                active: true,
                title: "Global Settings",
                width: 640,
                onclose: move |_| props.on_close.call(()),
                p { class: "has-text-grey", "Loading global settings…" }
            }
        };
    };

    let selected_idx = *selected.read();
    let selected_name = data
        .files
        .get(selected_idx)
        .map(|f| f.name.clone())
        .unwrap_or_default();
    let selected_file = data.files.get(selected_idx).cloned();

    // The content shown for the selected file: the user's edit if any,
    // otherwise the value from the gateway.
    let file_content = {
        let edits = edits.read();
        edits
            .get(&selected_name)
            .cloned()
            .or_else(|| {
                data.files
                    .iter()
                    .find(|f| f.name == selected_name)
                    .map(|f| f.content.clone())
            })
            .unwrap_or_default()
    };

    let on_save = props.on_save;
    let save_data = data.clone();
    let save = move |_| {
        let payload = rustyclaw_view::GlobalSettingsSaveData {
            agent_name: agent_name.read().trim().to_string(),
            system_prompt: {
                let sp = system_prompt.read().clone();
                (!sp.is_empty()).then_some(sp)
            },
            workspace_files: {
                let edits = edits.read();
                save_data
                    .files
                    .iter()
                    .filter_map(|f| {
                        edits.get(&f.name).and_then(|content| {
                            (content != &f.content).then(|| {
                                rustyclaw_core::gateway::protocol::frames::WorkspaceFileEdit {
                                    name: f.name.clone(),
                                    content: content.clone(),
                                }
                            })
                        })
                    })
                    .collect()
            },
        };
        on_save.call(payload);
    };

    // The file editor's oninput handler owns a copy of the selected name so
    // it can insert into the edits map without borrowing `data`.
    let edit_key = selected_name.clone();

    rsx! {
        RcModal {
            active: true,
            title: "Global Settings",
            width: 760,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| props.on_close.call(()),
                        "Cancel"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        onclick: save,
                        "Save"
                    }
                }
            },

            div { class: "block",
                p { class: "has-text-grey is-size-7",
                    "These values are injected into every system prompt. ",
                    "Workspace files that do not exist yet are created on save."
                }
            }

            // ── Agent identity ────────────────────────────────────────────
            Field {
                FieldLabel { "Agent name" }
                Control {
                    input {
                        class: "input",
                        r#type: "text",
                        value: "{agent_name}",
                        placeholder: "Agent name",
                        oninput: move |e| agent_name.set(e.value()),
                    }
                }
                Help { "Shown as the assistant's identity; also used by the desktop and TUI." }
            }

            // ── System prompt override ─────────────────────────────────────
            Field {
                FieldLabel { "System prompt override" }
                Control {
                    textarea {
                        class: "textarea global-settings-system-prompt",
                        rows: "5",
                        value: "{system_prompt}",
                        placeholder: "Optional — leave empty to use the built-in prompt",
                        oninput: move |e| system_prompt.set(e.value()),
                    }
                }
                Help { "When empty, the agent uses its built-in system prompt." }
            }

            // ── Workspace files ────────────────────────────────────────────
            Field {
                FieldLabel { "Workspace files" }
                Control {
                    select {
                        class: "is-fullwidth",
                        value: "{selected}",
                        onchange: move |e| {
                            if let Ok(i) = e.value().parse::<usize>() {
                                selected.set(i);
                            }
                        },
                        for (i, file) in data.files.iter().enumerate() {
                            option { value: "{i}", "{file.name}" }
                        }
                    }
                }
                if let Some(ref file) = selected_file {
                    Help {
                        "{file.name} — {file.status_label()}"
                    }
                    if file.truncated {
                        Help { class: "has-text-warning",
                            "File is larger than the transport limit; the visible content is truncated. Edit carefully — saving writes this content verbatim."
                        }
                    }
                    Control {
                        textarea {
                            class: "textarea global-settings-file-editor",
                            rows: "14",
                            value: "{file_content}",
                            placeholder: "(empty file)",
                            oninput: move |e| {
                                edits.write().insert(edit_key.clone(), e.value());
                            },
                        }
                    }
                }
                Help {
                    "Edited files are written to the workspace: ",
                    code { "{data.workspace_dir}" }
                }
            }

            if selected_name.is_empty() {
                p { class: "has-text-grey", "No workspace files available." }
            }
        }
    }
}

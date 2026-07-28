//! Prompt shown when a workspace change would discard unsaved editor changes.
//!
//! Changing the working directory — directly, or by switching project or
//! thread — invalidates every path the editor holds, because `Workspace*`
//! paths are relative to the current directory. Unsaved work therefore cannot
//! simply come along, and dropping it without asking would be the kind of
//! silent loss this codebase has been burned by. So the user decides.

use std::path::PathBuf;

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons};

use super::RcModal;

/// What the user chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedChoice {
    /// Write the changes, then go ahead.
    Save,
    /// Go ahead and lose them.
    Discard,
    /// Stay where we are.
    Cancel,
}

#[derive(Props, Clone, PartialEq)]
pub struct UnsavedChangesDialogProps {
    /// Files with unsaved changes, for display.
    pub files: Vec<PathBuf>,
    /// Where the workspace is about to move, phrased for a sentence.
    pub destination: String,
    pub on_choose: EventHandler<UnsavedChoice>,
}

#[component]
pub fn UnsavedChangesDialog(props: UnsavedChangesDialogProps) -> Element {
    let on_choose = props.on_choose;
    let count = props.files.len();
    let noun = if count == 1 { "file has" } else { "files have" };

    rsx! {
        RcModal {
            active: true,
            title: "Unsaved changes",
            width: 480,
            // Dismissing without choosing means "don't move", which is the
            // only choice that cannot lose anything.
            onclose: move |_| on_choose.call(UnsavedChoice::Cancel),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| on_choose.call(UnsavedChoice::Cancel),
                        "Cancel"
                    }
                    Button {
                        color: BulmaColor::Danger,
                        onclick: move |_| on_choose.call(UnsavedChoice::Discard),
                        "Don't save"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| on_choose.call(UnsavedChoice::Save),
                        "Save"
                    }
                }
            },

            p {
                "{count} {noun} unsaved changes. Moving to {props.destination} closes them, "
                "because the editor's files belong to the current working directory."
            }
            ul { class: "unsaved-list",
                for file in props.files.iter() {
                    li { key: "{file.display()}", "{file.display()}" }
                }
            }
        }
    }
}

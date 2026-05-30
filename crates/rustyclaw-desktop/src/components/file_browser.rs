//! File browser panel for the right-hand sidebar.
//!
//! Renders the `FileBrowserData` from `rustyclaw-view` as a VS Code-style
//! collapsible directory tree.  Changes to the root directory are handled
//! by the parent via `on_change_root`; expansion/selection events are
//! emitted so the parent can update state.

use std::path::PathBuf;

use dioxus::prelude::*;
use rustyclaw_view::FileBrowserData;

#[derive(Props, Clone, PartialEq)]
pub struct FileBrowserProps {
    pub data: FileBrowserData,
    /// Emitted when a file is clicked (to attach or open).
    pub on_select: EventHandler<PathBuf>,
    /// Emitted when a directory entry is toggled (expand/collapse).
    pub on_toggle: EventHandler<PathBuf>,
}

#[component]
pub fn FileBrowser(props: FileBrowserProps) -> Element {
    let root_label: String = props
        .data
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| props.data.root.display().to_string());

    let root_display = props.data.root.display().to_string();

    rsx! {
        div { class: "file-browser",
            div { class: "file-browser-header",
                span { class: "file-browser-root-label", title: "{root_display}", "{root_label}" }
            }
            div { class: "file-browser-tree",
                if props.data.entries.is_empty() {
                    div { class: "file-browser-empty", "No files" }
                } else {
                    for entry in props.data.entries.iter() {
                        {
                            let path = entry.path.clone();
                            let path_display = path.display().to_string();
                            let is_dir = entry.is_dir;
                            let is_selected = entry.is_selected;
                            let icon = entry.icon();
                            let indent_style = format!("padding-left: {}px", 6 + entry.depth * 14);
                            let cls = if is_selected {
                                "file-browser-entry is-selected"
                            } else if is_dir {
                                "file-browser-entry is-dir"
                            } else {
                                "file-browser-entry"
                            };
                            let toggle_path = path.clone();
                            let select_path = path.clone();
                            rsx! {
                                div {
                                    key: "{path_display}",
                                    class: "{cls}",
                                    style: "{indent_style}",
                                    onclick: move |_| {
                                        if is_dir {
                                            props.on_toggle.call(toggle_path.clone());
                                        } else {
                                            props.on_select.call(select_path.clone());
                                        }
                                    },
                                    span { class: "file-browser-icon", "{icon}" }
                                    span { class: "file-browser-name", "{entry.name}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

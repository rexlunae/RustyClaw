//! File editor — a native plugin for the plugin dock.
//!
//! Renders a file tree and open-file tabs over the thread's effective working
//! directory. Everything it touches goes through the `Workspace*` frames, which
//! confine access to that directory (see the gateway's `workspace_files`), so
//! the editor cannot reach a file the thread has no business editing.
//!
//! This is a *native* plugin: a compiled-in Dioxus component that registers in
//! the plugin registry rather than HTML shipped from a plugin directory. It is
//! listed and selected like any other plugin, and shares the dock with them —
//! which is why it may not be the visible one.
//!
//! Editing state (which files are open, what has been typed) lives in
//! [`AppState`](crate::state::AppState) rather than here, so switching to
//! another plugin and back does not discard unsaved work.

use std::path::{Path, PathBuf};

use dioxus::prelude::*;
use rustyclaw_core::gateway::WorkspaceEntryDto;

/// The name this plugin registers under.
pub const EDITOR_PLUGIN: &str = "editor";

/// What the editor asks the parent to do. The parent owns the gateway client,
/// so the component stays free of transport concerns and is testable as a
/// pure rendering of its props.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorAction {
    /// Load a directory's entries (expanding the tree, or the initial root).
    ListDir(PathBuf),
    /// Load a file's contents (opening it).
    OpenFile(PathBuf),
    /// Persist a file's edited contents.
    Save { path: PathBuf, content: String },
}

#[derive(Props, Clone, PartialEq)]
pub struct EditorProps {
    /// Directory listings, keyed by directory path relative to the workspace.
    pub listings: std::collections::HashMap<PathBuf, Vec<WorkspaceEntryDto>>,
    /// Loaded file contents, keyed by path.
    pub files: std::collections::HashMap<PathBuf, String>,
    /// Unsaved edits, keyed by path.
    pub edits: std::collections::HashMap<PathBuf, String>,
    /// Directories the tree has expanded.
    pub expanded: std::collections::HashSet<PathBuf>,
    /// Open tabs, in order.
    pub open: Vec<PathBuf>,
    /// The visible tab.
    pub active: Option<PathBuf>,
    /// The thread's working directory, for the header.
    pub root_label: String,

    pub on_action: EventHandler<EditorAction>,
    pub on_toggle_dir: EventHandler<PathBuf>,
    pub on_select_tab: EventHandler<PathBuf>,
    pub on_close_tab: EventHandler<PathBuf>,
    /// The user typed: `(path, new contents)`.
    pub on_edit: EventHandler<(PathBuf, String)>,
}

/// The text currently in the buffer: the edit if there is one, else what was
/// loaded. Returning `None` means the file has not arrived yet.
fn buffer_text<'a>(
    path: &Path,
    files: &'a std::collections::HashMap<PathBuf, String>,
    edits: &'a std::collections::HashMap<PathBuf, String>,
) -> Option<&'a String> {
    edits.get(path).or_else(|| files.get(path))
}

/// Whether a file has unsaved changes.
///
/// An edit that happens to restore the loaded text is *not* dirty — comparing
/// content rather than tracking a flag means the marker cannot drift out of
/// sync with what is actually in the buffer.
fn is_dirty(
    path: &Path,
    files: &std::collections::HashMap<PathBuf, String>,
    edits: &std::collections::HashMap<PathBuf, String>,
) -> bool {
    match (edits.get(path), files.get(path)) {
        (Some(edited), Some(loaded)) => edited != loaded,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Final path component, for display.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[component]
pub fn Editor(props: EditorProps) -> Element {
    let root = PathBuf::new();
    let on_action = props.on_action;

    // Ask for the root listing once, when it has never been requested. Doing
    // this on mount rather than making the parent remember to prime it keeps
    // the editor self-starting when the dock first shows it.
    let has_root = props.listings.contains_key(&root);
    use_effect(move || {
        if !has_root {
            on_action.call(EditorAction::ListDir(PathBuf::new()));
        }
    });

    let active_text = props
        .active
        .as_ref()
        .and_then(|p| buffer_text(p, &props.files, &props.edits))
        .cloned();
    let active_dirty = props
        .active
        .as_ref()
        .map(|p| is_dirty(p, &props.files, &props.edits))
        .unwrap_or(false);

    rsx! {
        div { class: "editor",
            div { class: "editor-header",
                span { class: "editor-root", title: "{props.root_label}", "{props.root_label}" }
            }

            div { class: "editor-body",
                div { class: "editor-tree",
                    EditorTree {
                        dir: root.clone(),
                        depth: 0,
                        listings: props.listings.clone(),
                        expanded: props.expanded.clone(),
                        edits: props.edits.clone(),
                        files: props.files.clone(),
                        active: props.active.clone(),
                        on_action: props.on_action,
                        on_toggle_dir: props.on_toggle_dir,
                    }
                }

                div { class: "editor-pane",
                    if props.open.is_empty() {
                        div { class: "editor-empty", "Select a file to edit" }
                    } else {
                        div { class: "editor-tabs",
                            for path in props.open.iter() {
                                {
                                    let p = path.clone();
                                    let is_active = props.active.as_ref() == Some(path);
                                    let dirty = is_dirty(path, &props.files, &props.edits);
                                    let cls = if is_active { "editor-tab active" } else { "editor-tab" };
                                    let close_path = p.clone();
                                    let select_path = p.clone();
                                    let on_select = props.on_select_tab;
                                    let on_close = props.on_close_tab;
                                    rsx! {
                                        div {
                                            key: "{p.display()}",
                                            class: "{cls}",
                                            title: "{p.display()}",
                                            onclick: move |_| on_select.call(select_path.clone()),
                                            span { class: "editor-tab-name", "{file_name(&p)}" }
                                            if dirty {
                                                span { class: "editor-tab-dirty", title: "Unsaved changes", "●" }
                                            }
                                            button {
                                                class: "editor-tab-close",
                                                "aria-label": "Close tab",
                                                onclick: move |evt| {
                                                    evt.stop_propagation();
                                                    on_close.call(close_path.clone());
                                                },
                                                "✕"
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(active) = props.active.clone() {
                            match active_text {
                                Some(text) => {
                                    let save_path = active.clone();
                                    let edit_path = active.clone();
                                    let on_edit = props.on_edit;
                                    let act = props.on_action;
                                    let save_text = text.clone();
                                    rsx! {
                                        div { class: "editor-toolbar",
                                            span { class: "editor-path", "{active.display()}" }
                                            button {
                                                class: "editor-save",
                                                disabled: !active_dirty,
                                                title: "Save (Ctrl+S)",
                                                onclick: move |_| {
                                                    act.call(EditorAction::Save {
                                                        path: save_path.clone(),
                                                        content: save_text.clone(),
                                                    });
                                                },
                                                if active_dirty { "Save" } else { "Saved" }
                                            }
                                        }
                                        textarea {
                                            class: "editor-text",
                                            spellcheck: false,
                                            value: "{text}",
                                            oninput: move |evt| on_edit.call((edit_path.clone(), evt.value())),
                                            onkeydown: {
                                                let path = active.clone();
                                                move |evt: KeyboardEvent| {
                                                    // Ctrl/Cmd+S saves, as the shortcut is
                                                    // reflexive in any editor.
                                                    if evt.key() == Key::Character("s".into())
                                                        && (evt.modifiers().ctrl() || evt.modifiers().meta())
                                                    {
                                                        evt.prevent_default();
                                                        act.call(EditorAction::Save {
                                                            path: path.clone(),
                                                            content: text.clone(),
                                                        });
                                                    }
                                                }
                                            },
                                        }
                                    }
                                }
                                // The open request is in flight. Saying so beats
                                // an empty box that looks like an empty file.
                                None => rsx! {
                                    div { class: "editor-empty", "Loading {active.display()}…" }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct EditorTreeProps {
    dir: PathBuf,
    depth: usize,
    listings: std::collections::HashMap<PathBuf, Vec<WorkspaceEntryDto>>,
    expanded: std::collections::HashSet<PathBuf>,
    edits: std::collections::HashMap<PathBuf, String>,
    files: std::collections::HashMap<PathBuf, String>,
    active: Option<PathBuf>,
    on_action: EventHandler<EditorAction>,
    on_toggle_dir: EventHandler<PathBuf>,
}

/// One directory level, recursing into expanded subdirectories.
#[component]
fn EditorTree(props: EditorTreeProps) -> Element {
    let Some(entries) = props.listings.get(&props.dir) else {
        // Requested but not yet arrived (or never requested — the parent asks
        // on expand). Render nothing rather than a misleading "empty".
        return rsx! {};
    };

    if entries.is_empty() {
        return rsx! {
            div {
                class: "editor-tree-empty",
                style: "padding-left: {8 + props.depth * 12}px",
                "empty"
            }
        };
    }

    rsx! {
        for entry in entries.iter() {
            {
                let path = entry.path.clone();
                let is_dir = entry.is_dir;
                let is_expanded = props.expanded.contains(&path);
                let is_active = props.active.as_ref() == Some(&path);
                let dirty = !is_dir && is_dirty(&path, &props.files, &props.edits);
                let indent = format!("padding-left: {}px", 8 + props.depth * 12);
                let mut cls = String::from("editor-tree-row");
                if is_active {
                    cls.push_str(" active");
                }
                let click_path = path.clone();
                let on_action = props.on_action;
                let on_toggle_dir = props.on_toggle_dir;
                rsx! {
                    div {
                        key: "{path.display()}",
                        class: "{cls}",
                        style: "{indent}",
                        title: "{path.display()}",
                        onclick: move |_| {
                            if is_dir {
                                on_toggle_dir.call(click_path.clone());
                            } else {
                                on_action.call(EditorAction::OpenFile(click_path.clone()));
                            }
                        },
                        span { class: "editor-tree-icon",
                            if is_dir {
                                if is_expanded { "▾" } else { "▸" }
                            } else {
                                "·"
                            }
                        }
                        span { class: "editor-tree-name", "{entry.name}" }
                        if dirty {
                            span { class: "editor-tab-dirty", "●" }
                        }
                    }
                    if is_dir && is_expanded {
                        EditorTree {
                            dir: path.clone(),
                            depth: props.depth + 1,
                            listings: props.listings.clone(),
                            expanded: props.expanded.clone(),
                            edits: props.edits.clone(),
                            files: props.files.clone(),
                            active: props.active.clone(),
                            on_action: props.on_action,
                            on_toggle_dir: props.on_toggle_dir,
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> std::collections::HashMap<PathBuf, String> {
        pairs
            .iter()
            .map(|(k, v)| (PathBuf::from(k), v.to_string()))
            .collect()
    }

    #[test]
    fn a_file_with_no_edit_is_clean() {
        let files = map(&[("a.rs", "fn main() {}")]);
        let edits = map(&[]);
        assert!(!is_dirty(Path::new("a.rs"), &files, &edits));
    }

    #[test]
    fn a_changed_edit_is_dirty() {
        let files = map(&[("a.rs", "fn main() {}")]);
        let edits = map(&[("a.rs", "fn main() { todo!() }")]);
        assert!(is_dirty(Path::new("a.rs"), &files, &edits));
    }

    /// Typing and then undoing back to the original leaves the file clean.
    /// Comparing content rather than setting a flag is what makes this fall
    /// out for free — a flag would still claim the file was modified.
    #[test]
    fn an_edit_that_restores_the_original_is_not_dirty() {
        let files = map(&[("a.rs", "fn main() {}")]);
        let edits = map(&[("a.rs", "fn main() {}")]);
        assert!(!is_dirty(Path::new("a.rs"), &files, &edits));
    }

    #[test]
    fn the_buffer_prefers_the_edit_over_the_loaded_text() {
        let files = map(&[("a.rs", "loaded")]);
        let edits = map(&[("a.rs", "typed")]);
        assert_eq!(
            buffer_text(Path::new("a.rs"), &files, &edits).map(String::as_str),
            Some("typed")
        );
    }

    /// A file that has not arrived yet has no buffer, which is what lets the
    /// pane say "Loading…" instead of showing an empty box that reads as an
    /// empty file.
    #[test]
    fn an_unloaded_file_has_no_buffer() {
        assert_eq!(buffer_text(Path::new("a.rs"), &map(&[]), &map(&[])), None);
    }

    #[test]
    fn file_name_uses_the_final_component() {
        assert_eq!(file_name(Path::new("src/app/mod.rs")), "mod.rs");
        assert_eq!(file_name(Path::new("README.md")), "README.md");
    }
}

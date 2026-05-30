//! Component data for the file-browser sidebar panel.
//!
//! Both clients render an identical file-tree panel on the right-hand
//! sidebar.  This module owns the platform-agnostic data shapes so
//! neither client has to invent its own listing logic.

use std::path::PathBuf;

/// A single entry in the file browser tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileBrowserEntry {
    /// Absolute path to this entry.
    pub path: PathBuf,

    /// Display name (file/directory basename only).
    pub name: String,

    /// Whether this entry is a directory.
    pub is_dir: bool,

    /// Nesting depth (0 = root item, 1 = one level deep, …).
    pub depth: usize,

    /// Whether a directory is expanded (irrelevant for files).
    pub is_expanded: bool,

    /// Whether this entry is currently selected.
    pub is_selected: bool,
}

impl FileBrowserEntry {
    /// Icon glyph appropriate for this entry.
    pub fn icon(&self) -> &'static str {
        if self.is_dir {
            if self.is_expanded { "▾" } else { "▸" }
        } else {
            "·"
        }
    }

    /// Indentation prefix (2 spaces per depth level).
    pub fn indent(&self) -> String {
        "  ".repeat(self.depth)
    }
}

/// Data for the full file-browser panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileBrowserData {
    /// Absolute path to the root directory being browsed.
    pub root: PathBuf,

    /// Flat list of currently visible entries (directories may be
    /// collapsed, hiding their children).
    pub entries: Vec<FileBrowserEntry>,

    /// Currently selected entry path (if any).
    pub selected: Option<PathBuf>,
}

impl FileBrowserData {
    /// Build a file-browser snapshot for `root_path`.
    ///
    /// Lists the directory one level deep (non-recursive); directories
    /// are not expanded by default.
    pub fn load(root_path: impl Into<PathBuf>) -> Self {
        let root: PathBuf = root_path.into();
        let mut entries = Vec::new();

        if let Ok(dir) = std::fs::read_dir(&root) {
            let mut items: Vec<_> = dir.filter_map(|e| e.ok()).collect();
            // Directories first, then files; alphabetical within each group.
            items.sort_by(|a, b| {
                let a_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let b_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
                match (a_dir, b_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.file_name().cmp(&b.file_name()),
                }
            });
            for entry in &items {
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                // Skip hidden entries (starting with ".").
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                entries.push(FileBrowserEntry {
                    path: entry.path(),
                    name,
                    is_dir,
                    depth: 0,
                    is_expanded: false,
                    is_selected: false,
                });
            }
        }

        Self {
            root,
            entries,
            selected: None,
        }
    }

    /// Toggle expansion of a directory entry.  Returns `true` if the
    /// tree changed and the caller should re-render.
    pub fn toggle_expand(&mut self, path: &std::path::Path) -> bool {
        let Some(idx) = self.entries.iter().position(|e| e.path == path && e.is_dir) else {
            return false;
        };

        if self.entries[idx].is_expanded {
            // Collapse: remove all children deeper than this entry.
            let depth = self.entries[idx].depth;
            let mut remove_end = idx + 1;
            while remove_end < self.entries.len() && self.entries[remove_end].depth > depth {
                remove_end += 1;
            }
            self.entries.drain(idx + 1..remove_end);
            self.entries[idx].is_expanded = false;
        } else {
            // Expand: insert children immediately after.
            let path_owned: PathBuf = path.to_path_buf();
            let depth = self.entries[idx].depth + 1;
            let mut children = Vec::new();
            if let Ok(dir) = std::fs::read_dir(&path_owned) {
                let mut items: Vec<_> = dir.filter_map(|e| e.ok()).collect();
                items.sort_by(|a, b| {
                    let a_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let b_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    match (a_dir, b_dir) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a.file_name().cmp(&b.file_name()),
                    }
                });
                for e in &items {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    children.push(FileBrowserEntry {
                        path: e.path(),
                        name,
                        is_dir,
                        depth,
                        is_expanded: false,
                        is_selected: false,
                    });
                }
            }
            let insert_at = idx + 1;
            for (i, child) in children.into_iter().enumerate() {
                self.entries.insert(insert_at + i, child);
            }
            self.entries[idx].is_expanded = true;
        }
        true
    }

    /// Set the selected entry by path.
    pub fn select(&mut self, path: &std::path::Path) {
        for entry in &mut self.entries {
            entry.is_selected = entry.path == path;
        }
        self.selected = Some(path.to_path_buf());
    }
}

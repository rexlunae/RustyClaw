//! Shared plumbing for the working-directory fields.
//!
//! Every dialog that asks for a directory offers the same three things: a
//! native folder picker, a starting point that is the user's home directory
//! rather than wherever the binary happened to be launched from, and an
//! example path that matches the platform. Keeping them here means a project
//! and a thread cannot drift apart on any of the three — and, in particular,
//! that no dialog suggests a Linux-shaped `/home/you/...` path to a macOS
//! user, where `/home` is owned by the automounter and cannot be written to.

use std::path::PathBuf;

/// The user's home directory.
///
/// Read from the environment rather than `dirs` so this agrees with the
/// sidebar's `~` collapsing, which reads the same variables.
pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// The home directory as text, for seeding a path field. Empty when there is
/// no home directory to speak of, which the dialogs treat as "no default".
pub(crate) fn home_path_text() -> String {
    home_dir()
        .map(|h| h.display().to_string())
        .unwrap_or_default()
}

/// An example directory for a placeholder, rooted in the real home directory
/// so it is a path the user could actually use.
pub(crate) fn example_path(leaf: &str) -> String {
    match home_dir() {
        Some(home) => home.join("code").join(leaf).display().to_string(),
        // No home directory: stay generic rather than invent a root.
        None => format!("path/to/{leaf}"),
    }
}

/// Open a native folder picker starting at `start`, returning the choice.
///
/// Falls back to the home directory when the field is empty, and to the
/// current directory only when there is no home — starting the picker at the
/// process's cwd means it opens in `/` for a bundled app launch.
pub(crate) async fn pick_folder(start: String) -> Option<String> {
    let start = if start.trim().is_empty() {
        home_path_text()
    } else {
        start
    };
    let mut dialog = rfd::AsyncFileDialog::new();
    if !start.is_empty() {
        dialog = dialog.set_directory(start);
    }
    dialog
        .pick_folder()
        .await
        .map(|folder| folder.path().display().to_string())
}

/// The last component of a path, as a project name.
///
/// Picking `~/code/money-tracker` in the folder dialog should not then require
/// typing "money-tracker" into the name field as well.
pub(crate) fn name_from_path(path: &str) -> Option<String> {
    let name = std::path::Path::new(path.trim())
        .file_name()?
        .to_string_lossy()
        .into_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_folder_names_the_project() {
        assert_eq!(
            name_from_path("/Users/tserica/code/money-tracker"),
            Some("money-tracker".to_string())
        );
        assert_eq!(
            name_from_path("  /Users/tserica/Money  "),
            Some("Money".to_string())
        );
        // A root has no leaf to borrow, so the name field is left to the user.
        assert_eq!(name_from_path("/"), None);
        assert_eq!(name_from_path(""), None);
    }

    /// The placeholder has to be a path that exists on *this* platform: the
    /// old hard-coded `/home/you/code/my-project` is unusable on macOS.
    #[test]
    fn the_example_path_is_rooted_in_the_home_directory() {
        let example = example_path("my-project");
        if let Some(home) = home_dir() {
            assert!(
                example.starts_with(&home.display().to_string()),
                "{example} should sit under {}",
                home.display()
            );
        }
        assert!(example.ends_with("my-project"));
    }
}

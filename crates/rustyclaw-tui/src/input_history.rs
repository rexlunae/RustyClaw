//! Persistent prompt input history (bash-style up/down recall).
//!
//! One entry per line, oldest first, capped at [`MAX_ENTRIES`]. The file
//! lives in the profile's settings directory so separate `--profile` runs
//! keep separate histories.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Maximum entries kept on disk and in memory.
pub const MAX_ENTRIES: usize = 1000;

static HISTORY_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Record where history is stored. First call wins; later calls are no-ops
/// (the TUI root can be re-rendered, but the profile never changes mid-run).
pub fn init(path: PathBuf) {
    let _unused = HISTORY_PATH.set(path);
}

/// Load history from disk, oldest first. Missing or unreadable files are an
/// empty history, never an error — recall is best-effort.
pub fn load() -> Vec<String> {
    let Some(path) = HISTORY_PATH.get() else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut entries: Vec<String> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    entries
}

/// Append `entry` to `entries` (skipping consecutive duplicates), trim to
/// [`MAX_ENTRIES`], and write the result to disk. Multi-line submissions are
/// stored with newlines flattened to spaces so the line-per-entry file stays
/// parseable.
pub fn push_and_save(entries: &mut Vec<String>, entry: &str) {
    let flattened = entry.replace(['\r', '\n'], " ");
    let trimmed = flattened.trim();
    if trimmed.is_empty() {
        return;
    }
    if entries.last().map(String::as_str) == Some(trimmed) {
        return;
    }
    entries.push(trimmed.to_string());
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    let Some(path) = HISTORY_PATH.get() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _unused = fs::create_dir_all(parent);
    }
    let _unused = fs::write(path, entries.join("\n") + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_skips_empty_and_consecutive_duplicates() {
        let mut entries = vec!["a".to_string()];
        push_and_save(&mut entries, "  ");
        push_and_save(&mut entries, "a");
        push_and_save(&mut entries, "b");
        push_and_save(&mut entries, "a");
        assert_eq!(entries, vec!["a", "b", "a"]);
    }

    #[test]
    fn push_flattens_newlines() {
        let mut entries = Vec::new();
        push_and_save(&mut entries, "line one\nline two");
        assert_eq!(entries, vec!["line one line two"]);
    }

    #[test]
    fn push_caps_entries() {
        let mut entries: Vec<String> = (0..MAX_ENTRIES).map(|i| i.to_string()).collect();
        push_and_save(&mut entries, "newest");
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries.last().map(String::as_str), Some("newest"));
        assert_eq!(entries.first().map(String::as_str), Some("1"));
    }
}

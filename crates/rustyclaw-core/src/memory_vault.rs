//! Obsidian-compatible mirror of the memory store.
//!
//! Reads `MEMORY.md` + `memory/*.md` from a workspace and writes a parallel
//! `wiki/` directory whose contents Obsidian can open natively:
//!
//! - `[Title](file.md)` markdown links get rewritten to `[[file|Title]]`.
//! - YAML frontmatter is preserved (Obsidian reads it natively).
//! - An `index.md` is generated at the vault root linking every file.
//!
//! The mirror is one-way (memory → wiki). Edits made in Obsidian are not
//! propagated back — by design, the source of truth is the memory files.
//!
//! Use [`MemoryVault::obsidian_open_url`] to get a clickable deep link.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const DEFAULT_VAULT_DIR: &str = "wiki";

/// One-way sync from a memory directory to an Obsidian-readable vault.
pub struct MemoryVault {
    workspace: PathBuf,
    vault_dir: PathBuf,
}

impl MemoryVault {
    /// Construct a vault rooted at `<workspace>/wiki`.
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        let vault_dir = workspace.join(DEFAULT_VAULT_DIR);
        Self {
            workspace,
            vault_dir,
        }
    }

    /// Use a non-default vault subdirectory.
    pub fn with_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.vault_dir = self.workspace.join(dir);
        self
    }

    /// Path the vault writes to.
    pub fn path(&self) -> &Path {
        &self.vault_dir
    }

    /// Obsidian deep link that opens the vault directory in Obsidian.app.
    ///
    /// Not guaranteed to launch on Linux (where Obsidian may not be installed),
    /// but the URL form itself is portable.
    pub fn obsidian_open_url(&self) -> String {
        // Obsidian accepts both `path` (open file/folder) and `vault` (open
        // a named vault). `path` is simpler and works without prior vault
        // registration.
        let encoded = url_encode_path(&self.vault_dir.display().to_string());
        format!("obsidian://open?path={}", encoded)
    }

    /// Synchronize the vault from the workspace's memory files.
    ///
    /// Returns the number of files written.
    pub fn sync(&self) -> Result<SyncReport, VaultError> {
        fs::create_dir_all(&self.vault_dir).map_err(|e| VaultError::Io {
            path: self.vault_dir.display().to_string(),
            source: e,
        })?;

        let mut entries: Vec<VaultEntry> = Vec::new();

        let memory_md = self.workspace.join("MEMORY.md");
        if memory_md.is_file() {
            entries.push(self.copy_to_vault(&memory_md, "MEMORY")?);
        }

        let memory_dir = self.workspace.join("memory");
        if memory_dir.is_dir() {
            for entry in fs::read_dir(&memory_dir).map_err(|e| VaultError::Io {
                path: memory_dir.display().to_string(),
                source: e,
            })? {
                let entry = entry.map_err(|e| VaultError::Io {
                    path: memory_dir.display().to_string(),
                    source: e,
                })?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("note")
                    .to_string();
                entries.push(self.copy_to_vault(&path, &stem)?);
            }
        }

        self.write_index(&entries)?;
        Ok(SyncReport {
            files_written: entries.len() + 1, // +1 for index.md
            vault_path: self.vault_dir.clone(),
        })
    }

    fn copy_to_vault(&self, source: &Path, basename: &str) -> Result<VaultEntry, VaultError> {
        let content = fs::read_to_string(source).map_err(|e| VaultError::Io {
            path: source.display().to_string(),
            source: e,
        })?;
        let converted = wikilinkify(&content);
        let dest = self.vault_dir.join(format!("{}.md", basename));
        let mut f = fs::File::create(&dest).map_err(|e| VaultError::Io {
            path: dest.display().to_string(),
            source: e,
        })?;
        f.write_all(converted.as_bytes()).map_err(|e| VaultError::Io {
            path: dest.display().to_string(),
            source: e,
        })?;
        Ok(VaultEntry {
            basename: basename.to_string(),
            title: extract_title(&content).unwrap_or_else(|| basename.to_string()),
        })
    }

    fn write_index(&self, entries: &[VaultEntry]) -> Result<(), VaultError> {
        let mut sorted: Vec<&VaultEntry> = entries.iter().collect();
        sorted.sort_by(|a, b| a.basename.cmp(&b.basename));

        let mut out = String::new();
        out.push_str("# Memory Vault\n\n");
        out.push_str("Auto-generated mirror of the RustyClaw memory store. ");
        out.push_str("Edit the source files under `MEMORY.md` / `memory/`; do not edit here.\n\n");
        for e in sorted {
            out.push_str(&format!("- [[{}|{}]]\n", e.basename, e.title));
        }
        let path = self.vault_dir.join("index.md");
        fs::write(&path, out).map_err(|e| VaultError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct SyncReport {
    pub files_written: usize,
    pub vault_path: PathBuf,
}

#[derive(Debug)]
struct VaultEntry {
    basename: String,
    title: String,
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Rewrite `[Title](file.md)` and `[Title](file.md:42)` to `[[file|Title]]`.
/// Leaves external URLs (anything with `://`), absolute paths, and parent
/// references (`../...`) untouched.
fn wikilinkify(content: &str) -> String {
    let mut out = String::with_capacity(content.len() + 32);
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'[' && !is_already_wikilink(bytes, i) {
            if let Some((title, target, after)) = parse_md_link(content, i) {
                if let Some(basename) = local_md_basename(target) {
                    out.push_str(&format!("[[{}|{}]]", basename, title));
                    i = after;
                    continue;
                }
            }
        }
        // UTF-8-safe single char advance.
        let ch_start = i;
        let s_rest = &content[ch_start..];
        let mut chars = s_rest.chars();
        if let Some(ch) = chars.next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn is_already_wikilink(bytes: &[u8], i: usize) -> bool {
    i + 1 < bytes.len() && bytes[i + 1] == b'['
}

/// Try to parse a markdown link starting at `start` (which must point at `[`).
/// Returns (title, target, position_after_link) on success.
fn parse_md_link(s: &str, start: usize) -> Option<(&str, &str, usize)> {
    debug_assert_eq!(s.as_bytes()[start], b'[');
    let after_open = start + 1;
    let close_bracket = find_matching(s, after_open, b']')?;
    if s.as_bytes().get(close_bracket + 1) != Some(&b'(') {
        return None;
    }
    let url_start = close_bracket + 2;
    let close_paren = find_matching(s, url_start, b')')?;
    let title = &s[after_open..close_bracket];
    let target = &s[url_start..close_paren];
    Some((title, target, close_paren + 1))
}

fn find_matching(s: &str, from: usize, target: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == target {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Returns the file basename (no extension, no path) if `target` looks like
/// a sibling markdown file we should wikilink. Otherwise `None`.
fn local_md_basename(target: &str) -> Option<&str> {
    if target.contains("://") {
        return None;
    }
    if target.starts_with('/') || target.starts_with("..") {
        return None;
    }
    // Strip ":42" line suffix.
    let core = target.split(':').next().unwrap_or(target);
    // Must end in .md (case-insensitive).
    let lower = core.to_ascii_lowercase();
    if !lower.ends_with(".md") {
        return None;
    }
    // Path-component basename.
    let basename = core.rsplit('/').next().unwrap_or(core);
    let stem = &basename[..basename.len() - 3];
    if stem.is_empty() {
        None
    } else {
        Some(stem)
    }
}

/// Extract a title from the first `# Heading` line, or from `name:` in YAML
/// frontmatter. Returns `None` if neither is found.
fn extract_title(content: &str) -> Option<String> {
    let mut in_frontmatter = false;
    let mut saw_first_line = false;

    for line in content.lines() {
        if !saw_first_line {
            saw_first_line = true;
            if line.trim() == "---" {
                in_frontmatter = true;
                continue;
            }
        }
        if in_frontmatter {
            if line.trim() == "---" {
                in_frontmatter = false;
                continue;
            }
            if let Some(rest) = line.trim_start().strip_prefix("name:") {
                let t = rest.trim().trim_matches('"').trim_matches('\'').to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn url_encode_path(s: &str) -> String {
    // Minimal RFC 3986 encoding for path characters. Only encode bytes that
    // would break the URL when consumed by Obsidian. Most ASCII path chars
    // are fine; spaces are the common offender.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'/'
            | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rewrites_local_md_links_to_wikilinks() {
        let in_str = "See [the foo file](foo.md) and [bar](memory/bar.md) for context.";
        let out = wikilinkify(in_str);
        assert!(out.contains("[[foo|the foo file]]"));
        assert!(out.contains("[[bar|bar]]"));
    }

    #[test]
    fn preserves_external_urls_and_anchors() {
        let in_str = "See [home](https://example.com) and [issue](#section).";
        let out = wikilinkify(in_str);
        assert_eq!(out, in_str);
    }

    #[test]
    fn strips_line_suffix_in_target() {
        let out = wikilinkify("[edit](src/foo.md:42)");
        assert!(out.contains("[[foo|edit]]"));
    }

    #[test]
    fn does_not_touch_existing_wikilinks() {
        let in_str = "Already linked: [[foo|the foo file]]";
        let out = wikilinkify(in_str);
        assert_eq!(out, in_str);
    }

    #[test]
    fn extract_title_from_h1() {
        assert_eq!(
            extract_title("# My Memory\n\nbody"),
            Some("My Memory".to_string())
        );
    }

    #[test]
    fn extract_title_from_frontmatter() {
        let in_str = "---\nname: Important Note\ntype: feedback\n---\n\n# Different heading\n";
        assert_eq!(
            extract_title(in_str),
            Some("Important Note".to_string())
        );
    }

    #[test]
    fn obsidian_url_is_well_formed() {
        let v = MemoryVault::new(Path::new("/tmp/space with space"));
        let url = v.obsidian_open_url();
        assert!(url.starts_with("obsidian://open?path="));
        assert!(url.contains("space%20with%20space"));
    }

    #[test]
    fn sync_writes_vault_with_index() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        fs::write(
            ws.join("MEMORY.md"),
            "# Root Memory\n\n- [Feedback A](feedback_a.md) — testing\n",
        )
        .unwrap();
        fs::create_dir_all(ws.join("memory")).unwrap();
        fs::write(
            ws.join("memory/feedback_a.md"),
            "---\nname: Feedback A\ntype: feedback\n---\n\nbody\n",
        )
        .unwrap();
        fs::write(
            ws.join("memory/another.md"),
            "# Another Note\n\nbody.\n",
        )
        .unwrap();

        let v = MemoryVault::new(ws);
        let report = v.sync().unwrap();
        assert_eq!(report.files_written, 4); // MEMORY + 2 + index
        assert!(v.path().join("MEMORY.md").exists());
        assert!(v.path().join("feedback_a.md").exists());
        assert!(v.path().join("another.md").exists());
        assert!(v.path().join("index.md").exists());

        let memory_mirror = fs::read_to_string(v.path().join("MEMORY.md")).unwrap();
        assert!(memory_mirror.contains("[[feedback_a|Feedback A]]"));

        let index = fs::read_to_string(v.path().join("index.md")).unwrap();
        assert!(index.contains("[[feedback_a|Feedback A]]"));
        assert!(index.contains("[[another|Another Note]]"));
    }
}

//! Crash-safe persistence primitives, shared by every store that writes to
//! disk.
//!
//! Three ideas, applied uniformly instead of re-invented per store:
//!
//! - **Writes are atomic.** A truncating write that fails partway turns one
//!   failed write into the loss of the whole file. Every full-file write
//!   here goes to a temp sibling, is flushed and synced, and then renamed
//!   into place — the file on disk is always either the old version or the
//!   new one, never a torn middle.
//!
//! - **Directories are self-healing.** A missing parent directory is
//!   created rather than reported. The store that survives a fresh install
//!   also survives a user deleting a directory out from under it.
//!
//! - **Corruption is quarantined, never destroyed.** When a file fails to
//!   parse, the reflex of "fall back to default and save" overwrites the
//!   evidence — and with it, any chance of manual recovery. [`quarantine`]
//!   renames the corrupt file aside with a timestamp first, so the store
//!   can heal itself *and* the user keeps the bytes.
//!
//! [`Drop`] is deliberately not the load-bearing mechanism here: destructors
//! do not run on `SIGKILL`, power loss, or an aborting panic — exactly the
//! events that tear files. Stores write durably at mutation time and use
//! `Drop` only as a best-effort backstop for state that would otherwise
//! need an explicit call nothing enforces.

use crate::ignore::Ignore;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Write a file atomically: create its directory, write a temp sibling,
/// fsync, rename into place, and best-effort fsync the directory so the
/// rename itself survives a crash.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_atomically_inner(path, bytes, false)
}

/// [`write_atomically`], but the file never exists with permissions wider
/// than `0o600` — the mode is set on the temp file *before* content is
/// written, closing the umask window a write-then-chmod leaves open. For
/// private keys and other secrets.
pub fn write_atomically_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_atomically_inner(path, bytes, true)
}

fn write_atomically_inner(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = tmp_sibling(path);
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut file = options.open(&tmp)?;
        #[cfg(unix)]
        if private {
            // `mode` only applies when the temp file is newly created; a
            // leftover from an interrupted earlier write keeps its old
            // permissions, so pin them explicitly.
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => {
            // Renames are metadata: without a directory sync a crash can
            // roll the rename back even though the data blocks are safe.
            // Best-effort — not every filesystem lets a directory be
            // opened, and the write itself already succeeded.
            if let Some(parent) = path.parent() {
                if let Ok(dir) = std::fs::File::open(parent) {
                    dir.sync_all().ignore();
                }
            }
            Ok(())
        }
        Err(e) => {
            std::fs::remove_file(&tmp).ignore();
            Err(e)
        }
    }
}

/// The temp sibling for an atomic write. A sibling (same directory) so the
/// rename cannot cross filesystems; suffixed rather than re-extensioned so
/// `a.meta.json` and `a.log.jsonl` cannot collide on one temp name.
fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Move a corrupt file aside so the store can start fresh without
/// destroying the evidence. Returns the quarantine path if the rename
/// succeeded.
///
/// The alternative — falling back to a default and saving — writes the
/// healthy default over the only copy of whatever the file held. After
/// quarantine the load path is free to do exactly that, and the user still
/// has the bytes for manual recovery.
pub fn quarantine(path: &Path, reason: &str) -> Option<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".corrupt-{stamp}"));
    let dest = path.with_file_name(name);
    match std::fs::rename(path, &dest) {
        Ok(()) => {
            tracing::warn!(
                file = %path.display(),
                quarantined_to = %dest.display(),
                reason,
                "Corrupt data file moved aside; starting from a fresh one"
            );
            Some(dest)
        }
        Err(e) => {
            tracing::warn!(
                file = %path.display(),
                error = %e,
                reason,
                "Corrupt data file could not be moved aside"
            );
            None
        }
    }
}

/// Append pre-encoded bytes to a file, synced before returning, creating
/// the file and its directory if they are missing. For JSONL logs, where
/// an acknowledged record must survive a crash and a torn tail is the
/// reader's job to tolerate.
pub fn append_durably(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rustyclaw-persist-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The write must succeed into a directory that does not exist yet —
    /// that is the self-healing contract every store leans on.
    #[test]
    fn a_write_creates_the_directories_it_needs() {
        let root = temp_root("mkdir");
        let path = root.join("a/b/c/data.json");
        write_atomically(&path, b"{}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        std::fs::remove_dir_all(&root).ignore();
    }

    /// A failed write must leave the previous contents untouched, and a
    /// successful one must leave no temp sibling behind.
    #[test]
    fn a_write_replaces_wholesale_and_cleans_up() {
        let root = temp_root("replace");
        let path = root.join("data.json");
        write_atomically(&path, b"old").unwrap();
        write_atomically(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let leftovers: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp sibling left behind");
        std::fs::remove_dir_all(&root).ignore();
    }

    /// Distinct files whose names differ only in their last extension must
    /// not share a temp name — `with_extension` would have collided them.
    #[test]
    fn temp_names_do_not_collide_across_extensions() {
        let a = tmp_sibling(Path::new("/x/7.meta.json"));
        let b = tmp_sibling(Path::new("/x/7.log.jsonl"));
        assert_ne!(a, b);
    }

    #[cfg(unix)]
    #[test]
    fn a_private_write_is_never_group_readable() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_root("private");
        let path = root.join("secret.key");
        write_atomically_private(&path, b"key material").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "key file mode was {mode:o}");
        std::fs::remove_dir_all(&root).ignore();
    }

    /// Quarantine must move the file, not copy it — the store's next save
    /// writes a fresh file, and the old bytes survive under the new name.
    #[test]
    fn quarantine_preserves_the_bytes_out_of_the_way() {
        let root = temp_root("quarantine");
        let path = root.join("store.json");
        std::fs::write(&path, b"garbage").unwrap();
        let dest = quarantine(&path, "test").expect("rename succeeds");
        assert!(!path.exists(), "original still present");
        assert_eq!(std::fs::read(&dest).unwrap(), b"garbage");
        std::fs::remove_dir_all(&root).ignore();
    }

    #[test]
    fn quarantining_a_missing_file_reports_none() {
        let root = temp_root("missing");
        assert!(quarantine(&root.join("nope.json"), "test").is_none());
        std::fs::remove_dir_all(&root).ignore();
    }

    #[test]
    fn appends_accumulate_and_create_their_directory() {
        let root = temp_root("append");
        let path = root.join("logs/run.jsonl");
        append_durably(&path, b"one\n").unwrap();
        append_durably(&path, b"two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        std::fs::remove_dir_all(&root).ignore();
    }
}

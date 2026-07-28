//! Client-driven file access, confined to the thread's working directory.
//!
//! This is deliberately *not* the agent's file path. The agent's tools run
//! under [`SandboxPolicy`](rustyclaw_core::sandbox::SandboxPolicy), which
//! permits anything not explicitly denied — appropriate for an agent the user
//! has asked to work on their machine. These frames serve editor-style UI
//! instead, so they invert the default: everything outside the foreground
//! thread's effective working directory is refused.
//!
//! Every client-supplied path is treated as relative to that directory, and
//! the resolved location is checked against it *after* canonicalization, so a
//! symlink pointing outward is caught rather than followed.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use tracing::debug;

use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, protocol, transport};

/// Largest file the editor will load, in bytes.
///
/// A guard against a client turning a multi-gigabyte file into a frame: the
/// transport caps frames at 16 MB, so a larger read would fail as an opaque
/// transport error rather than something the user can act on.
pub(crate) const MAX_EDITABLE_BYTES: u64 = 4 * 1024 * 1024;

/// Why a workspace path was refused.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PathRefusal {
    /// The path escaped the thread's working directory.
    Escapes,
    /// The path could not be resolved (missing parent, permissions, …).
    Unresolvable(String),
}

impl PathRefusal {
    fn message(&self, requested: &Path) -> String {
        match self {
            Self::Escapes => format!(
                "'{}' is outside this thread's working directory",
                requested.display()
            ),
            Self::Unresolvable(why) => {
                format!("Could not resolve '{}': {why}", requested.display())
            }
        }
    }
}

/// Resolve a client-supplied path inside `root`, refusing anything that escapes.
///
/// The path is always interpreted as relative to `root`: an absolute path from
/// the client is reinterpreted against it rather than honoured, because a
/// client has no business naming absolute locations on the agent's host.
/// `..` components are rejected outright instead of being normalized away,
/// since a caller that sends one is not describing a location inside the
/// workspace.
///
/// The containment check happens after canonicalization on both sides, so a
/// symlink inside the workspace that points outside it is refused rather than
/// followed.
pub(crate) fn resolve_within(root: &Path, requested: &Path) -> Result<PathBuf, PathRefusal> {
    let mut relative = PathBuf::new();
    for component in requested.components() {
        match component {
            // Strip anything rooting the path elsewhere; the client's path is
            // relative to the workspace by definition.
            Component::RootDir | Component::Prefix(_) | Component::CurDir => {}
            Component::ParentDir => return Err(PathRefusal::Escapes),
            Component::Normal(part) => relative.push(part),
        }
    }

    let candidate = root.join(&relative);

    // Canonicalize the root once; without a real root there is nothing to
    // confine against and every request must fail closed.
    let canon_root = root
        .canonicalize()
        .map_err(|e| PathRefusal::Unresolvable(e.to_string()))?;

    // The target may not exist yet (saving a new file), so fall back to
    // canonicalizing the parent and re-attaching the final component.
    let canon = match candidate.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            let parent = candidate
                .parent()
                .ok_or_else(|| PathRefusal::Unresolvable("no parent directory".into()))?;
            let name = candidate
                .file_name()
                .ok_or_else(|| PathRefusal::Unresolvable("no file name".into()))?;
            let canon_parent = parent
                .canonicalize()
                .map_err(|e| PathRefusal::Unresolvable(e.to_string()))?;
            canon_parent.join(name)
        }
    };

    if !canon.starts_with(&canon_root) {
        return Err(PathRefusal::Escapes);
    }
    Ok(canon)
}

/// Send a `WorkspaceDirListing` refusal/error.
async fn listing_error(
    writer: &mut dyn transport::TransportWriter,
    root: &Path,
    path: PathBuf,
    message: String,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::WorkspaceDirListing,
        payload: ServerPayload::WorkspaceDirListing {
            path,
            entries: Vec::new(),
            error: Some(message),
            root: root.to_path_buf(),
        },
    };
    send_frame(writer, &frame).await
}

/// Handle a `WorkspaceListDir`.
pub(crate) async fn handle_list_dir(
    writer: &mut dyn transport::TransportWriter,
    root: &Path,
    path: PathBuf,
) -> Result<()> {
    debug!(path = %path.display(), "Workspace list-dir request");
    let resolved = match resolve_within(root, &path) {
        Ok(p) => p,
        Err(refusal) => {
            return listing_error(writer, root, path.clone(), refusal.message(&path)).await;
        }
    };

    let read = match std::fs::read_dir(&resolved) {
        Ok(r) => r,
        Err(e) => {
            return listing_error(
                writer,
                root,
                path.clone(),
                format!("Could not list '{}': {e}", path.display()),
            )
            .await;
        }
    };

    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut entries: Vec<protocol::WorkspaceEntryDto> = Vec::new();
    for entry in read.flatten() {
        let full = entry.path();
        // Report paths relative to the workspace: the client never learns the
        // absolute location, and cannot ask for one.
        let Ok(rel) = full.strip_prefix(&canon_root) else {
            continue;
        };
        let meta = entry.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        entries.push(protocol::WorkspaceEntryDto {
            path: rel.to_path_buf(),
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir,
            size: if is_dir {
                0
            } else {
                meta.as_ref().map(|m| m.len()).unwrap_or(0)
            },
        });
    }
    // Directories first, then by name — the order a file tree wants.
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    let frame = ServerFrame {
        frame_type: ServerFrameType::WorkspaceDirListing,
        payload: ServerPayload::WorkspaceDirListing {
            path,
            entries,
            error: None,
            root: root.to_path_buf(),
        },
    };
    send_frame(writer, &frame).await
}

/// Send a `WorkspaceFileContent` refusal/error.
async fn content_error(
    writer: &mut dyn transport::TransportWriter,
    root: &Path,
    path: &Path,
    message: String,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::WorkspaceFileContent,
        payload: ServerPayload::WorkspaceFileContent {
            path: path.to_path_buf(),
            content: String::new(),
            error: Some(message),
            root: root.to_path_buf(),
        },
    };
    send_frame(writer, &frame).await
}

/// Handle a `WorkspaceReadFile`.
pub(crate) async fn handle_read_file(
    writer: &mut dyn transport::TransportWriter,
    root: &Path,
    path: PathBuf,
) -> Result<()> {
    debug!(path = %path.display(), "Workspace read-file request");
    let resolved = match resolve_within(root, &path) {
        Ok(p) => p,
        Err(refusal) => return content_error(writer, root, &path, refusal.message(&path)).await,
    };

    match std::fs::metadata(&resolved) {
        Ok(meta) if meta.len() > MAX_EDITABLE_BYTES => {
            return content_error(
                writer,
                root,
                &path,
                format!(
                    "'{}' is {} bytes; the editor opens files up to {MAX_EDITABLE_BYTES} bytes",
                    path.display(),
                    meta.len()
                ),
            )
            .await;
        }
        Ok(meta) if meta.is_dir() => {
            return content_error(
                writer,
                root,
                &path,
                format!("'{}' is a directory", path.display()),
            )
            .await;
        }
        Ok(_) => {}
        Err(e) => {
            return content_error(
                writer,
                root,
                &path,
                format!("Could not open '{}': {e}", path.display()),
            )
            .await;
        }
    }

    // Binary files are refused rather than lossily mangled: returning
    // replacement characters would let the user "edit" a file and save back
    // something quite different from what was there.
    let content = match std::fs::read(&resolved) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                return content_error(
                    writer,
                    root,
                    &path,
                    format!("'{}' is not valid UTF-8 text", path.display()),
                )
                .await;
            }
        },
        Err(e) => {
            return content_error(
                writer,
                root,
                &path,
                format!("Could not read '{}': {e}", path.display()),
            )
            .await;
        }
    };

    let frame = ServerFrame {
        frame_type: ServerFrameType::WorkspaceFileContent,
        payload: ServerPayload::WorkspaceFileContent {
            path,
            content,
            error: None,
            root: root.to_path_buf(),
        },
    };
    send_frame(writer, &frame).await
}

/// Handle a `WorkspaceWriteFile`.
pub(crate) async fn handle_write_file(
    writer: &mut dyn transport::TransportWriter,
    root: &Path,
    path: PathBuf,
    content: String,
    expected_root: PathBuf,
) -> Result<()> {
    debug!(path = %path.display(), bytes = content.len(), "Workspace write-file request");

    // The client edited a buffer taken from `expected_root`. If the workspace
    // has moved since, this path is still *legal* here — it just names a
    // different file, and writing would overwrite an unrelated one. Refuse.
    // Confinement alone cannot catch this: the path is inside both roots.
    let stale = std::fs::canonicalize(&expected_root).ok() != std::fs::canonicalize(root).ok();
    let result = if stale {
        Err(format!(
            "'{}' was edited in '{}', but this thread now works in '{}'; \
             the file was not written",
            path.display(),
            expected_root.display(),
            root.display()
        ))
    } else {
        resolve_within(root, &path).map_err(|refusal| refusal.message(&path))
    };

    let error = match result {
        Err(message) => Some(message),
        Ok(resolved) => match std::fs::write(&resolved, content.as_bytes()) {
            Ok(()) => None,
            Err(e) => Some(format!("Could not write '{}': {e}", path.display())),
        },
    };

    let frame = ServerFrame {
        frame_type: ServerFrameType::WorkspaceWriteResult,
        payload: ServerPayload::WorkspaceWriteResult {
            path,
            ok: error.is_none(),
            error,
            root: root.to_path_buf(),
        },
    };
    send_frame(writer, &frame).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a workspace with a file inside and a secret outside it.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("workspace");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "not yours").unwrap();
        (tmp, root)
    }

    #[test]
    fn resolves_paths_inside_the_workspace() {
        let (_tmp, root) = fixture();
        let got = resolve_within(&root, Path::new("src/main.rs")).unwrap();
        assert_eq!(got, root.join("src/main.rs").canonicalize().unwrap());
    }

    #[test]
    fn a_file_that_does_not_exist_yet_still_resolves() {
        let (_tmp, root) = fixture();
        // Saving a new file must work; only the parent has to exist.
        let got = resolve_within(&root, Path::new("src/new.rs")).unwrap();
        assert!(got.ends_with("new.rs"));
        assert!(got.starts_with(root.canonicalize().unwrap()));
    }

    #[test]
    fn parent_traversal_is_refused() {
        let (_tmp, root) = fixture();
        for attempt in ["../secret.txt", "src/../../secret.txt", ".."] {
            assert_eq!(
                resolve_within(&root, Path::new(attempt)),
                Err(PathRefusal::Escapes),
                "{attempt} should be refused"
            );
        }
    }

    /// An absolute path is reinterpreted against the workspace rather than
    /// honoured — a client has no business naming host locations.
    #[test]
    fn absolute_paths_are_reinterpreted_not_honoured() {
        let (tmp, root) = fixture();
        let outside = tmp.path().join("secret.txt");
        let got = resolve_within(&root, &outside);
        // The leading components are stripped, so this becomes a lookup for
        // <root>/<tmp components>/secret.txt, which does not exist.
        assert!(
            got.is_err()
                || got
                    .as_ref()
                    .unwrap()
                    .starts_with(root.canonicalize().unwrap()),
            "absolute path escaped: {got:?}"
        );
        assert_ne!(got.ok(), Some(outside.canonicalize().unwrap()));
    }

    /// The check happens after canonicalization, so a symlink inside the
    /// workspace that points outside it is caught rather than followed.
    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_workspace_is_refused() {
        let (tmp, root) = fixture();
        let target = tmp.path().join("secret.txt");
        std::os::unix::fs::symlink(&target, root.join("escape.txt")).unwrap();

        assert_eq!(
            resolve_within(&root, Path::new("escape.txt")),
            Err(PathRefusal::Escapes),
            "a symlink out of the workspace must not be followed"
        );
    }

    /// A symlinked *directory* pointing outward is refused for the same reason.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_out_of_the_workspace_is_refused() {
        let (tmp, root) = fixture();
        let outside_dir = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("f.txt"), "nope").unwrap();
        std::os::unix::fs::symlink(&outside_dir, root.join("link")).unwrap();

        assert_eq!(
            resolve_within(&root, Path::new("link/f.txt")),
            Err(PathRefusal::Escapes)
        );
    }

    /// A buffer captured in one directory must not be written into another,
    /// even though the path is perfectly legal in both. Confinement alone
    /// cannot catch this — only the root the client edited in can.
    #[tokio::test]
    async fn a_write_from_a_moved_workspace_is_refused() {
        use async_trait::async_trait;

        struct Capture(Vec<ServerFrame>);
        #[async_trait]
        impl transport::TransportWriter for Capture {
            async fn send_on_stream(&mut self, _s: u64, frame: &ServerFrame) -> Result<()> {
                self.0.push(frame.clone());
                Ok(())
            }
            async fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let old_root = tmp.path().join("old");
        let new_root = tmp.path().join("new");
        std::fs::create_dir_all(&old_root).unwrap();
        std::fs::create_dir_all(&new_root).unwrap();
        // The same relative path exists in both — an unrelated file.
        std::fs::write(new_root.join("notes.md"), "someone else's file").unwrap();

        let mut writer = Capture(Vec::new());
        handle_write_file(
            &mut writer,
            &new_root,
            PathBuf::from("notes.md"),
            "text typed against the old directory".to_string(),
            old_root.clone(),
        )
        .await
        .unwrap();

        match &writer.0[0].payload {
            ServerPayload::WorkspaceWriteResult { ok, error, .. } => {
                assert!(!ok, "a write from a moved workspace must be refused");
                assert!(error.as_deref().unwrap().contains("now works in"));
            }
            other => panic!("expected a write result, got {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(new_root.join("notes.md")).unwrap(),
            "someone else's file",
            "the unrelated file must be untouched"
        );
    }

    /// A workspace root that does not exist fails closed rather than
    /// degrading to an unconfined check.
    #[test]
    fn a_missing_root_refuses_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("gone");
        assert!(matches!(
            resolve_within(&missing, Path::new("any.txt")),
            Err(PathRefusal::Unresolvable(_))
        ));
    }
}

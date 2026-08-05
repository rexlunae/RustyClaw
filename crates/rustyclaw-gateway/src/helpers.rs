use std::io;
use std::path::{Path, PathBuf};

use tracing::debug;

use rustyclaw_core::gateway::ChatMessage;

// ── Context window helpers ──────────────────────────────────────────────────

/// Return the context-window size (in tokens) for a given model name.
/// Conservative defaults — these are *input* token limits.
pub fn context_window_for_model(model: &str) -> usize {
    let m = model.to_lowercase();
    let window =
        if m.contains("claude-opus") || m.contains("claude-sonnet") || m.contains("claude-haiku") {
            200_000
        } else if m.starts_with("gpt-4.1") {
            1_000_000
        } else if m.starts_with("o3") || m.starts_with("o4") {
            200_000
        } else if m.contains("gemini-2.5-pro")
            || m.contains("gemini-2.5-flash")
            || m.contains("gemini-2.0-flash")
        {
            1_000_000
        } else if m.contains("grok-3") {
            131_072
        } else if m.contains("llama") || m.contains("mistral") || m.contains("deepseek") {
            128_000
        } else {
            // Fallback: 128k is a safe default for modern models
            128_000
        };
    debug!(model, window, "Context window for model");
    window
}

/// Fast token estimate: roughly 1 token ≈ 4 characters for English text.
/// This is intentionally conservative (over-estimates) to trigger compaction
/// early rather than hitting the provider's hard limit.
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    let total_chars: usize = messages
        .iter()
        .map(|m| m.role.len() + m.content.len())
        .sum();
    // ~3.5 chars/token for English; we round down to be conservative.
    total_chars / 3
}

// ── Persistence ─────────────────────────────────────────────────────────────

/// Persist thread state, logging rather than discarding a failure.
///
/// Every call site used to be `let _ = thread_mgr.save_to_file(path)`. A
/// failed write there is silent *and* invisible: the in-memory thread keeps
/// its messages for the rest of the session, so nothing looks wrong until the
/// gateway restarts and the thread comes back short — or empty. Losing a
/// conversation deserves a log line at minimum.
///
/// `path` is the legacy `threads.json` location; the per-thread store lives
/// in a `threads/` directory beside it and appends new activity rather than
/// rewriting every conversation. `&mut` because appending drains each
/// thread's pending log records.
pub fn persist_threads(
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    path: &std::path::Path,
) {
    let store = rustyclaw_core::threads::ThreadStore::at_legacy_path(path);
    if let Err(e) = store.persist(thread_mgr) {
        tracing::error!(
            path = %path.display(),
            error = %e,
            "Failed to persist thread history — messages added this session \
             will be missing after a restart"
        );
    }
}

/// Persist threads, recording `foreground` as where this connection is.
///
/// The store keeps one foreground pointer and the connections each keep
/// their own, so the two only agree if somebody says so before the write.
/// Pair them here rather than at each call site: a `persist_threads` that
/// forgets the pointer does not fail, it just writes a stale one, and the
/// user finds out by reopening in the wrong conversation.
///
/// *Quietly* — no `Foregrounded` event. The other windows on this agent
/// have their own foreground and are not to be dragged onto this one; the
/// manager's copy is a note for the next reader, not news for the current
/// ones. Their views are unaffected by the `is_foreground` flags this
/// touches, because each rewrites them from its own pointer before render.
pub async fn persist_threads_focused(
    thread_mgr: &tokio::sync::Mutex<rustyclaw_core::threads::ThreadManager>,
    path: &Path,
    foreground: Option<rustyclaw_core::threads::ThreadId>,
) {
    let mut tm = thread_mgr.lock().await;
    tm.set_foreground_quietly(foreground);
    persist_threads(&mut tm, path);
}

/// Persist project state, logging rather than discarding a failure.
pub fn persist_projects(
    project_mgr: &rustyclaw_core::projects::ProjectManager,
    path: &std::path::Path,
) {
    if let Err(e) = project_mgr.save_to_file(path) {
        tracing::error!(
            path = %path.display(),
            error = %e,
            "Failed to persist projects"
        );
    }
}

/// Persist the gateway config, logging rather than discarding a failure.
///
/// A silently dropped config save means a setting the user just changed
/// (model, working directory, engine config) quietly reverts on restart.
pub fn persist_config(config: &rustyclaw_core::config::Config) {
    if let Err(e) = config.save(None) {
        tracing::error!(error = %e, "Failed to persist config — recent settings changes will be lost on restart");
    }
}

/// Reject a path that isn't valid UTF-8, with a message fit to show the user.
///
/// Both the JSON on disk and the bincode on the wire encode a path as its
/// UTF-8 string and error otherwise, so such a path could never round-trip
/// anyway — it would surface later as an opaque codec failure with nothing
/// identifying which path caused it. Checking here means the edit that
/// introduced it is what gets blamed.
pub fn reject_non_utf8_path(path: &std::path::Path) -> Result<(), String> {
    if path.to_str().is_some() {
        return Ok(());
    }
    Err(format!(
        "'{}' is not valid UTF-8; RustyClaw cannot store or transmit that path",
        path.display()
    ))
}

// ── Working directories ─────────────────────────────────────────────────────

/// Resolve a client-supplied working directory and make sure it exists,
/// returning the path to store or a message fit to show the user.
///
/// Every place a client names a directory — a project's, a thread's override —
/// goes through here, so `~` means the same thing everywhere and a failure
/// explains itself the same way. `purpose` names the field in the error
/// ("project directory", "thread's working directory").
///
/// The *expanded* path is what comes back: storing the raw `~/code/app` would
/// leave a literal `~` directory to be created again on the next restart.
pub fn prepare_workspace_dir(path: &Path, purpose: &str) -> Result<PathBuf, String> {
    reject_non_utf8_path(path)?;
    let expanded = expand_home(path);
    match std::fs::create_dir_all(&expanded) {
        Ok(()) => Ok(expanded),
        Err(e) => Err(describe_create_failure(&expanded, purpose, &e)),
    }
}

/// Expand a leading `~` against this machine's home directory.
///
/// `~` is shell syntax, not filesystem syntax: `create_dir_all` would happily
/// make a directory *named* `~` next to the gateway's working directory, and
/// the project would then point somewhere the user has never heard of. Shares
/// the tools' expansion so `~/code/app` means one thing across the gateway.
fn expand_home(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(text) => rustyclaw_core::tools::expand_tilde(text),
        // Not UTF-8, so there is no `~` we could have recognised anyway.
        None => path.to_path_buf(),
    }
}

/// Explain a failed directory creation in terms the user can act on.
///
/// The bare `io::Error` is often actively misleading: creating a directory
/// under `/home` on macOS fails with `Operation not supported (os error 45)`,
/// which reads like a bug in RustyClaw rather than "that path can't exist on
/// this OS". Where we recognise the situation we say what to do instead.
fn describe_create_failure(path: &Path, purpose: &str, error: &io::Error) -> String {
    let mut message = format!(
        "Could not create the {purpose} '{}': {error}",
        path.display()
    );
    if let Some(hint) = create_failure_hint(path, error) {
        message.push_str(". ");
        message.push_str(&hint);
    }
    message
}

fn create_failure_hint(path: &Path, error: &io::Error) -> Option<String> {
    if cfg!(target_os = "macos") {
        if let Some(suggestion) = macos_home_suggestion(path, dirs::home_dir().as_deref()) {
            return Some(format!(
                "On macOS '/home' belongs to the system automounter and cannot hold new \
                 directories — home directories live under '/Users'. Try '{}'",
                suggestion.display()
            ));
        }
    }
    if path.is_relative() {
        let base = std::env::current_dir().ok()?;
        return Some(format!(
            "That path is relative, so it was resolved against the gateway's working \
             directory '{}'. Use an absolute path",
            base.display()
        ));
    }
    let blocker = deepest_existing_ancestor(path)?;
    match error.kind() {
        io::ErrorKind::PermissionDenied => Some(format!(
            "'{}' is not writable by the user the gateway runs as",
            blocker.display()
        )),
        io::ErrorKind::NotADirectory => Some(format!(
            "'{}' is a file, not a directory",
            blocker.display()
        )),
        _ => Some(format!(
            "The closest directory that does exist is '{}'",
            blocker.display()
        )),
    }
}

/// The `/Users` path a macOS user probably meant by a `/home/...` one.
///
/// `/home/tserica/Money` → `<home>/Money`: the first component after `/home`
/// is a username, which on macOS is already part of the home directory.
/// `None` when the path isn't under `/home` or we have no home directory to
/// suggest, so the caller falls through to a generic hint.
fn macos_home_suggestion(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let home = home?;
    let under_home = path.strip_prefix("/home").ok()?;
    let mut components = under_home.components();
    // Drop the username component; whatever follows is what the user named.
    components.next()?;
    let tail = components.as_path();
    Some(if tail.as_os_str().is_empty() {
        home.to_path_buf()
    } else {
        home.join(tail)
    })
}

/// The nearest ancestor of `path` that exists, which is the directory that
/// actually refused the write.
fn deepest_existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors()
        .skip(1)
        .find(|a| !a.as_os_str().is_empty() && a.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client that sends `~/code/app` means the home directory, not a
    /// directory named `~` beside the gateway's cwd.
    #[test]
    fn tilde_expands_to_the_home_directory() {
        let home = dirs::home_dir().expect("test host has a home directory");
        assert_eq!(expand_home(Path::new("~")), home);
        assert_eq!(expand_home(Path::new("~/code/app")), home.join("code/app"));
        // Not a tilde path, or another user's home: left exactly as sent.
        assert_eq!(expand_home(Path::new("/srv/api")), Path::new("/srv/api"));
        assert_eq!(expand_home(Path::new("~root/x")), Path::new("~root/x"));
    }

    /// The report this came from: `/home/tserica/Money` on macOS fails with
    /// `os error 45`, which says nothing about the actual problem. The hint has
    /// to name the path that would have worked.
    #[test]
    fn a_home_path_is_translated_to_users() {
        let home = Path::new("/Users/tserica");
        assert_eq!(
            macos_home_suggestion(Path::new("/home/tserica/Money"), Some(home)),
            Some(PathBuf::from("/Users/tserica/Money"))
        );
        assert_eq!(
            macos_home_suggestion(Path::new("/home/tserica"), Some(home)),
            Some(home.to_path_buf())
        );
        // Nothing to suggest for a path that isn't under /home.
        assert_eq!(
            macos_home_suggestion(Path::new("/srv/api"), Some(home)),
            None
        );
        assert_eq!(macos_home_suggestion(Path::new("/home/x"), None), None);
    }

    /// A path inside an existing directory is created, and the stored path is
    /// the expanded one.
    #[test]
    fn preparing_a_directory_creates_it() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nested/project");
        let stored = prepare_workspace_dir(&target, "project directory").unwrap();
        assert_eq!(stored, target);
        assert!(target.is_dir());
    }

    /// A file where a directory should go is reported as such, naming the file
    /// rather than leaving the user with a bare OS error.
    #[test]
    fn a_file_in_the_way_is_named() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("notadir");
        std::fs::write(&file, b"x").unwrap();
        let message = prepare_workspace_dir(&file.join("project"), "project directory")
            .expect_err("a file cannot be a parent directory");
        assert!(
            message.contains(&file.display().to_string()),
            "message should name the offending path: {message}"
        );
    }
}

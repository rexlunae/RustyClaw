//! Hand a URL to the operating system's default browser.
//!
//! Used by clients that render agent output: a link in a webview must not
//! navigate the webview itself (that would replace the app UI), and a link in
//! a terminal has nowhere to go at all. Both hand the URL here instead.
//!
//! The URL is scheme-checked before it reaches the shell. Agent output is not
//! trusted input — it is shaped by tool results, fetched pages, and anything
//! a model was told to say — so `file://`, `javascript:` and friends must not
//! be handed to a platform opener that would happily act on them.

/// Schemes that may be opened externally.
const OPENABLE_SCHEMES: &[&str] = &["http://", "https://", "mailto:"];

/// Why a URL was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The scheme is not on the allowlist.
    UnsupportedScheme(String),
    /// Launching the platform opener failed.
    Launch(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScheme(url) => {
                write!(f, "refusing to open URL with unsupported scheme: {url}")
            }
            Self::Launch(msg) => write!(f, "failed to launch browser: {msg}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Whether `url` is safe to hand to the platform opener.
///
/// Requires an explicit allowlisted scheme — unlike in-page links, there is
/// no base URL here, so a relative path is meaningless and a bare string
/// could be interpreted as a local file by some openers.
pub fn is_openable(url: &str) -> bool {
    let trimmed = url.trim();
    // Interior whitespace (including control characters and newlines) could
    // split an argument for a platform opener, and smuggle a scheme past the
    // prefix check. A real URL percent-encodes spaces, so rejecting them
    // costs nothing.
    if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let lowered = trimmed.to_ascii_lowercase();
    OPENABLE_SCHEMES.iter().any(|s| lowered.starts_with(s))
}

/// Open `url` in the user's default browser.
///
/// Returns [`OpenError::UnsupportedScheme`] without spawning anything when
/// the URL is not openable.
pub fn open_external(url: &str) -> Result<(), OpenError> {
    if !is_openable(url) {
        return Err(OpenError::UnsupportedScheme(url.to_string()));
    }
    launch(url.trim())
}

/// Spawn a platform opener with the URL as its own argument, so nothing can
/// reinterpret the URL's contents as syntax.
///
/// The opener hands off to the browser and exits within milliseconds, but a
/// `Child` that is merely dropped is never waited on, so it lingers as a
/// zombie until the parent exits. The parent here is a long-running desktop
/// app, so a detached thread reaps each child. The thread costs nothing: it
/// blocks in `wait` and ends as soon as the opener does.
#[cfg(not(target_os = "windows"))]
fn spawn_opener(program: &str, url: &str) -> Result<(), OpenError> {
    use std::process::{Command, Stdio};

    let child = Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| OpenError::Launch(format!("{program}: {e}")))?;

    let program = program.to_string();
    std::thread::Builder::new()
        .name("url-opener-reaper".into())
        .spawn(move || {
            let mut child = child;
            match child.wait() {
                Ok(status) if !status.success() => {
                    tracing::debug!(%program, ?status, "URL opener exited non-zero");
                }
                Err(e) => tracing::debug!(%program, error = %e, "could not reap URL opener"),
                _ => {}
            }
        })
        // Failing to spawn the reaper is not worth failing the open over;
        // the link did launch. Worst case is one lingering zombie.
        .map_err(|e| tracing::debug!(error = %e, "could not spawn reaper thread"))
        .ok();

    Ok(())
}

#[cfg(target_os = "macos")]
fn launch(url: &str) -> Result<(), OpenError> {
    spawn_opener("open", url)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn launch(url: &str) -> Result<(), OpenError> {
    spawn_opener("xdg-open", url)
}

/// Windows: call `ShellExecuteW` directly.
///
/// Deliberately not `cmd /C start`. `start` is a cmd builtin, so cmd
/// re-parses the URL and applies its own metacharacter rules — `&`, `|`, `^`
/// and `%VAR%` are interpreted even when the URL is passed as a separate
/// argument. That breaks ordinary URLs with query strings ("?a=1&b=2") and,
/// because these URLs come from agent output, would let a trailing `&calc`
/// start a second command. `ShellExecuteW` takes the URL as a string
/// parameter, so no command line is constructed and no shell is involved.
#[cfg(target_os = "windows")]
fn launch(url: &str) -> Result<(), OpenError> {
    use std::ffi::OsStr;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// NUL-terminated UTF-16, as the W APIs expect.
    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(once(0)).collect()
    }

    let operation = wide("open");
    let file = wide(url);

    // SAFETY: `operation` and `file` are NUL-terminated wide strings that
    // outlive the call. The remaining pointers are null, which ShellExecuteW
    // documents as "use the default" for parameters, directory, and owner
    // window. The call has no other preconditions.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns a value greater than 32 on success; anything at
    // or below that is an error code rather than a real instance handle.
    let code = result as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(OpenError::Launch(format!(
            "ShellExecuteW failed with code {code}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_web_and_mail_schemes() {
        assert!(is_openable("https://example.com"));
        assert!(is_openable("http://example.com/a?b=c#d"));
        assert!(is_openable("mailto:someone@example.com"));
        assert!(is_openable("  https://example.com  "), "surrounding space");
        assert!(
            is_openable("HTTPS://EXAMPLE.COM"),
            "scheme is case-insensitive"
        );
    }

    #[test]
    fn rejects_everything_else() {
        for url in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "vbscript:msgbox",
            "/etc/passwd",
            "example.com",
            "",
            "   ",
        ] {
            assert!(!is_openable(url), "must refuse {url:?}");
        }
    }

    #[test]
    fn rejects_control_characters_and_interior_whitespace() {
        // A newline could split the argument for a naive opener, and
        // "java\nscript:" defeats a plain prefix check.
        assert!(!is_openable("java\nscript:alert(1)"));
        assert!(!is_openable("https://example.com\nrm -rf /"));
        assert!(!is_openable("https://example.com\0"));
        assert!(!is_openable("https://example.com/a b"), "interior space");
        assert!(!is_openable("https://example.com/a\tb"), "interior tab");
    }

    /// Query strings are the common case and must survive intact — the
    /// previous `cmd /C start` path let cmd treat `&` as a command separator.
    #[test]
    fn accepts_query_strings_with_shell_metacharacters() {
        for url in [
            "https://example.com/?a=1&b=2",
            "https://example.com/?q=a|b",
            "https://example.com/?pct=%20value",
            "https://example.com/?caret=a^b",
        ] {
            assert!(is_openable(url), "must accept {url:?}");
        }
    }

    #[test]
    fn open_external_refuses_without_spawning() {
        let err = open_external("javascript:alert(1)").unwrap_err();
        assert_eq!(
            err,
            OpenError::UnsupportedScheme("javascript:alert(1)".into())
        );
    }
}

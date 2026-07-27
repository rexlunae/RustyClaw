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

use std::io;
use std::process::{Command, Stdio};

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
/// Returns [`OpenError::UnsupportedScheme`] without touching the shell when
/// the URL is not openable.
pub fn open_external(url: &str) -> Result<(), OpenError> {
    if !is_openable(url) {
        return Err(OpenError::UnsupportedScheme(url.to_string()));
    }
    let url = url.trim();

    // The URL is always passed as a separate argument, never interpolated
    // into a shell string, so its contents cannot be interpreted as syntax.
    let result: io::Result<_> = {
        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg(url)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        }
        #[cfg(target_os = "windows")]
        {
            // Deliberately NOT `cmd /C start`. `start` is a cmd builtin, so
            // cmd re-parses the URL and applies its own metacharacter rules:
            // `&`, `|`, `^` and `%VAR%` are interpreted even when the URL is
            // passed as a separate argument. That breaks ordinary URLs with
            // query strings ("?a=1&b=2") and, because these URLs come from
            // agent output, lets `…&calc` start a second command.
            //
            // `rundll32` is an ordinary executable launched through
            // CreateProcess, so no shell parses the URL at any point.
            Command::new("rundll32.exe")
                .arg("url.dll,FileProtocolHandler")
                .arg(url)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Command::new("xdg-open")
                .arg(url)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        }
    };

    result
        .map(|_| ())
        .map_err(|e| OpenError::Launch(e.to_string()))
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

//! Where the gateway's diagnostics go.
//!
//! `tracing` is a no-op until a subscriber is installed, so this must run
//! before anything else in the daemon — every `error!`, `warn!` and `info!`
//! emitted before it is discarded with no trace that it happened.
//!
//! Destination, in order of precedence:
//!
//! 1. `RUSTYCLAW_LOG_FILE=<path>` — an explicit file, for service managers
//!    that do not capture the process's streams.
//! 2. `--ssh-stdio` — `<settings_dir>/logs/gateway.log`. See [`log_file`].
//! 3. Otherwise stderr, which a foreground run shows on the terminal and
//!    `rustyclaw gateway start` has already redirected into
//!    `<settings_dir>/logs/gateway.log`.
//!
//! Verbosity comes from `RUSTYCLAW_LOG` or `RUST_LOG` (the daemon spawner
//! sets both from `--log-level`), falling back to `rustyclaw=info,warn`.

use std::path::{Path, PathBuf};

use rustyclaw_core::config::Config;
use rustyclaw_core::daemon;
use rustyclaw_core::logging::LogConfig;

/// Environment variable naming an explicit log file for the gateway.
const LOG_FILE_ENV: &str = "RUSTYCLAW_LOG_FILE";

/// Install the gateway's tracing subscriber.
///
/// Call once, as early in `main` as the config and the mode are known.
pub(crate) fn init(config: &Config, protocol_stdio: bool) {
    let env_file = std::env::var_os(LOG_FILE_ENV).map(PathBuf::from);
    let base = LogConfig::daemon();

    match log_file(env_file, &config.settings_dir, protocol_stdio) {
        Some(path) => rustyclaw_core::logging::init(base.to_file(path)),
        None => rustyclaw_core::logging::init(base.to_stderr()),
    }
}

/// The file to log to, or `None` for stderr.
///
/// In `--ssh-stdio` mode stderr is not a terminal but an ssh channel that
/// the client only drains once the session ends. A chatty filter would fill
/// that pipe and block the gateway mid-write — turning "I turned on logging"
/// into a hung connection — so stdio mode logs to a file instead. stdin and
/// stdout are the wire protocol and are never candidates.
fn log_file(
    env_override: Option<PathBuf>,
    settings_dir: &Path,
    protocol_stdio: bool,
) -> Option<PathBuf> {
    if let Some(path) = env_override.filter(|p| !p.as_os_str().is_empty()) {
        return Some(path);
    }
    protocol_stdio.then(|| daemon::log_path(settings_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = "/settings";

    #[test]
    fn a_foreground_run_logs_to_stderr() {
        assert_eq!(log_file(None, Path::new(SETTINGS), false), None);
    }

    /// Logging to stderr under `--ssh-stdio` risks wedging the connection
    /// on a pipe nobody is reading — see [`log_file`].
    #[test]
    fn stdio_mode_logs_to_a_file_rather_than_the_ssh_channel() {
        assert_eq!(
            log_file(None, Path::new(SETTINGS), true),
            Some(daemon::log_path(Path::new(SETTINGS)))
        );
    }

    #[test]
    fn an_explicit_log_file_wins_in_either_mode() {
        let explicit = PathBuf::from("/var/log/rustyclaw.log");
        for stdio in [false, true] {
            assert_eq!(
                log_file(Some(explicit.clone()), Path::new(SETTINGS), stdio),
                Some(explicit.clone())
            );
        }
    }

    /// `RUSTYCLAW_LOG_FILE=` set but empty reads as "unset", not as a
    /// request to log to the current directory under an empty name.
    #[test]
    fn an_empty_log_file_override_is_ignored() {
        assert_eq!(
            log_file(Some(PathBuf::new()), Path::new(SETTINGS), false),
            None
        );
    }
}

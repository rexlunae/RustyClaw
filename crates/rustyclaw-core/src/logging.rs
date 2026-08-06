//! Structured logging configuration for RustyClaw.
//!
//! Uses `tracing` with `tracing-subscriber` for configurable log levels
//! and structured output. Supports JSON output for production environments.
//!
//! ## Environment Variables
//!
//! - `RUSTYCLAW_LOG` or `RUST_LOG`: Set log level (e.g., `debug`, `rustyclaw=debug,hyper=warn`)
//! - `RUSTYCLAW_LOG_FORMAT`: Set output format (`pretty`, `compact`, `json`)
//!
//! Either log variable outranks the filter carried in [`LogConfig`]: an
//! operator who exports `RUST_LOG` expects it to take effect whatever the
//! binary was compiled to prefer, and it is the first thing anyone reaches
//! for when a process is not saying enough.
//!
//! ## Examples
//!
//! ```bash
//! # Debug logging for RustyClaw, warn for everything else
//! RUSTYCLAW_LOG=rustyclaw=debug,warn rustyclaw gateway run
//!
//! # JSON output for production
//! RUSTYCLAW_LOG_FORMAT=json rustyclaw gateway run
//! ```

use crate::ignore::Ignore;
use std::path::{Path, PathBuf};
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan, writer::BoxMakeWriter},
    prelude::*,
};

/// The filter used when neither the environment nor the config supplies a
/// usable one.
///
/// `rustyclaw` matches by target prefix, so it covers every crate in the
/// workspace — `rustyclaw_core`, `rustyclaw_gateway`, and the clients.
const DEFAULT_FILTER: &str = "rustyclaw=info,warn";

/// Log output format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// Human-readable with colors and indentation
    #[default]
    Pretty,
    /// Compact single-line output
    Compact,
    /// JSON output for log aggregation
    Json,
}

impl std::str::FromStr for LogFormat {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "json" => Self::Json,
            "compact" => Self::Compact,
            _ => Self::Pretty,
        })
    }
}

/// Where log lines are written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LogDestination {
    /// Standard output.
    #[default]
    Stdout,
    /// Standard error. The right default for anything long-running: it keeps
    /// diagnostics out of whatever the process writes on stdout.
    Stderr,
    /// Append to a file, creating it (and its parent directory) if needed.
    ///
    /// For processes whose stdout *and* stderr are already spoken for — a TUI
    /// owning the terminal, a gateway serving the wire protocol over stdio.
    File(PathBuf),
}

/// Logging configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Log filter directive (e.g., "debug", "rustyclaw=debug,hyper=warn").
    ///
    /// `RUSTYCLAW_LOG`/`RUST_LOG` override this when either is set to a
    /// directive that parses.
    pub filter: String,
    /// Output format
    pub format: LogFormat,
    /// Where the lines go
    pub destination: LogDestination,
    /// Include span events (enter/exit)
    pub with_spans: bool,
    /// Include file/line in logs
    pub with_file: bool,
    /// Include thread IDs
    pub with_thread_ids: bool,
    /// Include target (module path)
    pub with_target: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: DEFAULT_FILTER.to_string(),
            format: LogFormat::Pretty,
            destination: LogDestination::Stdout,
            with_spans: false,
            with_file: false,
            with_thread_ids: false,
            with_target: true,
        }
    }
}

/// Read `RUSTYCLAW_LOG_FORMAT`, if it is set.
///
/// Returned as an `Option` so a caller with a better default than `Pretty`
/// — a daemon wanting compact single lines — can tell "unset" from "the
/// operator asked for pretty".
fn format_from_env() -> Option<LogFormat> {
    std::env::var("RUSTYCLAW_LOG_FORMAT")
        .ok()
        .map(|s| s.parse::<LogFormat>().unwrap_or_default())
}

/// Read `RUSTYCLAW_LOG`, then `RUST_LOG`.
fn filter_from_env() -> Option<String> {
    std::env::var("RUSTYCLAW_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok()
}

impl LogConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        Self {
            filter: filter_from_env().unwrap_or_else(|| DEFAULT_FILTER.to_string()),
            format: format_from_env().unwrap_or_default(),
            ..Default::default()
        }
    }

    /// Create a debug configuration
    pub fn debug() -> Self {
        Self {
            filter: "rustyclaw=debug,info".to_string(),
            with_file: true,
            ..Default::default()
        }
    }

    /// Create a production configuration with JSON output
    pub fn production() -> Self {
        Self {
            filter: DEFAULT_FILTER.to_string(),
            format: LogFormat::Json,
            with_spans: true,
            with_target: true,
            ..Default::default()
        }
    }

    /// Create a configuration for a long-running daemon.
    ///
    /// Environment-driven like [`from_env`](Self::from_env), but compact
    /// single lines rather than the multi-line pretty format: daemon output
    /// is read with `tail` and `grep`, not by eye.
    pub fn daemon() -> Self {
        Self {
            format: format_from_env().unwrap_or(LogFormat::Compact),
            destination: LogDestination::Stderr,
            ..Self::from_env()
        }
    }

    /// Send output to standard error.
    #[must_use]
    pub fn to_stderr(mut self) -> Self {
        self.destination = LogDestination::Stderr;
        self
    }

    /// Append output to `path`, creating the file and its parent if needed.
    #[must_use]
    pub fn to_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.destination = LogDestination::File(path.into());
        self
    }
}

/// Initialize the global tracing subscriber.
///
/// This should be called once at the start of the program.
/// Subsequent calls will be ignored.
///
/// # Examples
///
/// ```rust,ignore
/// use rustyclaw::logging::{init, LogConfig};
///
/// // Use environment-based configuration
/// init(LogConfig::from_env());
///
/// // Or use explicit configuration
/// init(LogConfig::debug());
/// ```
pub fn init(config: LogConfig) {
    // The one place the environment is consulted, so that everything below
    // works from a settled configuration.
    let config = LogConfig {
        filter: choose_filter(filter_from_env().as_deref(), &config.filter),
        ..config
    };
    let Some(subscriber) = build_subscriber(config) else {
        return;
    };
    tracing::subscriber::set_global_default(subscriber).ignore();
}

/// Pick the filter directives to use: environment first, then the config,
/// then [`DEFAULT_FILTER`].
///
/// A candidate that does not parse is skipped rather than fatal — a typo in
/// `RUST_LOG` should cost the operator their filter, not every log line the
/// process was going to emit.
fn choose_filter(env: Option<&str>, config_filter: &str) -> String {
    [env.unwrap_or_default(), config_filter, DEFAULT_FILTER]
        .into_iter()
        .find(|candidate| !candidate.trim().is_empty() && EnvFilter::try_new(candidate).is_ok())
        .unwrap_or(DEFAULT_FILTER)
        .to_string()
}

/// Build the writer for `destination`, and say whether ANSI colour suits it.
///
/// `None` means the destination could not be opened; the caller installs no
/// subscriber at all rather than falling back to a stream that may be
/// unsafe to write to (see [`LogDestination::File`]).
fn make_writer(destination: &LogDestination) -> Option<(BoxMakeWriter, bool)> {
    use std::io::IsTerminal;

    match destination {
        LogDestination::Stdout => Some((
            BoxMakeWriter::new(std::io::stdout),
            wants_ansi(std::io::stdout().is_terminal()),
        )),
        LogDestination::Stderr => Some((
            BoxMakeWriter::new(std::io::stderr),
            wants_ansi(std::io::stderr().is_terminal()),
        )),
        LogDestination::File(path) => match open_log_file(path) {
            // Colour codes in a file are noise for everything that reads it.
            Ok(file) => Some((BoxMakeWriter::new(std::sync::Mutex::new(file)), false)),
            Err(err) => {
                eprintln!("⚠ Could not open log file {}: {}", path.display(), err);
                None
            }
        },
    }
}

/// Whether to colour a stream that is (or is not) a terminal.
///
/// `tracing-subscriber` colours unconditionally, so a redirected stream
/// gets escape codes it never asked for. `rustyclaw gateway start` sends
/// the daemon's stderr straight to `gateway.log`, and a log file full of
/// `\x1b[2m` is a log file nobody can grep.
fn wants_ansi(is_terminal: bool) -> bool {
    is_terminal && crate::theme::color_enabled()
}

/// Open `path` for appending, creating its parent directory if needed.
fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// Assemble the subscriber described by `config`, whose filter is taken as
/// already settled — see [`init`].
///
/// Boxed because the three formats are three distinct types, and returned
/// rather than installed so tests can drive one through
/// `tracing::subscriber::with_default` without claiming the global slot.
fn build_subscriber(config: LogConfig) -> Option<Box<dyn tracing::Subscriber + Send + Sync>> {
    let env_filter =
        EnvFilter::try_new(&config.filter).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

    let (writer, ansi) = make_writer(&config.destination)?;

    let span_events = if config.with_spans {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };

    // The format selector comes first and the options after it: `pretty()`
    // turns file/line on, so applying it last would override `with_file`.
    macro_rules! subscriber {
        ($format:ident) => {
            Box::new(
                tracing_subscriber::registry().with(env_filter).with(
                    fmt::layer()
                        .$format()
                        .with_writer(writer)
                        .with_ansi(ansi)
                        .with_span_events(span_events)
                        .with_file(config.with_file)
                        .with_line_number(config.with_file)
                        .with_thread_ids(config.with_thread_ids)
                        .with_target(config.with_target),
                ),
            )
        };
    }

    Some(match config.format {
        LogFormat::Json => subscriber!(json),
        LogFormat::Compact => subscriber!(compact),
        LogFormat::Pretty => subscriber!(pretty),
    })
}

/// Initialize logging with environment-based configuration.
///
/// Convenience function that calls `init(LogConfig::from_env())`.
pub fn init_from_env() {
    init(LogConfig::from_env());
}

/// Initialize logging to a file so that TUI output is not corrupted.
///
/// Writes compact log lines to `log_path` (appending). If the file cannot
/// be opened logging is simply not initialised — the alternative, falling
/// back to a stream, would scribble over the drawn interface.
pub fn init_for_tui(log_path: &std::path::Path) {
    init(
        LogConfig {
            format: LogFormat::Compact,
            ..LogConfig::from_env()
        }
        .to_file(log_path),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_format_parsing() {
        assert_eq!("json".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("JSON".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("compact".parse::<LogFormat>().unwrap(), LogFormat::Compact);
        assert_eq!("pretty".parse::<LogFormat>().unwrap(), LogFormat::Pretty);
        assert_eq!("unknown".parse::<LogFormat>().unwrap(), LogFormat::Pretty);
    }

    #[test]
    fn test_config_from_env() {
        // Clear any existing env vars for test isolation
        // SAFETY: This test runs serially and these env vars are only read by LogConfig::from_env
        unsafe {
            std::env::remove_var("RUSTYCLAW_LOG");
            std::env::remove_var("RUST_LOG");
            std::env::remove_var("RUSTYCLAW_LOG_FORMAT");
        }

        let config = LogConfig::from_env();
        assert_eq!(config.filter, "rustyclaw=info,warn");
        assert_eq!(config.format, LogFormat::Pretty);
    }

    #[test]
    fn test_debug_config() {
        let config = LogConfig::debug();
        assert!(config.filter.contains("debug"));
        assert!(config.with_file);
    }

    #[test]
    fn test_production_config() {
        let config = LogConfig::production();
        assert_eq!(config.format, LogFormat::Json);
        assert!(config.with_spans);
    }

    /// `RUST_LOG=…` is the first thing anyone reaches for, so it has to
    /// outrank whatever filter the binary chose for itself.
    #[test]
    fn environment_filter_outranks_the_configured_one() {
        assert_eq!(
            choose_filter(Some("rustyclaw_gateway=trace"), "rustyclaw=info,warn"),
            "rustyclaw_gateway=trace"
        );
    }

    #[test]
    fn unset_or_empty_environment_leaves_the_configured_filter() {
        assert_eq!(choose_filter(None, "rustyclaw=debug"), "rustyclaw=debug");
        assert_eq!(
            choose_filter(Some(""), "rustyclaw=debug"),
            "rustyclaw=debug"
        );
        assert_eq!(
            choose_filter(Some("  "), "rustyclaw=debug"),
            "rustyclaw=debug"
        );
    }

    /// A typo in `RUST_LOG` should cost the operator their filter, not
    /// every log line the process was going to emit.
    #[test]
    fn an_unparseable_filter_falls_through_to_the_next_candidate() {
        assert_eq!(
            choose_filter(Some("rustyclaw=nosuchlevel"), "rustyclaw=debug"),
            "rustyclaw=debug"
        );
        assert_eq!(
            choose_filter(Some("rustyclaw=nosuchlevel"), "also=nonsense"),
            DEFAULT_FILTER
        );
    }

    /// The default filter is written against the `rustyclaw` prefix, and
    /// `EnvFilter` matches targets by prefix — so it has to cover the
    /// per-crate targets the workspace actually emits under.
    #[test]
    fn the_default_filter_covers_every_workspace_crate() {
        let filter = EnvFilter::try_new(DEFAULT_FILTER).expect("default filter must parse");
        let subscriber = tracing_subscriber::registry().with(filter);
        tracing::subscriber::with_default(subscriber, || {
            assert!(
                tracing::event_enabled!(target: "rustyclaw_gateway::listen", tracing::Level::INFO),
                "gateway events must survive the default filter"
            );
            assert!(
                tracing::event_enabled!(target: "rustyclaw_core::gateway", tracing::Level::INFO),
                "core events must survive the default filter"
            );
        });
    }

    /// The regression this module exists for: a configured subscriber has
    /// to actually put lines somewhere. A destination that writes nothing
    /// is what "every log the gateway emits is discarded" looked like.
    #[test]
    fn a_file_destination_receives_the_events_it_admits() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Nested, so the parent-directory creation is covered too.
        let path = dir.path().join("logs").join("gateway.log");

        let config = LogConfig {
            filter: "rustyclaw=info".to_string(),
            format: LogFormat::Compact,
            ..Default::default()
        }
        .to_file(&path);
        let subscriber = build_subscriber(config).expect("subscriber for a writable path");

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "rustyclaw_gateway::listen", "handling client frame");
            tracing::debug!(target: "rustyclaw_gateway::listen", "below the filter");
        });

        let logged = std::fs::read_to_string(&path).expect("log file should exist");
        assert!(
            logged.contains("handling client frame"),
            "admitted event missing from {}: {logged:?}",
            path.display()
        );
        assert!(
            !logged.contains("below the filter"),
            "filtered event should not have been written: {logged:?}"
        );
    }

    /// A file that cannot be opened yields no subscriber rather than a
    /// fallback onto stdout or stderr: the callers that ask for a file do
    /// so because those streams are already spoken for.
    #[test]
    fn an_unopenable_file_yields_no_subscriber() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory is not a file, so opening it for append must fail.
        assert!(build_subscriber(LogConfig::default().to_file(dir.path())).is_none());
    }
}

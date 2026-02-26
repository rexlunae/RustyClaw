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
//! ## Examples
//!
//! ```bash
//! # Debug logging for RustyClaw, warn for everything else
//! RUSTYCLAW_LOG=rustyclaw=debug,warn rustyclaw gateway run
//!
//! # JSON output for production
//! RUSTYCLAW_LOG_FORMAT=json rustyclaw gateway run
//! ```

use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan},
    prelude::*,
};

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

impl LogFormat {
    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => Self::Json,
            "compact" => Self::Compact,
            "pretty" | _ => Self::Pretty,
        }
    }
}

/// Logging configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Log filter directive (e.g., "debug", "rustyclaw=debug,hyper=warn")
    pub filter: String,
    /// Output format
    pub format: LogFormat,
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
            filter: "rustyclaw=info,warn".to_string(),
            format: LogFormat::Pretty,
            with_spans: false,
            with_file: false,
            with_thread_ids: false,
            with_target: true,
        }
    }
}

impl LogConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        let filter = std::env::var("RUSTYCLAW_LOG")
            .or_else(|_| std::env::var("RUST_LOG"))
            .unwrap_or_else(|_| "rustyclaw=info,warn".to_string());

        let format = std::env::var("RUSTYCLAW_LOG_FORMAT")
            .map(|s| LogFormat::from_str(&s))
            .unwrap_or_default();

        Self {
            filter,
            format,
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
            filter: "rustyclaw=info,warn".to_string(),
            format: LogFormat::Json,
            with_spans: true,
            with_target: true,
            ..Default::default()
        }
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
    let env_filter = EnvFilter::try_new(&config.filter)
        .unwrap_or_else(|_| EnvFilter::new("rustyclaw=info,warn"));

    let span_events = if config.with_spans {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };

    match config.format {
        LogFormat::Json => {
            let subscriber = tracing_subscriber::registry().with(env_filter).with(
                fmt::layer()
                    .json()
                    .with_span_events(span_events)
                    .with_file(config.with_file)
                    .with_line_number(config.with_file)
                    .with_thread_ids(config.with_thread_ids)
                    .with_target(config.with_target),
            );
            let _ = tracing::subscriber::set_global_default(subscriber);
        }
        LogFormat::Compact => {
            let subscriber = tracing_subscriber::registry().with(env_filter).with(
                fmt::layer()
                    .compact()
                    .with_span_events(span_events)
                    .with_file(config.with_file)
                    .with_line_number(config.with_file)
                    .with_thread_ids(config.with_thread_ids)
                    .with_target(config.with_target),
            );
            let _ = tracing::subscriber::set_global_default(subscriber);
        }
        LogFormat::Pretty => {
            let subscriber = tracing_subscriber::registry().with(env_filter).with(
                fmt::layer()
                    .pretty()
                    .with_span_events(span_events)
                    .with_file(config.with_file)
                    .with_line_number(config.with_file)
                    .with_thread_ids(config.with_thread_ids)
                    .with_target(config.with_target),
            );
            let _ = tracing::subscriber::set_global_default(subscriber);
        }
    }
}

/// Initialize logging with environment-based configuration.
///
/// Convenience function that calls `init(LogConfig::from_env())`.
pub fn init_from_env() {
    init(LogConfig::from_env());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_format_parsing() {
        assert_eq!(LogFormat::from_str("json"), LogFormat::Json);
        assert_eq!(LogFormat::from_str("JSON"), LogFormat::Json);
        assert_eq!(LogFormat::from_str("compact"), LogFormat::Compact);
        assert_eq!(LogFormat::from_str("pretty"), LogFormat::Pretty);
        assert_eq!(LogFormat::from_str("unknown"), LogFormat::Pretty);
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
}

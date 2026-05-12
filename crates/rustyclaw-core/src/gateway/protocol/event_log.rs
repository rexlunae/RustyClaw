//! Protocol event logger for debugging gateway communication.
//!
//! Records timestamped protocol events (frames sent/received, errors) to a
//! rotating log file. Secrets are never logged — only frame types and sizes.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Maximum log file size before rotation (1 MB).
const MAX_LOG_SIZE: u64 = 1_024 * 1_024;

/// Direction of a protocol frame.
#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Sent,
    Received,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Direction::Sent => write!(f, "TX"),
            Direction::Received => write!(f, "RX"),
        }
    }
}

/// A single protocol event to log.
#[derive(Debug, Clone)]
pub enum ProtocolEvent {
    /// A frame was successfully sent or received.
    Frame {
        direction: Direction,
        frame_type: String,
        stream_id: u64,
        size_bytes: usize,
    },
    /// A frame failed to deserialize.
    DecodeError {
        direction: Direction,
        size_bytes: usize,
        error: String,
    },
    /// A frame failed to serialize.
    EncodeError {
        frame_type: String,
        error: String,
    },
    /// Connection event (connect, disconnect, auth).
    Connection {
        message: String,
    },
    /// Device flow specific event (for debugging the current issue).
    DeviceFlow {
        message: String,
    },
}

impl fmt::Display for ProtocolEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolEvent::Frame {
                direction,
                frame_type,
                stream_id,
                size_bytes,
            } => write!(
                f,
                "{} frame={} stream={} size={}",
                direction, frame_type, stream_id, size_bytes
            ),
            ProtocolEvent::DecodeError {
                direction,
                size_bytes,
                error,
            } => write!(
                f,
                "{} DECODE_ERROR size={} err={}",
                direction, size_bytes, error
            ),
            ProtocolEvent::EncodeError { frame_type, error } => {
                write!(f, "TX ENCODE_ERROR frame={} err={}", frame_type, error)
            }
            ProtocolEvent::Connection { message } => write!(f, "CONN {}", message),
            ProtocolEvent::DeviceFlow { message } => write!(f, "DEVICE_FLOW {}", message),
        }
    }
}

/// Thread-safe protocol event logger.
///
/// Writes events to a file with timestamps. Rotates when the file exceeds
/// `MAX_LOG_SIZE`. The logger is designed to never fail — I/O errors are
/// silently ignored so protocol operations are never blocked by logging.
#[derive(Clone, Debug)]
pub struct ProtocolEventLog {
    inner: Arc<Mutex<LogInner>>,
}

#[derive(Debug)]
struct LogInner {
    file: Option<std::fs::File>,
    path: PathBuf,
}

impl ProtocolEventLog {
    /// Create a new event log writing to the given path.
    ///
    /// Creates parent directories if needed. Returns a no-op logger if the
    /// file cannot be opened (never fails).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let file = Self::open_log_file(&path);
        Self {
            inner: Arc::new(Mutex::new(LogInner { file, path })),
        }
    }

    /// Create a no-op logger that discards all events.
    pub fn noop() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogInner {
                file: None,
                path: PathBuf::new(),
            })),
        }
    }

    /// Log a protocol event.
    pub fn log(&self, event: ProtocolEvent) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };

        // Check rotation
        let needs_rotate = inner
            .file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .is_some_and(|m| m.len() > MAX_LOG_SIZE);

        if needs_rotate {
            Self::rotate(&inner.path);
            inner.file = Self::open_log_file(&inner.path);
        }

        let Some(file) = inner.file.as_mut() else {
            return;
        };

        let _ = Self::write_event(file, &event);
    }

    /// Log a frame event (convenience wrapper).
    pub fn log_frame(
        &self,
        direction: Direction,
        frame_type: &str,
        stream_id: u64,
        size_bytes: usize,
    ) {
        self.log(ProtocolEvent::Frame {
            direction,
            frame_type: frame_type.to_string(),
            stream_id,
            size_bytes,
        });
    }

    /// Log a decode error (convenience wrapper).
    pub fn log_decode_error(&self, direction: Direction, size_bytes: usize, error: &str) {
        self.log(ProtocolEvent::DecodeError {
            direction,
            size_bytes,
            error: error.to_string(),
        });
    }

    /// Log a device flow event (convenience wrapper).
    pub fn log_device_flow(&self, message: &str) {
        self.log(ProtocolEvent::DeviceFlow {
            message: message.to_string(),
        });
    }

    fn write_event(file: &mut std::fs::File, event: &ProtocolEvent) -> std::io::Result<()> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let millis = now.subsec_millis();
        writeln!(file, "{}.{:03} {}", secs, millis, event)?;
        file.flush()
    }

    fn open_log_file(path: &Path) -> Option<std::fs::File> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    }

    fn rotate(path: &Path) {
        let rotated = path.with_extension("log.old");
        let _ = std::fs::rename(path, rotated);
    }
}

/// Determine the default protocol event log path.
///
/// Uses `~/.rustyclaw/protocol_events.log`.
pub fn default_log_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".rustyclaw").join("protocol_events.log"))
}

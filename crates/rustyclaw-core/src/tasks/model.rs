//! Task model — core types for task representation.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// Unique identifier for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub u64);

impl TaskId {
    /// Generate a new unique task ID.
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::SeqCst))
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Task status with progress tracking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task is waiting to start
    Pending,

    /// Task is currently running
    Running {
        /// Optional progress (0.0 - 1.0)
        progress: Option<f32>,
        /// Optional status message
        message: Option<String>,
    },

    /// Task is running but not receiving user attention
    Background {
        progress: Option<f32>,
        message: Option<String>,
    },

    /// Task is paused (can be resumed)
    Paused {
        reason: Option<String>,
    },

    /// Task completed successfully
    Completed {
        /// Summary of what was accomplished
        summary: Option<String>,
        /// Output/result data
        output: Option<String>,
    },

    /// Task failed
    Failed {
        error: String,
        /// Whether the task can be retried
        retryable: bool,
    },

    /// Task was cancelled by user
    Cancelled,

    /// Task is waiting for user input
    WaitingForInput {
        prompt: String,
    },
}

impl TaskStatus {
    /// Check if the task is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled)
    }

    /// Check if the task is running (foreground or background).
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. } | Self::Background { .. })
    }

    /// Check if the task is in the foreground.
    pub fn is_foreground(&self) -> bool {
        matches!(self, Self::Running { .. } | Self::WaitingForInput { .. })
    }

    /// Get progress if available.
    pub fn progress(&self) -> Option<f32> {
        match self {
            Self::Running { progress, .. } | Self::Background { progress, .. } => *progress,
            Self::Completed { .. } => Some(1.0),
            _ => None,
        }
    }

    /// Get status message if available.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Running { message, .. } | Self::Background { message, .. } => message.as_deref(),
            Self::Paused { reason } => reason.as_deref(),
            Self::Completed { summary, .. } => summary.as_deref(),
            Self::Failed { error, .. } => Some(error.as_str()),
            Self::WaitingForInput { prompt } => Some(prompt.as_str()),
            _ => None,
        }
    }
}

/// Kind of task — determines behavior and display.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskKind {
    /// Shell command execution
    Command {
        command: String,
        pid: Option<u32>,
    },

    /// Sub-agent session
    SubAgent {
        session_key: String,
        label: Option<String>,
    },

    /// Cron job execution
    CronJob {
        job_id: String,
        job_name: Option<String>,
    },

    /// MCP tool call
    McpTool {
        server: String,
        tool: String,
    },

    /// Browser automation
    Browser {
        action: String,
        url: Option<String>,
    },

    /// File operation (download, upload, copy)
    FileOp {
        operation: String,
        path: String,
    },

    /// Web request
    WebRequest {
        url: String,
        method: String,
    },

    /// Generic/custom task
    Custom {
        name: String,
        details: Option<String>,
    },
}

impl TaskKind {
    /// Get a short display name for the task kind.
    pub fn display_name(&self) -> &str {
        match self {
            Self::Command { .. } => "Command",
            Self::SubAgent { .. } => "Sub-agent",
            Self::CronJob { .. } => "Cron job",
            Self::McpTool { .. } => "MCP",
            Self::Browser { .. } => "Browser",
            Self::FileOp { .. } => "File",
            Self::WebRequest { .. } => "Web",
            Self::Custom { name, .. } => name.as_str(),
        }
    }

    /// Get a detailed description.
    pub fn description(&self) -> String {
        match self {
            Self::Command { command, pid } => {
                if let Some(p) = pid {
                    format!("{} (pid {})", command, p)
                } else {
                    command.clone()
                }
            }
            Self::SubAgent { label, session_key } => {
                label.clone().unwrap_or_else(|| session_key.clone())
            }
            Self::CronJob { job_name, job_id } => {
                job_name.clone().unwrap_or_else(|| job_id.clone())
            }
            Self::McpTool { server, tool } => format!("{}:{}", server, tool),
            Self::Browser { action, url } => {
                if let Some(u) = url {
                    format!("{} {}", action, u)
                } else {
                    action.clone()
                }
            }
            Self::FileOp { operation, path } => format!("{} {}", operation, path),
            Self::WebRequest { method, url } => format!("{} {}", method, url),
            Self::Custom { name, details } => {
                if let Some(d) = details {
                    format!("{}: {}", name, d)
                } else {
                    name.clone()
                }
            }
        }
    }
}

/// Progress information for a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskProgress {
    /// Progress as fraction (0.0 - 1.0), None if indeterminate
    pub fraction: Option<f32>,

    /// Current step / total steps
    pub steps: Option<(u32, u32)>,

    /// Bytes processed / total bytes
    pub bytes: Option<(u64, u64)>,

    /// Items processed / total items  
    pub items: Option<(u32, u32)>,

    /// ETA in seconds
    pub eta_secs: Option<u64>,

    /// Current status message
    pub message: Option<String>,
}

impl TaskProgress {
    /// Create indeterminate progress.
    pub fn indeterminate() -> Self {
        Self {
            fraction: None,
            steps: None,
            bytes: None,
            items: None,
            eta_secs: None,
            message: None,
        }
    }

    /// Create progress from a fraction.
    pub fn fraction(f: f32) -> Self {
        Self {
            fraction: Some(f.clamp(0.0, 1.0)),
            ..Self::indeterminate()
        }
    }

    /// Create progress from steps.
    pub fn steps(current: u32, total: u32) -> Self {
        let frac = if total > 0 {
            Some(current as f32 / total as f32)
        } else {
            None
        };
        Self {
            fraction: frac,
            steps: Some((current, total)),
            ..Self::indeterminate()
        }
    }

    /// Add a message to the progress.
    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    /// Add ETA to the progress.
    pub fn with_eta(mut self, secs: u64) -> Self {
        self.eta_secs = Some(secs);
        self
    }
}

/// A task with full metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique task ID
    pub id: TaskId,

    /// What kind of task this is
    pub kind: TaskKind,

    /// Current status
    pub status: TaskStatus,

    /// When the task was created
    #[serde(with = "system_time_serde")]
    pub created_at: SystemTime,

    /// When the task started running
    #[serde(with = "option_system_time_serde")]
    pub started_at: Option<SystemTime>,

    /// When the task finished (completed/failed/cancelled)
    #[serde(with = "option_system_time_serde")]
    pub finished_at: Option<SystemTime>,

    /// Session that owns this task
    pub session_key: Option<String>,

    /// User-provided label
    pub label: Option<String>,

    /// Whether task output should stream to chat
    pub stream_output: bool,

    /// Accumulated output (if buffering)
    #[serde(skip)]
    pub output_buffer: String,
}

impl Task {
    /// Create a new pending task.
    pub fn new(kind: TaskKind) -> Self {
        Self {
            id: TaskId::new(),
            kind,
            status: TaskStatus::Pending,
            created_at: SystemTime::now(),
            started_at: None,
            finished_at: None,
            session_key: None,
            label: None,
            stream_output: false,
            output_buffer: String::new(),
        }
    }

    /// Set the session key.
    pub fn with_session(mut self, key: impl Into<String>) -> Self {
        self.session_key = Some(key.into());
        self
    }

    /// Set a label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Enable output streaming.
    pub fn with_streaming(mut self) -> Self {
        self.stream_output = true;
        self
    }

    /// Mark task as running.
    pub fn start(&mut self) {
        self.started_at = Some(SystemTime::now());
        self.status = TaskStatus::Running {
            progress: None,
            message: None,
        };
    }

    /// Move task to background.
    pub fn background(&mut self) {
        if let TaskStatus::Running { progress, message } = &self.status {
            self.status = TaskStatus::Background {
                progress: *progress,
                message: message.clone(),
            };
        }
    }

    /// Move task to foreground.
    pub fn foreground(&mut self) {
        if let TaskStatus::Background { progress, message } = &self.status {
            self.status = TaskStatus::Running {
                progress: *progress,
                message: message.clone(),
            };
        }
    }

    /// Update progress.
    pub fn update_progress(&mut self, progress: TaskProgress) {
        match &mut self.status {
            TaskStatus::Running { progress: p, message: m } |
            TaskStatus::Background { progress: p, message: m } => {
                *p = progress.fraction;
                if progress.message.is_some() {
                    *m = progress.message;
                }
            }
            _ => {}
        }
    }

    /// Mark task as completed.
    pub fn complete(&mut self, summary: Option<String>) {
        self.finished_at = Some(SystemTime::now());
        let output = if self.output_buffer.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.output_buffer))
        };
        self.status = TaskStatus::Completed { summary, output };
    }

    /// Mark task as failed.
    pub fn fail(&mut self, error: impl Into<String>, retryable: bool) {
        self.finished_at = Some(SystemTime::now());
        self.status = TaskStatus::Failed {
            error: error.into(),
            retryable,
        };
    }

    /// Mark task as cancelled.
    pub fn cancel(&mut self) {
        self.finished_at = Some(SystemTime::now());
        self.status = TaskStatus::Cancelled;
    }

    /// Get elapsed time since start.
    pub fn elapsed(&self) -> Option<Duration> {
        self.started_at.map(|start| {
            let end = self.finished_at.unwrap_or_else(SystemTime::now);
            end.duration_since(start).unwrap_or_default()
        })
    }

    /// Get display label (user label or auto-generated).
    pub fn display_label(&self) -> String {
        self.label.clone().unwrap_or_else(|| self.kind.description())
    }
}

// Serde helpers for SystemTime
mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S: Serializer>(time: &SystemTime, ser: S) -> Result<S::Ok, S::Error> {
        let millis = time.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        millis.serialize(ser)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<SystemTime, D::Error> {
        let millis = u64::deserialize(de)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_millis(millis))
    }
}

mod option_system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S: Serializer>(time: &Option<SystemTime>, ser: S) -> Result<S::Ok, S::Error> {
        match time {
            Some(t) => {
                let millis = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                Some(millis).serialize(ser)
            }
            None => None::<u64>.serialize(ser),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Option<SystemTime>, D::Error> {
        let millis: Option<u64> = Option::deserialize(de)?;
        Ok(millis.map(|m| UNIX_EPOCH + std::time::Duration::from_millis(m)))
    }
}

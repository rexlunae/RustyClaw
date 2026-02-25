//! Task display — icons, indicators, and formatting.

use super::model::{Task, TaskId, TaskStatus, TaskKind};

/// Icon representation for task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskIcon {
    /// ⏳ Pending/waiting
    Pending,
    /// 🔄 Running (spinning)
    Running,
    /// ⏸️ Paused
    Paused,
    /// ✅ Completed
    Completed,
    /// ❌ Failed
    Failed,
    /// 🚫 Cancelled
    Cancelled,
    /// 💬 Waiting for input
    WaitingInput,
    /// 📦 Background
    Background,
}

impl TaskIcon {
    /// Get from task status.
    pub fn from_status(status: &TaskStatus) -> Self {
        match status {
            TaskStatus::Pending => Self::Pending,
            TaskStatus::Running { .. } => Self::Running,
            TaskStatus::Background { .. } => Self::Background,
            TaskStatus::Paused { .. } => Self::Paused,
            TaskStatus::Completed { .. } => Self::Completed,
            TaskStatus::Failed { .. } => Self::Failed,
            TaskStatus::Cancelled => Self::Cancelled,
            TaskStatus::WaitingForInput { .. } => Self::WaitingInput,
        }
    }

    /// Get emoji representation.
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Pending => "⏳",
            Self::Running => "🔄",
            Self::Paused => "⏸️",
            Self::Completed => "✅",
            Self::Failed => "❌",
            Self::Cancelled => "🚫",
            Self::WaitingInput => "💬",
            Self::Background => "📦",
        }
    }

    /// Get ANSI color code for terminal.
    pub fn ansi_color(&self) -> &'static str {
        match self {
            Self::Pending => "\x1b[33m",      // Yellow
            Self::Running => "\x1b[36m",      // Cyan
            Self::Paused => "\x1b[33m",       // Yellow
            Self::Completed => "\x1b[32m",    // Green
            Self::Failed => "\x1b[31m",       // Red
            Self::Cancelled => "\x1b[90m",    // Gray
            Self::WaitingInput => "\x1b[35m", // Magenta
            Self::Background => "\x1b[34m",  // Blue
        }
    }

    /// Get unicode spinner frame (for Running state).
    pub fn spinner_frame(frame: usize) -> &'static str {
        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[frame % FRAMES.len()]
    }

    /// Get progress bar character.
    pub fn progress_char(filled: bool) -> &'static str {
        if filled { "█" } else { "░" }
    }
}

impl std::fmt::Display for TaskIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.emoji())
    }
}

/// Indicator style for chat display.
#[derive(Debug, Clone)]
pub struct TaskIndicator {
    /// The icon to show
    pub icon: TaskIcon,
    
    /// Short label
    pub label: String,
    
    /// Optional progress bar (width in chars)
    pub progress_bar: Option<String>,
    
    /// Optional time remaining
    pub eta: Option<String>,
    
    /// Whether this is the foreground task
    pub foreground: bool,
}

impl TaskIndicator {
    /// Create from a task.
    pub fn from_task(task: &Task) -> Self {
        let icon = TaskIcon::from_status(&task.status);
        let label = task.display_label();
        
        let progress_bar = task.status.progress().map(|p| {
            format_progress_bar(p, 10)
        });
        
        let eta = if let TaskStatus::Running { .. } | TaskStatus::Background { .. } = &task.status {
            task.elapsed().map(|d| format_duration(d))
        } else {
            None
        };
        
        Self {
            icon,
            label,
            progress_bar,
            eta,
            foreground: task.status.is_foreground(),
        }
    }

    /// Format as a compact inline indicator for chat.
    pub fn inline(&self) -> String {
        let mut s = format!("{} {}", self.icon, self.label);
        
        if let Some(ref bar) = self.progress_bar {
            s.push_str(&format!(" {}", bar));
        }
        
        if let Some(ref eta) = self.eta {
            s.push_str(&format!(" ({})", eta));
        }
        
        s
    }

    /// Format as a badge for status bar.
    pub fn badge(&self) -> String {
        if let Some(ref bar) = self.progress_bar {
            format!("{}{}", self.icon, bar)
        } else {
            format!("{}", self.icon)
        }
    }
}

/// Format a progress bar.
pub fn format_progress_bar(fraction: f32, width: usize) -> String {
    let filled = (fraction * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);
    
    format!(
        "[{}{}] {}%",
        "█".repeat(filled),
        "░".repeat(empty),
        (fraction * 100.0).round() as u32
    )
}

/// Format a duration for display.
pub fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Format a task status for display.
pub fn format_task_status(task: &Task) -> String {
    let icon = TaskIcon::from_status(&task.status);
    let label = task.display_label();
    
    let status_msg = match &task.status {
        TaskStatus::Pending => "Waiting to start".to_string(),
        TaskStatus::Running { message, progress } => {
            let mut s = "Running".to_string();
            if let Some(p) = progress {
                s.push_str(&format!(" ({}%)", (*p * 100.0).round() as u32));
            }
            if let Some(m) = message {
                s.push_str(&format!(": {}", m));
            }
            s
        }
        TaskStatus::Background { message, progress } => {
            let mut s = "Background".to_string();
            if let Some(p) = progress {
                s.push_str(&format!(" ({}%)", (*p * 100.0).round() as u32));
            }
            if let Some(m) = message {
                s.push_str(&format!(": {}", m));
            }
            s
        }
        TaskStatus::Paused { reason } => {
            if let Some(r) = reason {
                format!("Paused: {}", r)
            } else {
                "Paused".to_string()
            }
        }
        TaskStatus::Completed { summary, .. } => {
            if let Some(s) = summary {
                format!("Completed: {}", s)
            } else {
                "Completed".to_string()
            }
        }
        TaskStatus::Failed { error, retryable } => {
            let retry = if *retryable { " (retryable)" } else { "" };
            format!("Failed{}: {}", retry, error)
        }
        TaskStatus::Cancelled => "Cancelled".to_string(),
        TaskStatus::WaitingForInput { prompt } => {
            format!("Waiting: {}", prompt)
        }
    };
    
    let elapsed = task.elapsed()
        .map(|d| format!(" [{}]", format_duration(d)))
        .unwrap_or_default();
    
    format!("{} {} — {}{}", icon, label, status_msg, elapsed)
}

/// Format a set of tasks as icon bar (for status display).
pub fn format_task_icons(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return String::new();
    }
    
    tasks.iter()
        .map(|t| TaskIcon::from_status(&t.status).emoji())
        .collect::<Vec<_>>()
        .join("")
}

/// Format tasks for chat indicator line.
pub fn format_task_indicators(tasks: &[Task], max_display: usize) -> String {
    if tasks.is_empty() {
        return String::new();
    }
    
    let active: Vec<_> = tasks.iter()
        .filter(|t| !t.status.is_terminal())
        .take(max_display)
        .collect();
    
    if active.is_empty() {
        return String::new();
    }
    
    let indicators: Vec<_> = active.iter()
        .map(|t| TaskIndicator::from_task(t).badge())
        .collect();
    
    let remaining = tasks.iter()
        .filter(|t| !t.status.is_terminal())
        .count()
        .saturating_sub(max_display);
    
    if remaining > 0 {
        format!("{} +{}", indicators.join(" "), remaining)
    } else {
        indicators.join(" ")
    }
}

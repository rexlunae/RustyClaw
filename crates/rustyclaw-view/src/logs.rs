//! Component data for the logs viewer panel.

/// Log source options.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum LogSource {
    #[default]
    Gateway,
    Agent,
    Cron,
    Service(String),
}

impl LogSource {
    pub fn label(&self) -> &str {
        match self {
            Self::Gateway => "Gateway",
            Self::Agent => "Agent",
            Self::Cron => "Cron",
            Self::Service(name) => name.as_str(),
        }
    }

    /// Wire value for the protocol request.
    pub fn wire_value(&self) -> String {
        match self {
            Self::Gateway => "gateway".into(),
            Self::Agent => "agent".into(),
            Self::Cron => "cron".into(),
            Self::Service(name) => name.clone(),
        }
    }
}

/// Full state for the logs viewer panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LogsPanelData {
    pub source: LogSource,
    pub lines: Vec<String>,
    pub following: bool,
    pub scroll_offset: usize,
    pub status: Option<String>,
}

impl LogsPanelData {
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Append new lines (for follow/tail mode).
    pub fn append(&mut self, new_lines: &[String]) {
        self.lines.extend_from_slice(new_lines);
        if self.following {
            self.scroll_to_bottom();
        }
    }

    /// Scroll to show the last page of lines.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.lines.len().saturating_sub(30);
    }

    /// Toggle follow mode.
    pub fn toggle_follow(&mut self) {
        self.following = !self.following;
        if self.following {
            self.scroll_to_bottom();
        }
    }
}

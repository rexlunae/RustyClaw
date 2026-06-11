//! Component data for swarm management UIs.
//!
//! These models are shared by desktop and TUI clients to render the swarm
//! manager consistently.

/// Summary of an agent within a swarm.
#[derive(Clone, Debug, PartialEq)]
pub struct SwarmAgentData {
    pub id: String,
    pub name: String,
    pub role: String,
    pub description: String,
    pub has_session: bool,
}

impl SwarmAgentData {
    /// Icon to display for the role.
    pub fn role_icon(&self) -> &'static str {
        match self.role.as_str() {
            "Orchestrator" => "🎯",
            "Virtual Assistant" => "💼",
            "Deep Research" => "🔬",
            "Data Analyst" => "📊",
            "Slides" => "📽",
            "Docs" => "📄",
            "Image Generation" => "🎨",
            "Video Generation" => "🎬",
            _ => "🤖",
        }
    }
}

/// Summary of a swarm instance.
#[derive(Clone, Debug, PartialEq)]
pub struct SwarmData {
    pub name: String,
    pub status: String,
    pub description: String,
    pub agents: Vec<SwarmAgentData>,
    pub tasks_routed: u64,
    pub uptime_secs: u64,
}

impl SwarmData {
    /// Status class hint for badge/chip rendering.
    pub fn status_class(&self) -> &'static str {
        match self.status.as_str() {
            "Running" => "is-success",
            "Idle" => "is-info",
            "Paused" => "is-warn",
            "Stopped" => "is-muted",
            _ => "is-danger",
        }
    }

    /// Semantic tone for the status badge.
    pub fn status_tone(&self) -> crate::tone::Tone {
        use crate::tone::Tone;
        match self.status.as_str() {
            "Running" => Tone::Success,
            "Idle" => Tone::Info,
            "Paused" => Tone::Warning,
            "Stopped" => Tone::Neutral,
            _ => Tone::Danger,
        }
    }

    /// Whether this swarm should show a stop action.
    pub fn is_stoppable(&self) -> bool {
        matches!(self.status.as_str(), "Running" | "Idle" | "Paused")
    }
}

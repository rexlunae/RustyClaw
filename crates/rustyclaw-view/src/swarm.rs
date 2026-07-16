//! Component data for swarm management UIs.
//!
//! These models are shared by desktop and TUI clients to render the swarm
//! manager consistently. Swarms are persistent groups of registered agents,
//! so status is *derived*: a swarm is "ready" when every member's backing
//! agent exists in the registry and "degraded" when some are missing.

/// Summary of an agent within a swarm.
#[derive(Clone, Debug, PartialEq)]
pub struct SwarmAgentData {
    pub id: String,
    pub name: String,
    pub role: String,
    pub description: String,
    /// Whether the member has an active swarm session.
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

/// Summary of a swarm.
#[derive(Clone, Debug, PartialEq)]
pub struct SwarmData {
    pub name: String,
    /// Derived health: "ready" or "degraded".
    pub status: String,
    pub description: String,
    pub agents: Vec<SwarmAgentData>,
    /// Number of members with an active swarm session.
    pub active_sessions: u64,
    /// Seconds since the swarm was created.
    pub age_secs: u64,
}

impl SwarmData {
    /// Status class hint for badge/chip rendering.
    pub fn status_class(&self) -> &'static str {
        match self.status.as_str() {
            "ready" => "is-success",
            "degraded" => "is-warning",
            _ => "is-muted",
        }
    }

    /// Semantic tone for the status badge.
    pub fn status_tone(&self) -> crate::tone::Tone {
        use crate::tone::Tone;
        match self.status.as_str() {
            "ready" => Tone::Success,
            "degraded" => Tone::Warning,
            _ => Tone::Neutral,
        }
    }

    /// Whether this swarm should show a stop action (it has sessions to
    /// complete; the swarm and its agents always remain until deleted).
    pub fn is_stoppable(&self) -> bool {
        self.active_sessions > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swarm(status: &str, active: u64) -> SwarmData {
        SwarmData {
            name: "s".into(),
            status: status.into(),
            description: String::new(),
            agents: vec![],
            active_sessions: active,
            age_secs: 0,
        }
    }

    #[test]
    fn status_maps_to_class_and_tone() {
        assert_eq!(swarm("ready", 0).status_class(), "is-success");
        assert_eq!(swarm("degraded", 0).status_class(), "is-warning");
        assert_eq!(swarm("ready", 0).status_tone(), crate::tone::Tone::Success);
    }

    #[test]
    fn stoppable_only_with_active_sessions() {
        assert!(!swarm("ready", 0).is_stoppable());
        assert!(swarm("ready", 2).is_stoppable());
    }
}

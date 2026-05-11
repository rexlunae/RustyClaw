//! Swarm configuration types.

use serde::{Deserialize, Serialize};

/// Predefined specialist roles for swarm agents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Routes requests — never executes tasks directly.
    Orchestrator,
    /// General-purpose virtual assistant (scheduling, messaging, integrations).
    VirtualAssistant,
    /// Evidence-based web/academic research with citations.
    DeepResearch,
    /// Data analysis, KPIs, charts, and statistical modelling.
    DataAnalyst,
    /// Slide deck creation, editing, and export.
    Slides,
    /// Document creation (PDF, Markdown, DOCX).
    Docs,
    /// Image generation and editing.
    ImageGeneration,
    /// Video generation and editing.
    VideoGeneration,
    /// User-defined role with a freeform name.
    Custom(String),
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Orchestrator => write!(f, "Orchestrator"),
            Self::VirtualAssistant => write!(f, "Virtual Assistant"),
            Self::DeepResearch => write!(f, "Deep Research"),
            Self::DataAnalyst => write!(f, "Data Analyst"),
            Self::Slides => write!(f, "Slides"),
            Self::Docs => write!(f, "Docs"),
            Self::ImageGeneration => write!(f, "Image Generation"),
            Self::VideoGeneration => write!(f, "Video Generation"),
            Self::Custom(name) => write!(f, "{name}"),
        }
    }
}

/// Communication pattern between agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    /// Parallel delegation — orchestrator dispatches and merges outputs.
    SendMessage,
    /// Full context transfer — specialist takes over the conversation.
    Handoff,
}

impl std::fmt::Display for FlowKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SendMessage => write!(f, "SendMessage"),
            Self::Handoff => write!(f, "Handoff"),
        }
    }
}

/// A directed communication edge between two agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationFlow {
    /// Source agent identifier (matches `SwarmAgent::id`).
    pub from: String,
    /// Target agent identifier.
    pub to: String,
    /// Communication pattern.
    pub kind: FlowKind,
}

/// Definition of a single agent within a swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmAgent {
    /// Unique identifier within this swarm (e.g. "orchestrator", "research").
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Specialist role.
    pub role: AgentRole,
    /// System-prompt instructions for this agent.
    pub instructions: String,
    /// Description shown in routing tables and UI.
    pub description: String,
    /// Tool names this agent is allowed to invoke (empty = all tools).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Example conversation starters.
    #[serde(default)]
    pub conversation_starters: Vec<String>,
}

/// Lifecycle status of a running swarm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmStatus {
    /// Defined but not yet started.
    Idle,
    /// Actively processing.
    Running,
    /// Paused — agents are alive but not accepting new tasks.
    Paused,
    /// Stopped — all agent sessions have been cleaned up.
    Stopped,
    /// An error prevented normal operation.
    Error,
}

impl std::fmt::Display for SwarmStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Running => write!(f, "Running"),
            Self::Paused => write!(f, "Paused"),
            Self::Stopped => write!(f, "Stopped"),
            Self::Error => write!(f, "Error"),
        }
    }
}

/// Full swarm configuration — can be loaded from TOML or created from a
/// built-in template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    /// Unique name for this swarm (e.g. "swarm", "seo-swarm").
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Shared instructions injected into every agent's system prompt.
    #[serde(default)]
    pub shared_instructions: String,
    /// Agent definitions.
    pub agents: Vec<SwarmAgent>,
    /// Communication flow edges.
    pub flows: Vec<CommunicationFlow>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_role_display() {
        assert_eq!(AgentRole::Orchestrator.to_string(), "Orchestrator");
        assert_eq!(AgentRole::DeepResearch.to_string(), "Deep Research");
        assert_eq!(
            AgentRole::Custom("SEO Planner".into()).to_string(),
            "SEO Planner"
        );
    }

    #[test]
    fn swarm_config_roundtrip() {
        let cfg = SwarmConfig {
            name: "test".into(),
            description: "A test swarm".into(),
            shared_instructions: String::new(),
            agents: vec![SwarmAgent {
                id: "orch".into(),
                name: "Orchestrator".into(),
                role: AgentRole::Orchestrator,
                instructions: "Route tasks.".into(),
                description: "Routes tasks.".into(),
                tools: vec![],
                conversation_starters: vec![],
            }],
            flows: vec![],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: SwarmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.name, "test");
        assert_eq!(deser.agents.len(), 1);
    }
}

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
    /// Source member identifier (matches `SwarmAgent::id`).
    pub from: String,
    /// Target member identifier.
    pub to: String,
    /// Communication pattern.
    pub kind: FlowKind,
}

/// Definition of a single member within a swarm.
///
/// This is a *spec*, not runtime state: when the swarm is created, each
/// member is materialized as a persistent registered agent whose system
/// prompt carries the member's instructions (plus the swarm's shared
/// instructions and roster).
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
    /// Tool names this agent should prefer (advisory; embedded in the
    /// generated system prompt — empty = no guidance).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Example conversation starters.
    #[serde(default)]
    pub conversation_starters: Vec<String>,
}

/// Full swarm configuration — can be loaded from TOML/JSON or created from
/// a built-in template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    /// Unique name for this swarm (e.g. "swarm", "seo-swarm"). Becomes the
    /// prefix of every member's registry agent id, so the same character
    /// rules apply (lowercase alphanumerics, `-`, `_`).
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Shared instructions injected into every member's system prompt.
    #[serde(default)]
    pub shared_instructions: String,
    /// Member definitions.
    pub agents: Vec<SwarmAgent>,
    /// Communication flow edges (by member id).
    pub flows: Vec<CommunicationFlow>,
}

impl SwarmConfig {
    /// The member that plays the orchestrator role, or the first member
    /// when none is marked as such. `None` only for an empty roster.
    pub fn orchestrator(&self) -> Option<&SwarmAgent> {
        self.agents
            .iter()
            .find(|a| a.role == AgentRole::Orchestrator)
            .or_else(|| self.agents.first())
    }

    /// Structural validation: non-empty roster, unique member ids, and
    /// flows that reference known members.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.agents.is_empty() {
            anyhow::bail!("Swarm '{}' has no agents", self.name);
        }
        let mut seen = std::collections::HashSet::new();
        for agent in &self.agents {
            if agent.id.trim().is_empty() {
                anyhow::bail!("Swarm '{}' has an agent with an empty id", self.name);
            }
            if !seen.insert(agent.id.as_str()) {
                anyhow::bail!(
                    "Swarm '{}' has duplicate agent id '{}'",
                    self.name,
                    agent.id
                );
            }
        }
        for flow in &self.flows {
            for endpoint in [&flow.from, &flow.to] {
                if !seen.contains(endpoint.as_str()) {
                    anyhow::bail!(
                        "Swarm '{}' flow references unknown agent '{}'",
                        self.name,
                        endpoint
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, role: AgentRole) -> SwarmAgent {
        SwarmAgent {
            id: id.into(),
            name: id.into(),
            role,
            instructions: "do things".into(),
            description: "a member".into(),
            tools: vec![],
            conversation_starters: vec![],
        }
    }

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
            agents: vec![member("orch", AgentRole::Orchestrator)],
            flows: vec![],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: SwarmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.name, "test");
        assert_eq!(deser.agents.len(), 1);

        // TOML too — that's the on-disk record format.
        let toml_str = toml::to_string(&cfg).unwrap();
        let deser: SwarmConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(deser.name, "test");
    }

    #[test]
    fn orchestrator_lookup_prefers_role_then_first() {
        let cfg = SwarmConfig {
            name: "t".into(),
            description: String::new(),
            shared_instructions: String::new(),
            agents: vec![
                member("helper", AgentRole::Docs),
                member("boss", AgentRole::Orchestrator),
            ],
            flows: vec![],
        };
        assert_eq!(cfg.orchestrator().unwrap().id, "boss");

        let no_orch = SwarmConfig {
            agents: vec![member("solo", AgentRole::Docs)],
            ..cfg
        };
        assert_eq!(no_orch.orchestrator().unwrap().id, "solo");
    }

    #[test]
    fn validate_rejects_bad_configs() {
        let base = SwarmConfig {
            name: "t".into(),
            description: String::new(),
            shared_instructions: String::new(),
            agents: vec![member("a", AgentRole::Orchestrator)],
            flows: vec![],
        };
        assert!(base.validate().is_ok());

        let empty = SwarmConfig {
            agents: vec![],
            ..base.clone()
        };
        assert!(empty.validate().is_err());

        let dup = SwarmConfig {
            agents: vec![
                member("a", AgentRole::Orchestrator),
                member("a", AgentRole::Docs),
            ],
            ..base.clone()
        };
        assert!(dup.validate().is_err());

        let bad_flow = SwarmConfig {
            flows: vec![CommunicationFlow {
                from: "a".into(),
                to: "ghost".into(),
                kind: FlowKind::SendMessage,
            }],
            ..base
        };
        assert!(bad_flow.validate().is_err());
    }
}

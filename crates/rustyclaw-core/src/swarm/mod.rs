//! Swarm — multi-agent orchestration.
//!
//! A *swarm* is a named collection of specialist agents coordinated by an
//! orchestrator.  Each agent has a role, system instructions, and a set of
//! allowed tools.  The orchestrator routes user requests to the right
//! specialist(s) using one of two communication patterns:
//!
//! * **SendMessage** — parallel delegation: the orchestrator dispatches
//!   independent sub-tasks to multiple specialists and merges their outputs.
//! * **Handoff** — full context transfer: the orchestrator yields the
//!   conversation to a single specialist who interacts directly with the user.
//!
//! Swarms can be created from built-in templates (e.g. the default 8-agent
//! layout) or defined in TOML configuration files.

mod config;
mod manager;
mod templates;

pub use config::{
    AgentRole, CommunicationFlow, FlowKind, SwarmAgent, SwarmConfig, SwarmStatus,
};
pub use manager::{SharedSwarmManager, SwarmInstance, SwarmManager, swarm_manager};
pub use templates::builtin_templates;

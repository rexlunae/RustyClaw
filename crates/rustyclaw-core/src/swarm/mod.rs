//! Swarm — multi-agent orchestration on top of the agent registry.
//!
//! A *swarm* is a named group of persistent agents (see [`crate::agents`])
//! coordinated by an orchestrator. Creating a swarm **materializes** its
//! members as real registered agents: each member gets its own directory
//! under `<settings_dir>/agents/<swarm>-<member>/` with the member's
//! instructions installed as the agent's system prompt. Members therefore
//! show up in `agents_list`, can be switched to from any connected client,
//! and can be targeted with `sessions_spawn` like any other agent.
//!
//! The swarm itself is recorded at `<settings_dir>/swarms/<name>.toml` —
//! filesystem-backed with no in-memory cache, for the same reason as the
//! agent registry: the gateway, CLI, and model tools must always agree.
//!
//! Coordination uses two communication patterns, declared as flows:
//!
//! * **SendMessage** — parallel delegation: the orchestrator dispatches
//!   independent sub-tasks to multiple specialists and merges their outputs.
//! * **Handoff** — full context transfer: the orchestrator yields the
//!   conversation to a single specialist who interacts directly with the user.
//!
//! Swarms can be created from built-in templates (e.g. the default 8-agent
//! layout) or from a custom [`SwarmConfig`].

mod config;
mod deploy;
mod store;
mod templates;

pub use config::{AgentRole, CommunicationFlow, FlowKind, SwarmAgent, SwarmConfig};
pub use deploy::{
    DeleteReport, MemberStatus, RouteResult, create_swarm, delete_swarm, member_statuses,
    send_to_member, stop_swarm_sessions, swarm_created_by,
};
pub use store::{SwarmMemberBinding, SwarmRecord, SwarmStore};
pub use templates::builtin_templates;

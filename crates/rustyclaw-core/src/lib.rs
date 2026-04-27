//! `rustyclaw-core` — shared configuration, gateway protocol, secrets management,
//! tool dispatch, skills, providers, commands, and shared display types used by
//! all RustyClaw clients.
//!
//! ## Crate layout
//!
//! - [`config`] — TOML-based configuration with OpenClaw compatibility
//! - [`gateway`] — WebSocket gateway protocol and session management
//! - [`tools`] — sandboxed tool dispatch (filesystem, web, shell, etc.)
//! - [`secrets`] — encrypted vault and credential storage
//! - [`providers`] — LLM provider abstraction (Anthropic, OpenAI, Ollama, …)
//! - [`messengers`] — messenger trait and built-in adapters
//! - [`skills`] — dynamic skill loading from the workspace directory
//! - [`soul`] — agent personality definition (`SOUL.md`)

pub mod args;
pub mod canvas;
pub mod commands;
pub mod config;
pub mod cron;
pub mod daemon;
pub mod error;
pub mod error_details;
pub mod gateway;
pub mod logging;
pub mod mcp;
pub mod memory;
pub mod memory_consolidation;
pub mod memory_flush;
pub mod messengers;
pub mod mnemo;
pub mod models;
pub mod observability;
pub mod pairing;
pub mod process_manager;
pub mod protocols;
pub mod provider_registry;
pub mod providers;
pub mod retry;
pub mod runtime;
pub mod runtime_ctx;
pub mod sandbox;
pub mod secrets;
pub mod security;
pub mod sessions;
pub mod skills;
pub mod soul;
#[cfg(feature = "steel-memory")]
pub mod steel_memory;
pub mod streaming;
pub mod tasks;
pub mod theme;
pub mod threads;
pub mod tools;
pub mod types;
pub mod user_prompt_types;
pub mod workspace_context;

// Re-export messenger types at crate root for convenience
pub use messengers::{
    GenericMessenger, Message, Messenger, MessengerManager, PresenceStatus, SendOptions,
};

// Re-export shared display types at crate root for convenience
pub use types::{GatewayStatus, InputMode, MessageRole};

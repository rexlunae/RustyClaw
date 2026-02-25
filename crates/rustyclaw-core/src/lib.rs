// ── RustyClaw Core Library ───────────────────────────────────────────────────
//
// This crate contains all shared logic used by any RustyClaw client:
// configuration, gateway protocol, secrets management, tool dispatch,
// skills, providers, commands, and shared display types.

pub mod args;
pub mod canvas;
pub mod commands;
pub mod config;
pub mod cron;
pub mod daemon;
pub mod error;
pub mod gateway;
pub mod logging;
pub mod mcp;
pub mod memory;
pub mod memory_consolidation;
pub mod memory_flush;
pub mod messengers;
pub mod observability;
pub mod process_manager;
pub mod providers;
pub mod retry;
pub mod runtime;
pub mod sandbox;
pub mod secrets;
pub mod security;
pub mod sessions;
pub mod skills;
pub mod soul;
pub mod streaming;
pub mod theme;
pub mod tools;
pub mod types;
pub mod user_prompt_types;
pub mod workspace_context;

// Re-export messenger types at crate root for convenience
pub use messengers::{Message, Messenger, MessengerManager, SendOptions};

// Re-export shared display types at crate root for convenience
pub use types::{GatewayStatus, InputMode, MessageRole};

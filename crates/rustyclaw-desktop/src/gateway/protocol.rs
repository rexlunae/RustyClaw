//! Protocol types for gateway communication.
//!
//! Re-exported from `rustyclaw_core::gateway::client_types` so the desktop
//! crate can keep existing import paths while the canonical definitions
//! live in the shared core crate.

pub use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent, ThreadInfoDto};

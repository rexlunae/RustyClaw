//! Command handlers extracted from main.rs for modularity.
//!
//! Each submodule handles a specific command group (gateway, skills, etc.)

pub mod config;
pub mod gateway;
pub mod import;
pub mod refresh_token;
pub mod shared;
pub mod status;

// Re-export handlers for use in main.rs
pub(crate) use config::{config_get, config_set, config_unset};
pub use gateway::{handle_restart, handle_run, handle_start, handle_status, handle_stop};
pub(crate) use import::run_import;
pub(crate) use refresh_token::run_refresh_token;

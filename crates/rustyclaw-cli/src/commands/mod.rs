//! Command handlers extracted from main.rs for modularity.
//!
//! Each submodule handles a specific command group (gateway, skills, etc.)

pub mod gateway;

// Re-export handlers for use in main.rs
pub use gateway::{
    handle_reload_result, handle_restart, handle_run, handle_start, handle_status, handle_stop,
    parse_gateway_defaults,
};

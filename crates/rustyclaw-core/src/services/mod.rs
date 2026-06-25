//! Managed backend services for the gateway.
//!
//! This module provides first-class process lifecycle management for backend
//! services that the gateway (and agents running on it) can start, stop,
//! restart, and interact with via MCP.
//!
//! # Service Types
//!
//! - **MCP servers** — native MCP stdio servers; tools are auto-discovered
//! - **HTTP services** — HTTP/REST backends with optional health checks
//! - **Custom processes** — arbitrary executables with log capture
//! - **Agentic payloads** — agent sub-sessions running as services
//!
//! # Configuration
//!
//! Services are declared in `rustyclaw.toml`:
//!
//! ```toml
//! [services.my-api]
//! command = "/usr/local/bin/my-api"
//! args = ["--port", "8080"]
//! service_type = "http"
//! restart = "on-failure"
//!
//! [services.my-mcp-server]
//! command = "npx"
//! args = ["-y", "@my/mcp-server"]
//! service_type = "mcp"
//! auto_start = true
//! ```

mod manager;
mod types;

pub use manager::ServiceManager;
pub use types::*;

use std::sync::Arc;
use tokio::sync::RwLock;

/// Thread-safe shared service manager.
pub type SharedServiceManager = Arc<RwLock<ServiceManager>>;

/// Create a new shared service manager.
pub fn create_service_manager(config: ServicesConfig) -> SharedServiceManager {
    Arc::new(RwLock::new(ServiceManager::new(config)))
}

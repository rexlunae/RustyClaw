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

pub use manager::{ServiceError, ServiceManager};
pub use types::*;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::debug;

/// Thread-safe shared service manager.
pub type SharedServiceManager = Arc<RwLock<ServiceManager>>;

/// Default interval for the service poller (2 seconds).
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Create a new shared service manager.
pub fn create_service_manager(config: ServicesConfig) -> SharedServiceManager {
    Arc::new(RwLock::new(ServiceManager::new(config)))
}

/// Spawn a background task that periodically polls service status and runs
/// health checks.
///
/// Returns a [`tokio::task::JoinHandle`] that can be used to abort the
/// polling loop on shutdown.
pub fn spawn_service_poller(
    mgr: SharedServiceManager,
    interval: Option<Duration>,
) -> tokio::task::JoinHandle<()> {
    let interval = interval.unwrap_or(DEFAULT_POLL_INTERVAL);

    tokio::spawn(async move {
        loop {
            {
                let mut m = mgr.write().await;
                let changed = m.poll().await;
                if !changed.is_empty() {
                    debug!(services = ?changed, "Service status changed");
                }
                let hc_changed = m.run_health_checks().await;
                if !hc_changed.is_empty() {
                    debug!(services = ?hc_changed, "Health check status changed");
                }
            }

            tokio::time::sleep(interval).await;
        }
    })
}

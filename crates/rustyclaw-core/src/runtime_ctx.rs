//! Global runtime context for tool access.
//!
//! This module provides a global store for runtime information that tools
//! need to access, such as the current model context, host capabilities,
//! and real-time load data.

use std::sync::{Arc, Mutex, OnceLock};

use crate::host::SharedHostCapabilities;
use crate::load::SharedLoadTracker;
use crate::services::SharedServiceManager;

/// Model information available to tools.
#[derive(Clone, Default)]
pub struct RuntimeInfo {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// Static host hardware profile.
    pub host: Option<SharedHostCapabilities>,
    /// Live load tracker (periodically sampled).
    pub load_tracker: Option<SharedLoadTracker>,
    /// Managed backend services.
    pub service_manager: Option<SharedServiceManager>,
}

/// Shared runtime context.
pub type SharedRuntimeCtx = Arc<Mutex<RuntimeInfo>>;

/// Global runtime context instance.
static RUNTIME_CTX: OnceLock<SharedRuntimeCtx> = OnceLock::new();

/// Get the global runtime context.
pub fn runtime_ctx() -> &'static SharedRuntimeCtx {
    RUNTIME_CTX.get_or_init(|| Arc::new(Mutex::new(RuntimeInfo::default())))
}

/// Update the runtime context with model information.
pub fn set_model_info(provider: &str, model: &str, base_url: &str) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.provider = Some(provider.to_string());
        ctx.model = Some(model.to_string());
        ctx.base_url = Some(base_url.to_string());
    }
}

/// Get current model information.
pub fn get_model_info() -> Option<(String, String, String)> {
    runtime_ctx().lock().ok().and_then(|ctx| {
        Some((
            ctx.provider.clone()?,
            ctx.model.clone()?,
            ctx.base_url.clone()?,
        ))
    })
}

/// Store the host capabilities snapshot in the runtime context.
pub fn set_host(host: SharedHostCapabilities) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.host = Some(host);
    }
}

/// Get the host capabilities snapshot.
pub fn get_host() -> Option<SharedHostCapabilities> {
    runtime_ctx().lock().ok().and_then(|ctx| ctx.host.clone())
}

/// Store the load tracker in the runtime context.
pub fn set_load_tracker(tracker: SharedLoadTracker) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.load_tracker = Some(tracker);
    }
}

/// Get the load tracker.
pub fn get_load_tracker() -> Option<SharedLoadTracker> {
    runtime_ctx()
        .lock()
        .ok()
        .and_then(|ctx| ctx.load_tracker.clone())
}

/// Store the service manager in the runtime context.
pub fn set_service_manager(mgr: SharedServiceManager) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.service_manager = Some(mgr);
    }
}

/// Get the service manager.
pub fn get_service_manager() -> Option<SharedServiceManager> {
    runtime_ctx()
        .lock()
        .ok()
        .and_then(|ctx| ctx.service_manager.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_ctx() {
        set_model_info(
            "github-copilot",
            "claude-sonnet-4",
            "https://api.githubcopilot.com",
        );
        let info = get_model_info();
        assert!(info.is_some());
        let (provider, model, base_url) = info.unwrap();
        assert_eq!(provider, "github-copilot");
        assert_eq!(model, "claude-sonnet-4");
        assert_eq!(base_url, "https://api.githubcopilot.com");
    }
}

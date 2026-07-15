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
    /// Root state directory (`~/.rustyclaw`), so tools can reach
    /// installation-level state such as the agent registry.
    pub settings_dir: Option<std::path::PathBuf>,
    /// Display name of the `main` agent (from `Config::agent_name`).
    pub main_agent_name: Option<String>,
    /// Agent id the currently-dispatching conversation belongs to.
    pub active_agent: Option<String>,
    /// Wakes the gateway's trigger manager after tool-side trigger edits.
    pub trigger_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    /// Static host hardware profile.
    pub host: Option<SharedHostCapabilities>,
    /// Live load tracker (periodically sampled).
    pub load_tracker: Option<SharedLoadTracker>,
    /// Managed backend services.
    pub service_manager: Option<SharedServiceManager>,
    /// Connected MCP servers.
    #[cfg(feature = "mcp")]
    pub mcp_manager: Option<crate::mcp::SharedMcpManager>,
    /// Usage stats / log-ring observer.
    pub stats_observer: Option<crate::observability::SharedStatsObserver>,
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

/// Store the settings dir and main-agent display name so tools can build
/// an agent registry.
pub fn set_agent_registry_info(settings_dir: &std::path::Path, main_agent_name: &str) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.settings_dir = Some(settings_dir.to_path_buf());
        ctx.main_agent_name = Some(main_agent_name.to_string());
    }
}

/// The gateway's settings dir, when published — the root for
/// installation-level state such as the agent registry and trigger store.
pub fn get_settings_dir() -> Option<std::path::PathBuf> {
    runtime_ctx()
        .lock()
        .ok()
        .and_then(|ctx| ctx.settings_dir.clone())
}

/// Register the trigger manager's wake handle so trigger tools can request
/// an immediate re-sync after editing the store.
pub fn set_trigger_notify(notify: std::sync::Arc<tokio::sync::Notify>) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.trigger_notify = Some(notify);
    }
}

/// Wake the trigger manager (no-op when no gateway is running).
pub fn notify_triggers_changed() {
    if let Ok(ctx) = runtime_ctx().lock() {
        if let Some(notify) = &ctx.trigger_notify {
            notify.notify_one();
        }
    }
}

/// Build an agent registry from the stored settings dir, if available.
pub fn get_agent_registry() -> Option<crate::agents::AgentRegistry> {
    runtime_ctx().lock().ok().and_then(|ctx| {
        let dir = ctx.settings_dir.clone()?;
        let name = ctx
            .main_agent_name
            .clone()
            .unwrap_or_else(|| "RustyClaw".to_string());
        Some(crate::agents::AgentRegistry::new(&dir, &name))
    })
}

/// Record which agent the currently-dispatching conversation belongs to.
///
/// NOTE: like the rest of [`RuntimeInfo`], this is a process-wide value with
/// last-writer-wins semantics. With multiple concurrent client connections
/// on different agents it may reflect another connection's agent, so treat
/// it as *best-effort* context: it feeds provenance metadata
/// (`AgentManifest::created_by`) and advisory guards, never authorization
/// or routing decisions. Per-connection routing state lives in the
/// gateway's `AgentSession`.
pub fn set_active_agent(agent_id: &str) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.active_agent = Some(agent_id.to_string());
    }
}

/// Agent id of the currently-dispatching conversation, when known.
/// Best-effort under concurrent connections — see [`set_active_agent`].
pub fn get_active_agent() -> Option<String> {
    runtime_ctx()
        .lock()
        .ok()
        .and_then(|ctx| ctx.active_agent.clone())
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

/// Store the MCP manager in the runtime context.
#[cfg(feature = "mcp")]
pub fn set_mcp_manager(mgr: crate::mcp::SharedMcpManager) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.mcp_manager = Some(mgr);
    }
}

/// Get the MCP manager.
#[cfg(feature = "mcp")]
pub fn get_mcp_manager() -> Option<crate::mcp::SharedMcpManager> {
    runtime_ctx()
        .lock()
        .ok()
        .and_then(|ctx| ctx.mcp_manager.clone())
}

/// Store the stats observer in the runtime context.
pub fn set_stats_observer(observer: crate::observability::SharedStatsObserver) {
    if let Ok(mut ctx) = runtime_ctx().lock() {
        ctx.stats_observer = Some(observer);
    }
}

/// Get the stats observer.
pub fn get_stats_observer() -> Option<crate::observability::SharedStatsObserver> {
    runtime_ctx()
        .lock()
        .ok()
        .and_then(|ctx| ctx.stats_observer.clone())
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

//! Lifecycle Hook System
//!
//! Provides an extensible hook system for gateway and tool lifecycle events.
//! Hooks can observe events, modify execution context, or abort operations.

pub mod builtin;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Hook execution context containing event metadata
#[derive(Debug, Clone)]
pub struct HookContext {
    /// Event type that triggered the hook
    pub event: HookEvent,
    /// Event metadata (tool name, connection ID, etc.)
    pub metadata: HashMap<String, Value>,
    /// Timestamp of the event
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl HookContext {
    pub fn new(event: HookEvent) -> Self {
        Self {
            event,
            metadata: HashMap::new(),
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub fn get_metadata(&self, key: &str) -> Option<&Value> {
        self.metadata.get(key)
    }
}

/// Lifecycle events that can trigger hooks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    /// Gateway started
    Startup,
    /// Gateway shutting down
    Shutdown,
    /// New WebSocket connection established
    Connection,
    /// WebSocket connection closed
    Disconnection,
    /// Authentication succeeded
    AuthSuccess,
    /// Authentication failed
    AuthFailure,
    /// About to execute a tool
    BeforeToolCall,
    /// Tool execution completed
    AfterToolCall,
    /// About to call LLM provider
    BeforeProviderCall,
    /// LLM provider call completed
    AfterProviderCall,
    /// Configuration reloaded
    ConfigReload,
    /// Security event detected
    SecurityEvent,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Startup => "Startup",
            Self::Shutdown => "Shutdown",
            Self::Connection => "Connection",
            Self::Disconnection => "Disconnection",
            Self::AuthSuccess => "AuthSuccess",
            Self::AuthFailure => "AuthFailure",
            Self::BeforeToolCall => "BeforeToolCall",
            Self::AfterToolCall => "AfterToolCall",
            Self::BeforeProviderCall => "BeforeProviderCall",
            Self::AfterProviderCall => "AfterProviderCall",
            Self::ConfigReload => "ConfigReload",
            Self::SecurityEvent => "SecurityEvent",
        }
    }
}

/// Action to take after hook execution
#[derive(Debug)]
pub enum HookAction {
    /// Continue with normal execution
    Continue,
    /// Abort operation with error message
    Abort(String),
    /// Modify execution context (for advanced hooks)
    ModifyContext(HashMap<String, Value>),
}

/// Lifecycle hook trait
#[async_trait]
pub trait LifecycleHook: Send + Sync {
    /// Hook name for logging/debugging
    fn name(&self) -> &str;

    /// Events this hook is interested in
    fn events(&self) -> &[HookEvent];

    /// Called when a lifecycle event occurs
    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction> {
        // Default implementation for convenience
        match ctx.event {
            HookEvent::Startup => self.on_startup(ctx).await,
            HookEvent::Shutdown => self.on_shutdown(ctx).await,
            HookEvent::Connection => self.on_connection(ctx).await,
            HookEvent::Disconnection => self.on_disconnection(ctx).await,
            HookEvent::AuthSuccess => self.on_auth_success(ctx).await,
            HookEvent::AuthFailure => self.on_auth_failure(ctx).await,
            HookEvent::BeforeToolCall => self.on_before_tool_call(ctx).await,
            HookEvent::AfterToolCall => self.on_after_tool_call(ctx).await,
            HookEvent::BeforeProviderCall => self.on_before_provider_call(ctx).await,
            HookEvent::AfterProviderCall => self.on_after_provider_call(ctx).await,
            HookEvent::ConfigReload => self.on_config_reload(ctx).await,
            HookEvent::SecurityEvent => self.on_security_event(ctx).await,
        }
    }

    // Individual event handlers (default: continue)
    async fn on_startup(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_shutdown(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_connection(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_disconnection(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_auth_success(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_auth_failure(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_before_tool_call(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_after_tool_call(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_before_provider_call(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_after_provider_call(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_config_reload(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }

    async fn on_security_event(&self, _ctx: &HookContext) -> Result<HookAction> {
        Ok(HookAction::Continue)
    }
}

/// Hook registry that manages and invokes hooks
pub struct HookRegistry {
    hooks: Vec<Arc<dyn LifecycleHook>>,
    event_map: HashMap<HookEvent, Vec<Arc<dyn LifecycleHook>>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: Vec::new(),
            event_map: HashMap::new(),
        }
    }

    /// Register a new hook
    pub fn register(&mut self, hook: Arc<dyn LifecycleHook>) {
        let events = hook.events();

        // Add to event map
        for event in events {
            self.event_map
                .entry(*event)
                .or_insert_with(Vec::new)
                .push(hook.clone());
        }

        // Add to hooks list
        self.hooks.push(hook);
    }

    /// Invoke all hooks for a specific event
    pub async fn invoke(&self, ctx: &HookContext) -> Result<HookAction> {
        if let Some(hooks) = self.event_map.get(&ctx.event) {
            for hook in hooks {
                match hook.on_event(ctx).await {
                    Ok(HookAction::Continue) => continue,
                    Ok(action) => return Ok(action),
                    Err(e) => {
                        eprintln!("[hooks] Error in hook '{}': {}", hook.name(), e);
                        // Continue with other hooks on error
                        continue;
                    }
                }
            }
        }
        Ok(HookAction::Continue)
    }

    /// Get number of registered hooks
    pub fn count(&self) -> usize {
        self.hooks.len()
    }

    /// Get all registered hook names
    pub fn hook_names(&self) -> Vec<String> {
        self.hooks.iter().map(|h| h.name().to_string()).collect()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHook {
        name: String,
        events: Vec<HookEvent>,
        should_abort: bool,
    }

    #[async_trait]
    impl LifecycleHook for TestHook {
        fn name(&self) -> &str {
            &self.name
        }

        fn events(&self) -> &[HookEvent] {
            &self.events
        }

        async fn on_before_tool_call(&self, _ctx: &HookContext) -> Result<HookAction> {
            if self.should_abort {
                Ok(HookAction::Abort("Test abort".to_string()))
            } else {
                Ok(HookAction::Continue)
            }
        }
    }

    #[tokio::test]
    async fn test_hook_registry_registration() {
        let mut registry = HookRegistry::new();

        let hook = Arc::new(TestHook {
            name: "test_hook".to_string(),
            events: vec![HookEvent::BeforeToolCall],
            should_abort: false,
        });

        registry.register(hook);
        assert_eq!(registry.count(), 1);
        assert_eq!(registry.hook_names(), vec!["test_hook"]);
    }

    #[tokio::test]
    async fn test_hook_invocation_continue() {
        let mut registry = HookRegistry::new();

        let hook = Arc::new(TestHook {
            name: "continue_hook".to_string(),
            events: vec![HookEvent::BeforeToolCall],
            should_abort: false,
        });

        registry.register(hook);

        let ctx = HookContext::new(HookEvent::BeforeToolCall)
            .with_metadata("tool_name", "test_tool");

        let action = registry.invoke(&ctx).await.unwrap();
        match action {
            HookAction::Continue => (),
            _ => panic!("Expected Continue action"),
        }
    }

    #[tokio::test]
    async fn test_hook_invocation_abort() {
        let mut registry = HookRegistry::new();

        let hook = Arc::new(TestHook {
            name: "abort_hook".to_string(),
            events: vec![HookEvent::BeforeToolCall],
            should_abort: true,
        });

        registry.register(hook);

        let ctx = HookContext::new(HookEvent::BeforeToolCall)
            .with_metadata("tool_name", "test_tool");

        let action = registry.invoke(&ctx).await.unwrap();
        match action {
            HookAction::Abort(msg) => assert_eq!(msg, "Test abort"),
            _ => panic!("Expected Abort action"),
        }
    }

    #[tokio::test]
    async fn test_hook_context_metadata() {
        let ctx = HookContext::new(HookEvent::Connection)
            .with_metadata("peer_addr", "127.0.0.1:8080")
            .with_metadata("connection_id", 123);

        assert_eq!(ctx.get_metadata("peer_addr").unwrap(), "127.0.0.1:8080");
        assert_eq!(ctx.get_metadata("connection_id").unwrap(), 123);
        assert!(ctx.get_metadata("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_hook_event_as_str() {
        assert_eq!(HookEvent::Startup.as_str(), "Startup");
        assert_eq!(HookEvent::BeforeToolCall.as_str(), "BeforeToolCall");
        assert_eq!(HookEvent::SecurityEvent.as_str(), "SecurityEvent");
    }
}

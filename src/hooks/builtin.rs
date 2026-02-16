//! Built-in lifecycle hooks

use super::{HookAction, HookContext, HookEvent, LifecycleHook};
use crate::metrics;
use anyhow::Result;
use async_trait::async_trait;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

/// Metrics hook - Updates Prometheus metrics on lifecycle events
pub struct MetricsHook;

#[async_trait]
impl LifecycleHook for MetricsHook {
    fn name(&self) -> &str {
        "metrics"
    }

    fn events(&self) -> &[HookEvent] {
        &[
            HookEvent::Connection,
            HookEvent::Disconnection,
            HookEvent::AuthSuccess,
            HookEvent::AuthFailure,
            HookEvent::BeforeToolCall,
            HookEvent::AfterToolCall,
            HookEvent::BeforeProviderCall,
            HookEvent::AfterProviderCall,
            HookEvent::SecurityEvent,
        ]
    }

    async fn on_connection(&self, _ctx: &HookContext) -> Result<HookAction> {
        metrics::GATEWAY_CONNECTIONS.inc();
        Ok(HookAction::Continue)
    }

    async fn on_disconnection(&self, _ctx: &HookContext) -> Result<HookAction> {
        metrics::GATEWAY_CONNECTIONS.dec();
        Ok(HookAction::Continue)
    }

    async fn on_auth_success(&self, _ctx: &HookContext) -> Result<HookAction> {
        metrics::AUTH_ATTEMPTS_TOTAL.with_label_values(&["success"]).inc();
        Ok(HookAction::Continue)
    }

    async fn on_auth_failure(&self, _ctx: &HookContext) -> Result<HookAction> {
        metrics::AUTH_ATTEMPTS_TOTAL.with_label_values(&["failure"]).inc();
        Ok(HookAction::Continue)
    }

    async fn on_after_tool_call(&self, ctx: &HookContext) -> Result<HookAction> {
        if let Some(tool_name) = ctx.get_metadata("tool_name") {
            if let Some(name) = tool_name.as_str() {
                let success = ctx
                    .get_metadata("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let result = if success { "success" } else { "error" };
                metrics::TOOL_CALLS_TOTAL
                    .with_label_values(&[name, result])
                    .inc();
            }
        }
        Ok(HookAction::Continue)
    }

    async fn on_after_provider_call(&self, ctx: &HookContext) -> Result<HookAction> {
        if let Some(provider) = ctx.get_metadata("provider") {
            if let Some(name) = provider.as_str() {
                let success = ctx
                    .get_metadata("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let result = if success { "success" } else { "error" };
                metrics::PROVIDER_REQUESTS_TOTAL
                    .with_label_values(&[name, result])
                    .inc();

                // Track token usage if available
                if let Some(input_tokens) = ctx.get_metadata("input_tokens") {
                    if let Some(tokens) = input_tokens.as_i64() {
                        metrics::TOKENS_TOTAL
                            .with_label_values(&[name, "input"])
                            .inc_by(tokens as f64);
                    }
                }
                if let Some(output_tokens) = ctx.get_metadata("output_tokens") {
                    if let Some(tokens) = output_tokens.as_i64() {
                        metrics::TOKENS_TOTAL
                            .with_label_values(&[name, "output"])
                            .inc_by(tokens as f64);
                    }
                }
            }
        }
        Ok(HookAction::Continue)
    }

    async fn on_security_event(&self, ctx: &HookContext) -> Result<HookAction> {
        if let Some(event_type) = ctx.get_metadata("event_type") {
            if let Some(name) = event_type.as_str() {
                // Map security events to appropriate metrics
                match name {
                    "prompt_injection" => {
                        let action = ctx
                            .get_metadata("action")
                            .and_then(|v| v.as_str())
                            .unwrap_or("warn");
                        metrics::PROMPT_INJECTION_DETECTED
                            .with_label_values(&[action])
                            .inc();
                    }
                    "ssrf" => {
                        let reason = ctx
                            .get_metadata("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        metrics::SSRF_BLOCKED_TOTAL.with_label_values(&[reason]).inc();
                    }
                    _ => {
                        // Generic security event - could add a catch-all metric
                    }
                }
            }
        }
        Ok(HookAction::Continue)
    }
}

/// Audit log hook - Logs security-relevant events to file
pub struct AuditLogHook {
    log_path: PathBuf,
}

impl AuditLogHook {
    pub fn new(log_path: PathBuf) -> Self {
        // Ensure parent directory exists
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        Self { log_path }
    }

    fn write_log_entry(&self, ctx: &HookContext) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;

        let timestamp = ctx.timestamp.format("%Y-%m-%d %H:%M:%S%.3f");
        let event = ctx.event.as_str();

        // Build log line
        let mut log_line = format!("[{}] {}", timestamp, event);

        // Add relevant metadata
        for (key, value) in &ctx.metadata {
            log_line.push_str(&format!(" {}={}", key, value));
        }

        writeln!(file, "{}", log_line)?;
        Ok(())
    }
}

#[async_trait]
impl LifecycleHook for AuditLogHook {
    fn name(&self) -> &str {
        "audit_log"
    }

    fn events(&self) -> &[HookEvent] {
        &[
            HookEvent::AuthSuccess,
            HookEvent::AuthFailure,
            HookEvent::BeforeToolCall,
            HookEvent::SecurityEvent,
            HookEvent::ConfigReload,
        ]
    }

    async fn on_auth_success(&self, ctx: &HookContext) -> Result<HookAction> {
        if let Err(e) = self.write_log_entry(ctx) {
            eprintln!("[audit_log] Failed to write log entry: {}", e);
        }
        Ok(HookAction::Continue)
    }

    async fn on_auth_failure(&self, ctx: &HookContext) -> Result<HookAction> {
        if let Err(e) = self.write_log_entry(ctx) {
            eprintln!("[audit_log] Failed to write log entry: {}", e);
        }
        Ok(HookAction::Continue)
    }

    async fn on_before_tool_call(&self, ctx: &HookContext) -> Result<HookAction> {
        // Only log security-sensitive tools
        if let Some(tool_name) = ctx.get_metadata("tool_name") {
            if let Some(name) = tool_name.as_str() {
                if matches!(
                    name,
                    "execute_command" | "secrets_get" | "secrets_store" | "gateway"
                ) {
                    if let Err(e) = self.write_log_entry(ctx) {
                        eprintln!("[audit_log] Failed to write log entry: {}", e);
                    }
                }
            }
        }
        Ok(HookAction::Continue)
    }

    async fn on_security_event(&self, ctx: &HookContext) -> Result<HookAction> {
        if let Err(e) = self.write_log_entry(ctx) {
            eprintln!("[audit_log] Failed to write log entry: {}", e);
        }
        Ok(HookAction::Continue)
    }

    async fn on_config_reload(&self, ctx: &HookContext) -> Result<HookAction> {
        if let Err(e) = self.write_log_entry(ctx) {
            eprintln!("[audit_log] Failed to write log entry: {}", e);
        }
        Ok(HookAction::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_metrics_hook_connection() {
        let hook = MetricsHook;
        let ctx = HookContext::new(HookEvent::Connection);

        let action = hook.on_connection(&ctx).await.unwrap();
        match action {
            HookAction::Continue => (),
            _ => panic!("Expected Continue action"),
        }
    }

    #[tokio::test]
    async fn test_audit_log_hook_writes_entry() {
        let temp_file = NamedTempFile::new().unwrap();
        let log_path = temp_file.path().to_path_buf();

        let hook = AuditLogHook::new(log_path.clone());

        let ctx = HookContext::new(HookEvent::AuthSuccess)
            .with_metadata("user", "test_user")
            .with_metadata("method", "totp");

        let action = hook.on_auth_success(&ctx).await.unwrap();
        match action {
            HookAction::Continue => (),
            _ => panic!("Expected Continue action"),
        }

        // Verify log file was written
        let log_content = std::fs::read_to_string(&log_path).unwrap();
        assert!(log_content.contains("AuthSuccess"));
        assert!(log_content.contains("user="));
        assert!(log_content.contains("method="));
    }

    #[tokio::test]
    async fn test_audit_log_hook_filters_tools() {
        let temp_file = NamedTempFile::new().unwrap();
        let log_path = temp_file.path().to_path_buf();

        let hook = AuditLogHook::new(log_path.clone());

        // Security-sensitive tool - should log
        let ctx1 = HookContext::new(HookEvent::BeforeToolCall)
            .with_metadata("tool_name", "execute_command");

        hook.on_before_tool_call(&ctx1).await.unwrap();

        // Non-sensitive tool - should not log
        let ctx2 = HookContext::new(HookEvent::BeforeToolCall)
            .with_metadata("tool_name", "read_file");

        hook.on_before_tool_call(&ctx2).await.unwrap();

        let log_content = std::fs::read_to_string(&log_path).unwrap();
        assert!(log_content.contains("execute_command"));
        assert!(!log_content.contains("read_file"));
    }
}

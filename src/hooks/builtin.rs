//! Built-in lifecycle hooks

use super::{HookAction, HookContext, HookEvent, LifecycleHook};
use crate::metrics;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Map, Value};
use std::collections::VecDeque;
use std::fs;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};

const AUDIT_ROTATE_BYTES: u64 = 50 * 1024 * 1024;
const AUDIT_QUEUE_CAPACITY: usize = 1024;

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
    sender: SyncSender<String>,
    queue_warned: AtomicBool,
}

impl AuditLogHook {
    pub fn new(log_path: PathBuf) -> Self {
        // Ensure parent directory exists before starting writer thread.
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let (sender, receiver) = mpsc::sync_channel::<String>(AUDIT_QUEUE_CAPACITY);
        let writer_path = log_path.clone();
        std::thread::Builder::new()
            .name("rustyclaw-audit-log".to_string())
            .spawn(move || run_audit_writer(writer_path, receiver))
            .expect("failed to spawn audit log writer thread");

        Self {
            sender,
            queue_warned: AtomicBool::new(false),
        }
    }

    fn enqueue_log_entry(&self, ctx: &HookContext) {
        let entry = build_log_entry(ctx);
        match self.sender.try_send(entry) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                // Avoid flooding stderr if queue is persistently full.
                if !self.queue_warned.swap(true, Ordering::Relaxed) {
                    eprintln!("[audit_log] Queue full; dropping audit events");
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                eprintln!("[audit_log] Writer disconnected; dropping audit event");
            }
        }
    }
}

fn run_audit_writer(log_path: PathBuf, receiver: mpsc::Receiver<String>) {
    while let Ok(entry) = receiver.recv() {
        if let Err(e) = write_log_entry_with_rotation(&log_path, &entry) {
            eprintln!("[audit_log] Failed to write log entry: {}", e);
        }
    }
}

fn build_log_entry(ctx: &HookContext) -> String {
    let mut metadata = Map::new();
    for (key, value) in &ctx.metadata {
        metadata.insert(key.clone(), value.clone());
    }

    let record = serde_json::json!({
        "timestamp": ctx.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "event": ctx.event.as_str(),
        "metadata": metadata,
    });

    // Serialization failures are extremely unlikely; preserve a valid JSON line either way.
    serde_json::to_string(&record).unwrap_or_else(|_| {
        serde_json::json!({
            "timestamp": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "event": "AuditSerializationError",
            "metadata": {"error": "failed_to_serialize_entry"}
        })
        .to_string()
    })
}

fn write_log_entry_with_rotation(log_path: &Path, entry: &str) -> Result<()> {
    rotate_if_needed(log_path, entry.as_bytes().len() as u64 + 1)?;

    let mut file = OpenOptions::new().create(true).append(true).open(log_path)?;
    writeln!(file, "{}", entry)?;
    Ok(())
}

fn rotate_if_needed(log_path: &Path, incoming_bytes: u64) -> Result<()> {
    let current_size = match fs::metadata(log_path) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    if current_size.saturating_add(incoming_bytes) <= AUDIT_ROTATE_BYTES {
        return Ok(());
    }

    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let stem = log_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audit");
    let ext = log_path.extension().and_then(|e| e.to_str()).unwrap_or("log");
    let rotated = log_path.with_file_name(format!("{}.{}.{}", stem, timestamp, ext));
    fs::rename(log_path, rotated)?;
    Ok(())
}

pub fn query_audit_log(
    log_path: &Path,
    event_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let file = match fs::File::open(log_path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let mut recent = VecDeque::with_capacity(limit);
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<Value>(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(filter) = event_filter {
            let event = value.get("event").and_then(|v| v.as_str()).unwrap_or("");
            if event != filter {
                continue;
            }
        }
        if recent.len() == limit {
            recent.pop_front();
        }
        recent.push_back(value);
    }

    Ok(recent.into_iter().collect())
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
        self.enqueue_log_entry(ctx);
        Ok(HookAction::Continue)
    }

    async fn on_auth_failure(&self, ctx: &HookContext) -> Result<HookAction> {
        self.enqueue_log_entry(ctx);
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
                    self.enqueue_log_entry(ctx);
                }
            }
        }
        Ok(HookAction::Continue)
    }

    async fn on_security_event(&self, ctx: &HookContext) -> Result<HookAction> {
        self.enqueue_log_entry(ctx);
        Ok(HookAction::Continue)
    }

    async fn on_config_reload(&self, ctx: &HookContext) -> Result<HookAction> {
        self.enqueue_log_entry(ctx);
        Ok(HookAction::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::time::{Duration, Instant};

    fn wait_for_log(path: &Path, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(content) = std::fs::read_to_string(path) {
                if content.contains(needle) {
                    return;
                }
            }
            assert!(Instant::now() < deadline, "timed out waiting for {}", needle);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

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
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("audit.log");

        let hook = AuditLogHook::new(log_path.clone());

        let ctx = HookContext::new(HookEvent::AuthSuccess)
            .with_metadata("user", "test_user")
            .with_metadata("method", "totp");

        let action = hook.on_auth_success(&ctx).await.unwrap();
        match action {
            HookAction::Continue => (),
            _ => panic!("Expected Continue action"),
        }

        wait_for_log(&log_path, "AuthSuccess");
        let log_content = std::fs::read_to_string(&log_path).unwrap();
        assert!(log_content.contains("\"event\":\"AuthSuccess\""));
        assert!(log_content.contains("\"user\":\"test_user\""));
        assert!(log_content.contains("\"method\":\"totp\""));
    }

    #[tokio::test]
    async fn test_audit_log_hook_filters_tools() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("audit.log");

        let hook = AuditLogHook::new(log_path.clone());

        // Security-sensitive tool - should log
        let ctx1 = HookContext::new(HookEvent::BeforeToolCall)
            .with_metadata("tool_name", "execute_command");

        hook.on_before_tool_call(&ctx1).await.unwrap();

        // Non-sensitive tool - should not log
        let ctx2 = HookContext::new(HookEvent::BeforeToolCall)
            .with_metadata("tool_name", "read_file");

        hook.on_before_tool_call(&ctx2).await.unwrap();

        wait_for_log(&log_path, "execute_command");
        let log_content = std::fs::read_to_string(&log_path).unwrap();
        assert!(log_content.contains("execute_command"));
        assert!(!log_content.contains("read_file"));
    }

    #[test]
    fn test_query_audit_log_filters_and_limits() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("audit.log");
        let lines = [
            serde_json::json!({"timestamp":"2026-01-01T00:00:00.000Z","event":"AuthSuccess","metadata":{"user":"a"}}).to_string(),
            serde_json::json!({"timestamp":"2026-01-01T00:01:00.000Z","event":"SecurityEvent","metadata":{"action":"block"}}).to_string(),
            serde_json::json!({"timestamp":"2026-01-01T00:02:00.000Z","event":"AuthSuccess","metadata":{"user":"b"}}).to_string(),
        ];
        std::fs::write(&log_path, format!("{}\n{}\n{}\n", lines[0], lines[1], lines[2])).unwrap();

        let filtered = query_audit_log(&log_path, Some("AuthSuccess"), 10).unwrap();
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0]["metadata"]["user"], "a");
        assert_eq!(filtered[1]["metadata"]["user"], "b");

        let limited = query_audit_log(&log_path, None, 2).unwrap();
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0]["event"], "SecurityEvent");
        assert_eq!(limited[1]["event"], "AuthSuccess");
    }

    #[test]
    fn test_rotation_when_size_exceeded() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("audit.log");
        std::fs::write(&log_path, vec![b'x'; (AUDIT_ROTATE_BYTES as usize) + 64]).unwrap();

        write_log_entry_with_rotation(&log_path, "{\"event\":\"AuthSuccess\"}").unwrap();

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        assert!(
            entries.iter().any(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("audit.") && n.ends_with(".log"))
                    .unwrap_or(false)
            }),
            "expected a rotated audit file"
        );
        let current = std::fs::read_to_string(&log_path).unwrap();
        assert!(current.contains("AuthSuccess"));
    }
}

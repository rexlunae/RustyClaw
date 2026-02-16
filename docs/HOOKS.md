# Lifecycle Hooks System

RustyClaw provides an extensible lifecycle hooks system that allows you to observe and respond to gateway and tool events in real-time.

## Overview

Hooks are callback functions that execute at specific lifecycle events (connection, authentication, tool execution, etc.). They can:
- **Observe** events for monitoring and logging
- **Modify** execution context (advanced use)
- **Abort** operations based on custom logic

## Built-in Hooks

### 1. Metrics Hook

Automatically updates Prometheus metrics for lifecycle events.

**Events handled:**
- `Connection` / `Disconnection` - Updates active connection gauge
- `AuthSuccess` / `AuthFailure` - Tracks authentication attempts
- `AfterToolCall` - Records tool execution counts
- `AfterProviderCall` - Tracks LLM API calls and token usage
- `SecurityEvent` - Records security blocks (SSRF, prompt injection)

**Configuration:**
```toml
[hooks]
enabled = true
metrics_hook = true  # Enabled by default
```

**Metrics exported:**
- `rustyclaw_gateway_connections` — Active connections
- `rustyclaw_auth_attempts_total{result}` — Auth attempts (success/failure)
- `rustyclaw_tool_calls_total{tool_name,result}` — Tool usage
- `rustyclaw_provider_requests_total{provider,result}` — API calls
- `rustyclaw_tokens_total{provider,type}` — Token usage
- `rustyclaw_prompt_injection_detected_total{action}` — Prompt injection blocks
- `rustyclaw_ssrf_blocked_total{reason}` — SSRF blocks

### 2. Audit Log Hook

Logs security-relevant events to a file for compliance and forensics.

**Events logged:**
- `AuthSuccess` / `AuthFailure` - Authentication attempts
- `BeforeToolCall` - Security-sensitive tools only (execute_command, secrets_*, gateway)
- `SecurityEvent` - All security blocks
- `ConfigReload` - Configuration changes

**Configuration:**
```toml
[hooks]
enabled = true
audit_log_hook = true
audit_log_path = "/var/log/rustyclaw/audit.log"  # Optional, defaults to ~/.rustyclaw/logs/audit.log
```

**Log format:**
```
[2026-02-16 10:30:15.123] AuthSuccess peer_addr="192.168.1.100:54321" method="totp"
[2026-02-16 10:30:18.456] BeforeToolCall tool_name="execute_command" tool_id="call_abc123"
[2026-02-16 10:30:19.789] SecurityEvent event_type="prompt_injection" action="block" peer_addr="192.168.1.200:12345"
[2026-02-16 10:31:00.000] ConfigReload changes="ssrf.enabled: false -> true"
```

## Lifecycle Events

### Gateway Events

| Event | When | Metadata | Hook Use Case |
|-------|------|----------|---------------|
| `Startup` | Gateway starts | `gateway_url`, `tls_enabled` | Initialize resources, log startup |
| `Shutdown` | Gateway stops | - | Cleanup, final metrics flush |
| `Connection` | New WebSocket connection | `peer_addr`, `peer_ip` | Rate limiting, IP filtering |
| `Disconnection` | Connection closed | `peer_addr`, `duration` | Cleanup, session logging |
| `ConfigReload` | Config hot-reloaded | `changes` | Audit config changes |

### Authentication Events

| Event | When | Metadata | Hook Use Case |
|-------|------|----------|---------------|
| `AuthSuccess` | Auth succeeds | `peer_addr`, `method` | Login notifications, metrics |
| `AuthFailure` | Auth fails | `peer_addr`, `method`, `attempts`, `locked_out` | Intrusion detection, alerting |

### Tool Events

| Event | When | Metadata | Hook Use Case |
|-------|------|----------|---------------|
| `BeforeToolCall` | Before tool execution | `tool_name`, `tool_id` | Tool authorization, pre-processing |
| `AfterToolCall` | After tool execution | `tool_name`, `tool_id`, `success` | Result logging, post-processing |

### Provider Events

| Event | When | Metadata | Hook Use Case |
|-------|------|----------|---------------|
| `BeforeProviderCall` | Before LLM API call | `provider`, `model` | Request logging, quota checks |
| `AfterProviderCall` | After LLM API call | `provider`, `model`, `success`, `input_tokens`, `output_tokens` | Token accounting, error tracking |

### Security Events

| Event | When | Metadata | Hook Use Case |
|-------|------|----------|---------------|
| `SecurityEvent` | Security violation detected | `event_type`, `action`, `reason` | Alerting, blocking, forensics |

**Security event types:**
- `prompt_injection` — Prompt injection attempt detected
- `ssrf` — SSRF attack blocked
- `command_injection` — Command injection attempt
- `rate_limit` — Rate limit exceeded

## Configuration

### Enabling Hooks

```toml
[hooks]
enabled = true  # Master switch (default: true)
metrics_hook = true  # Prometheus metrics (default: true)
audit_log_hook = false  # Security audit log (default: false)
audit_log_path = "~/.rustyclaw/logs/audit.log"  # Optional custom path
```

### Disabling Hooks

```toml
[hooks]
enabled = false  # Disables all hooks
```

Or disable specific hooks:
```toml
[hooks]
enabled = true
metrics_hook = false  # No metrics
audit_log_hook = false  # No audit log
```

## Custom Hooks (Advanced)

You can implement custom hooks by creating a type that implements the `LifecycleHook` trait.

### Example: Slack Notification Hook

```rust
use rustyclaw::hooks::{LifecycleHook, HookContext, HookEvent, HookAction};
use async_trait::async_trait;
use anyhow::Result;

struct SlackHook {
    webhook_url: String,
}

#[async_trait]
impl LifecycleHook for SlackHook {
    fn name(&self) -> &str {
        "slack_notifications"
    }

    fn events(&self) -> &[HookEvent] {
        &[HookEvent::AuthFailure, HookEvent::SecurityEvent]
    }

    async fn on_auth_failure(&self, ctx: &HookContext) -> Result<HookAction> {
        let peer_addr = ctx.get_metadata("peer_addr")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Send Slack notification
        let message = format!("🔒 Auth failure from {}", peer_addr);
        send_slack_message(&self.webhook_url, &message).await?;

        Ok(HookAction::Continue)
    }

    async fn on_security_event(&self, ctx: &HookContext) -> Result<HookAction> {
        let event_type = ctx.get_metadata("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Send Slack alert
        let message = format!("🚨 Security event: {}", event_type);
        send_slack_message(&self.webhook_url, &message).await?;

        Ok(HookAction::Continue)
    }
}

async fn send_slack_message(webhook_url: &str, message: &str) -> Result<()> {
    // Implementation...
    Ok(())
}
```

### Example: IP Whitelist Hook

```rust
use rustyclaw::hooks::{LifecycleHook, HookContext, HookEvent, HookAction};
use async_trait::async_trait;
use anyhow::Result;
use std::net::IpAddr;

struct IpWhitelistHook {
    allowed_ips: Vec<IpAddr>,
}

#[async_trait]
impl LifecycleHook for IpWhitelistHook {
    fn name(&self) -> &str {
        "ip_whitelist"
    }

    fn events(&self) -> &[HookEvent] {
        &[HookEvent::Connection]
    }

    async fn on_connection(&self, ctx: &HookContext) -> Result<HookAction> {
        let peer_ip = ctx.get_metadata("peer_ip")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<IpAddr>().ok());

        if let Some(ip) = peer_ip {
            if !self.allowed_ips.contains(&ip) {
                return Ok(HookAction::Abort(
                    format!("IP {} not in whitelist", ip)
                ));
            }
        }

        Ok(HookAction::Continue)
    }
}
```

### Registering Custom Hooks

Custom hooks must be registered in the gateway initialization code:

```rust
use rustyclaw::hooks::HookRegistry;
use std::sync::Arc;

let mut registry = HookRegistry::new();

// Register built-in hooks
registry.register(Arc::new(MetricsHook));
registry.register(Arc::new(AuditLogHook::new(log_path)));

// Register custom hooks
registry.register(Arc::new(SlackHook {
    webhook_url: "https://hooks.slack.com/...".to_string(),
}));

registry.register(Arc::new(IpWhitelistHook {
    allowed_ips: vec!["192.168.1.0/24".parse()?],
}));
```

## Hook Actions

Hooks can return three types of actions:

### 1. Continue
Allow operation to proceed normally.

```rust
async fn on_event(&self, ctx: &HookContext) -> Result<HookAction> {
    // Observe event
    eprintln!("Event: {:?}", ctx.event);
    Ok(HookAction::Continue)
}
```

### 2. Abort
Stop operation with an error message.

```rust
async fn on_connection(&self, ctx: &HookContext) -> Result<HookAction> {
    if is_blacklisted(ctx) {
        return Ok(HookAction::Abort("IP blocked".to_string()));
    }
    Ok(HookAction::Continue)
}
```

### 3. ModifyContext (Advanced)
Modify execution context for downstream hooks or operations.

```rust
async fn on_before_tool_call(&self, ctx: &HookContext) -> Result<HookAction> {
    let mut modifications = HashMap::new();
    modifications.insert("injected_param".to_string(), json!("value"));
    Ok(HookAction::ModifyContext(modifications))
}
```

## Hook Execution Order

Hooks execute in registration order:
1. Built-in metrics hook (if enabled)
2. Built-in audit log hook (if enabled)
3. Custom hooks (in registration order)

**Early termination:** If a hook returns `Abort`, subsequent hooks are skipped.

## Performance Considerations

### Memory Impact
- Hooks system: ~2MB baseline
- Metrics hook: ~3MB (Prometheus registry)
- Audit log hook: ~1MB
- Custom hooks: Depends on implementation

**Total overhead: ~6MB**

### Latency Impact
- Hook invocation: < 1ms per event
- Metrics hook: < 0.1ms (counter/gauge updates)
- Audit log hook: < 5ms (async file write)
- Custom hooks: Depends on implementation

**Recommendation:** Keep hook logic lightweight. For expensive operations (HTTP calls, database writes), spawn background tasks.

### Example: Background Task Hook

```rust
async fn on_security_event(&self, ctx: &HookContext) -> Result<HookAction> {
    let ctx_clone = ctx.clone();

    // Spawn background task
    tokio::spawn(async move {
        send_email_alert(&ctx_clone).await;
    });

    // Return immediately
    Ok(HookAction::Continue)
}
```

## Debugging

### Enable Hook Logging

Set `RUST_LOG` environment variable:

```bash
export RUST_LOG=rustyclaw::hooks=debug
rustyclaw gateway start
```

Output:
```
[hooks] Registered metrics hook
[hooks] Registered audit log hook
[hooks] Invoking Connection hook for 192.168.1.100:54321
[hooks] Hook 'metrics' returned Continue
[hooks] Hook 'audit_log' returned Continue
```

### Hook Registry Inspection

```rust
let registry = HookRegistry::new();
// ... register hooks ...

println!("Registered hooks: {:?}", registry.hook_names());
println!("Total hooks: {}", registry.count());
```

## Use Cases

### 1. Security Monitoring
- **Goal:** Real-time intrusion detection
- **Hooks:** `AuthFailure`, `SecurityEvent`
- **Action:** Send alerts to SIEM, block IPs

### 2. Compliance Auditing
- **Goal:** SOC 2 / HIPAA compliance
- **Hooks:** `AuthSuccess`, `BeforeToolCall`, `ConfigReload`
- **Action:** Tamper-proof audit logs

### 3. Usage Analytics
- **Goal:** Track tool usage patterns
- **Hooks:** `AfterToolCall`, `AfterProviderCall`
- **Action:** Export to data warehouse

### 4. Cost Management
- **Goal:** Monitor LLM API costs
- **Hooks:** `AfterProviderCall`
- **Action:** Track tokens, set budget alerts

### 5. Custom Authorization
- **Goal:** IP whitelist, time-based access
- **Hooks:** `Connection`, `BeforeToolCall`
- **Action:** Abort unauthorized operations

## Troubleshooting

### Hooks Not Firing

**Problem:** Events occur but hooks don't execute

**Solutions:**
1. Check `[hooks] enabled = true` in config
2. Verify hook events match: `events(&self) -> &[HookEvent]`
3. Enable debug logging: `RUST_LOG=rustyclaw::hooks=debug`

### Hook Errors Ignored

**Problem:** Hook returns error but gateway continues

**Behavior:** Hook errors are logged but don't crash the gateway. This is intentional for resilience.

**Solution:** Check logs for `[hooks] Error in hook '<name>': <error>`

### Audit Log Not Writing

**Problem:** Audit log hook enabled but no log file

**Solutions:**
1. Check log path permissions: `ls -la ~/.rustyclaw/logs/`
2. Verify parent directory exists: `mkdir -p ~/.rustyclaw/logs`
3. Check hook is registered: Look for "[gateway] Registered audit log hook" on startup

### High Latency

**Problem:** Gateway slow after enabling hooks

**Solutions:**
1. Profile hooks: Add timing logs in hook methods
2. Move expensive operations to background tasks
3. Disable slow hooks: `metrics_hook = false`

## Related

- [Prometheus Metrics](./METRICS.md) — Metrics exposed by metrics hook
- [Security](./SECURITY.md) — Security events that trigger hooks
- [Configuration](../README.md#configuration) — Hook configuration options


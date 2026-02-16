# Configuration Hot-Reload

RustyClaw gateway supports zero-downtime configuration reloading via SIGHUP signal (Unix systems only).

## Overview

When the gateway receives a SIGHUP signal, it:
1. Reloads the configuration file from disk
2. Updates security settings (SSRF, prompt guard, TLS, metrics)
3. Reloads model provider credentials
4. Applies changes to new connections (existing connections continue with old config)

## Usage

### Starting the Gateway

```bash
rustyclaw gateway start
# Output: [gateway] Hot-reload enabled: Send SIGHUP (kill -HUP 12345) to reload config
```

The gateway will print its process ID (PID) on startup.

### Triggering a Reload

**Method 1: Using kill command**
```bash
# Find the gateway PID
ps aux | grep rustyclaw

# Send SIGHUP signal
kill -HUP <PID>
```

**Method 2: Using pkill**
```bash
pkill -HUP rustyclaw
```

**Method 3: If running in foreground**
Press `Ctrl+\` (sends SIGQUIT on most terminals, but you may need to configure it for SIGHUP)

### Verifying the Reload

After sending SIGHUP, check the gateway logs:

```
[gateway] Received SIGHUP signal, reloading configuration...
[gateway] ✓ Configuration reloaded successfully
[gateway]   New settings will apply to new connections
[gateway]   SSRF protection: true -> false
[gateway]   Prompt guard: false -> true
```

## What Gets Reloaded

### Immediately Applied (to all connections)
- Model provider settings
- API keys and credentials
- Provider URLs and endpoints

### Applied to New Connections Only
- Security settings (SSRF, prompt guard)
- TLS configuration
- Metrics endpoint
- Sandbox mode
- Rate limiting settings

### Not Reloaded (requires restart)
- Listen address and port
- WebSocket protocol settings
- Messenger integrations

## Configuration Changes

You can modify any setting in `~/.rustyclaw/config.toml` and reload:

### Example: Enable Security Features

```toml
[ssrf]
enabled = true  # Changed from false

[prompt_guard]
enabled = true  # Changed from false
action = "block"  # Changed from "warn"
sensitivity = 0.8  # Changed from 0.5
```

Then reload:
```bash
kill -HUP $(pgrep rustyclaw)
```

### Example: Switch Model Provider

```toml
[model]
provider = "anthropic"  # Changed from "openai"
model = "claude-3-5-sonnet-20241022"  # New model
# api_key loaded from vault
```

Reload to apply:
```bash
kill -HUP $(pgrep rustyclaw)
```

## Error Handling

If the configuration file has errors, the reload will fail and the gateway will continue using the old configuration:

```
[gateway] Received SIGHUP signal, reloading configuration...
[gateway] ✗ Config reload failed: TOML parse error at line 10
[gateway]   Continuing with current configuration
```

**The gateway never crashes due to configuration errors during reload.**

## Platform Support

- **Linux**: ✅ Full support
- **macOS**: ✅ Full support
- **Windows**: ❌ Not supported (SIGHUP is Unix-only)

On Windows, you must restart the gateway to apply configuration changes.

## Best Practices

### 1. Test Configuration Before Reload

```bash
# Validate config syntax
rustyclaw config validate ~/.rustyclaw/config.toml

# Or use TOML linter
tomlq . ~/.rustyclaw/config.toml
```

### 2. Monitor Reload Success

```bash
# Watch logs during reload
tail -f ~/.rustyclaw/logs/gateway.log &
kill -HUP $(pgrep rustyclaw)
```

### 3. Gradual Rollout

When changing security settings in production:

1. First enable in "warn" mode:
   ```toml
   [prompt_guard]
   enabled = true
   action = "warn"  # Just log, don't block
   ```

2. Monitor logs for false positives

3. Switch to "block" mode after validation:
   ```toml
   [prompt_guard]
   action = "block"  # Now enforce
   ```

### 4. Version Control

Keep your configuration in version control:

```bash
cd ~/.rustyclaw
git init
git add config.toml
git commit -m "Security hardening"

# After reload, verify and commit
git diff
git add config.toml
git commit -m "Enable SSRF protection"
```

## Automation

### Systemd Service with Reload

Create `/etc/systemd/system/rustyclaw.service`:

```ini
[Unit]
Description=RustyClaw AI Gateway
After=network.target

[Service]
Type=simple
User=rustyclaw
ExecStart=/usr/local/bin/rustyclaw gateway start
ExecReload=/bin/kill -HUP $MAINPID
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Then reload with:
```bash
sudo systemctl reload rustyclaw
```

### Automated Config Updates

```bash
#!/bin/bash
# update_config.sh - Safely update and reload RustyClaw config

CONFIG_FILE="$HOME/.rustyclaw/config.toml"
BACKUP_FILE="$CONFIG_FILE.backup"

# Backup current config
cp "$CONFIG_FILE" "$BACKUP_FILE"

# Apply changes (example: enable SSRF protection)
sed -i 's/enabled = false/enabled = true/' "$CONFIG_FILE"

# Validate new config
if rustyclaw config validate "$CONFIG_FILE"; then
    echo "Config valid, reloading gateway..."
    pkill -HUP rustyclaw

    # Wait and verify
    sleep 2
    if pgrep rustyclaw > /dev/null; then
        echo "✓ Reload successful"
        rm "$BACKUP_FILE"
    else
        echo "✗ Gateway crashed, restoring backup"
        mv "$BACKUP_FILE" "$CONFIG_FILE"
        rustyclaw gateway start
    fi
else
    echo "✗ Config validation failed, restoring backup"
    mv "$BACKUP_FILE" "$CONFIG_FILE"
fi
```

## Metrics

Configuration reloads are tracked in Prometheus metrics:

```
# Number of config reload attempts
rustyclaw_config_reloads_total{status="success"}
rustyclaw_config_reloads_total{status="failure"}

# Last reload timestamp
rustyclaw_config_last_reload_timestamp_seconds
```

## Troubleshooting

### Gateway Not Reloading

**Problem**: SIGHUP sent but no reload message in logs

**Solutions**:
1. Verify PID is correct: `ps aux | grep rustyclaw`
2. Check process has permissions: `kill -0 <PID>`
3. Ensure running on Unix system (not Windows)

### Configuration Not Applied

**Problem**: Config reloaded but changes not visible

**Solutions**:
1. Check if change requires restart (see "What Gets Reloaded")
2. Verify correct config file location: `rustyclaw config show`
3. Check for config errors in logs

### Gateway Crashes After Reload

**Problem**: Gateway terminates after SIGHUP

**Cause**: This should never happen (reload errors are caught)

**Action**: Please report as bug with:
- Config file contents
- Gateway logs
- Output of `rustyclaw --version`

## Related

- [Configuration Guide](../README.md#configuration)
- [Security Settings](./SECURITY.md)
- [Prometheus Metrics](./METRICS.md)
- [Deployment Guide](./DEPLOYMENT.md)

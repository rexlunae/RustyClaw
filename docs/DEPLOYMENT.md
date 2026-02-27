# Deployment Guide

This guide covers deploying RustyClaw in production environments.

## Quick Start

```bash
# Install via setup script
curl -fsSL https://raw.githubusercontent.com/rexlunae/RustyClaw/main/scripts/setup.sh | bash

# Or install from crates.io
cargo install rustyclaw

# Run the interactive setup
rustyclaw onboard
```

## Deployment Options

### 1. Interactive TUI (Development/Personal Use)

```bash
rustyclaw tui
```

The TUI provides a full chat interface with model selection, secrets management, and tool approval dialogs.

### 2. Gateway Daemon (Production/Integration)

The gateway runs as a background service, exposing a WebSocket API for clients:

```bash
# Start gateway daemon
rustyclaw gateway start

# Or run in foreground with custom options
rustyclaw gateway run --listen 127.0.0.1:3000

# Check status
rustyclaw gateway status

# Stop daemon
rustyclaw gateway stop
```

**Gateway options:**
| Option | Description | Default |
|--------|-------------|---------|
| `--listen` | Bind address | `127.0.0.1:3000` |
| `--tls-cert` | TLS certificate path | None |
| `--tls-key` | TLS private key path | None |
| `--config` | Config file path | `~/.config/rustyclaw/config.toml` |

### 3. Systemd Service (Linux)

Create `/etc/systemd/system/rustyclaw.service`:

```ini
[Unit]
Description=RustyClaw AI Gateway
After=network.target

[Service]
Type=simple
User=rustyclaw
Group=rustyclaw
WorkingDirectory=/home/rustyclaw
ExecStart=/usr/local/bin/rustyclaw gateway run --listen 127.0.0.1:3000
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/home/rustyclaw/.config/rustyclaw /home/rustyclaw/.local/share/rustyclaw

[Install]
WantedBy=multi-user.target
```

```bash
# Create service user
sudo useradd -r -s /bin/false rustyclaw

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable rustyclaw
sudo systemctl start rustyclaw

# Check logs
journalctl -u rustyclaw -f
```

### 4. Docker Container

```dockerfile
FROM rust:1.85-slim as builder
WORKDIR /app
RUN cargo install rustyclaw
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/rustyclaw /usr/local/bin/
EXPOSE 3000
CMD ["rustyclaw", "gateway", "run", "--listen", "0.0.0.0:3000"]
```

```bash
docker build -t rustyclaw .
docker run -d -p 3000:3000 -v ~/.config/rustyclaw:/root/.config/rustyclaw rustyclaw
```

## Configuration

Config file location: `~/.config/rustyclaw/config.toml`

```toml
# Provider configuration
[provider]
name = "anthropic"          # or "openai", "google", "ollama"
model = "claude-sonnet-4-20250514"

# Workspace settings
[workspace]
dir = "~/.rustyclaw/workspace"

# Sandbox isolation
[sandbox]
mode = "strict"             # "strict", "permissive", or "none"
deny_paths = ["/etc/shadow", "/root/.ssh"]

# Messenger integrations
[telegram]
bot_token = "..."           # From @BotFather

[matrix]
homeserver = "https://matrix.org"
user_id = "@bot:matrix.org"
access_token = "..."

# TLS configuration (for production)
[gateway]
listen = "0.0.0.0:3000"
tls_cert = "/etc/rustyclaw/cert.pem"
tls_key = "/etc/rustyclaw/key.pem"
```

## Secrets Management

RustyClaw includes an encrypted vault for API keys and credentials:

```bash
# Store a secret (interactive prompt for value)
rustyclaw secrets store ANTHROPIC_API_KEY

# List stored secrets
rustyclaw secrets list

# Vault is encrypted with AES-256 at rest
# Optional TOTP 2FA for vault access
rustyclaw secrets enable-totp
```

**Security policies:**
- `Always` — Tool can access without prompting
- `WithApproval` — User must approve each access
- `WithAuth` — Requires TOTP verification
- `SkillOnly` — Only accessible from skills, not direct tool calls

## Monitoring

### Health Endpoints

The gateway exposes health check endpoints:

```bash
# Health check (for load balancers)
curl http://localhost:3000/health

# Detailed status with metrics
curl http://localhost:3000/status

# Prometheus-compatible metrics
curl http://localhost:3000/metrics
```

### Logging

Set log level via environment variable:

```bash
RUST_LOG=debug rustyclaw gateway run
RUST_LOG=rustyclaw=debug,tower_http=info rustyclaw gateway run
```

Log levels: `error`, `warn`, `info`, `debug`, `trace`

### Observability

RustyClaw records events via the Observer trait:
- `LlmRequest` / `LlmResponse` — Provider call telemetry
- `ToolCallStart` / `ToolCall` — Tool execution metrics
- `ChannelMessage` — Messenger activity

Events are emitted to structured logs by default.

## Security Checklist

- [ ] Run as non-root user
- [ ] Enable TLS for production
- [ ] Configure sandbox mode (`strict` recommended)
- [ ] Set up TOTP for vault access
- [ ] Review tool permissions in config
- [ ] Use `deny_paths` to protect sensitive files
- [ ] Enable rate limiting for API endpoints
- [ ] Store secrets in vault, not config files

## Scaling

### Single Instance

For most use cases, a single RustyClaw instance handles:
- Multiple concurrent WebSocket connections
- Messenger polling (Telegram, Matrix, etc.)
- Background task execution
- Scheduled jobs (cron)

### Multi-Instance

For high availability:
1. Run multiple gateway instances behind a load balancer
2. Use sticky sessions (WebSocket affinity)
3. Share config via mounted volume or config management
4. External secrets store (1Password, Vault) via skills

## Troubleshooting

### Gateway won't start

```bash
# Check if port is in use
ss -tlnp | grep 3000

# Check config syntax
rustyclaw config validate

# Run with debug logging
RUST_LOG=debug rustyclaw gateway run
```

### Provider authentication fails

```bash
# Verify secret is stored
rustyclaw secrets list

# Re-store the API key
rustyclaw secrets store ANTHROPIC_API_KEY

# Test provider connectivity
rustyclaw chat --message "hello" --once
```

### Sandbox blocking tools

```bash
# Check sandbox mode
rustyclaw config get sandbox.mode

# Temporarily disable for debugging
rustyclaw config set sandbox.mode none

# Review deny_paths
rustyclaw config get sandbox.deny_paths
```

## Platform-Specific Notes

### Linux
- Landlock LSM support (kernel 5.13+) for additional isolation
- Bubblewrap available for container-like sandboxing

### macOS
- `sandbox-exec` used for isolation
- Gatekeeper may require signing for distribution

### Raspberry Pi / ARM
- Build with `--no-default-features --features web-tools` for minimal footprint
- ~15MB RAM typical usage

## Next Steps

- [Security Model](SECURITY.md) — Deep dive into isolation and vault
- [Sandbox Configuration](SANDBOX.md) — Fine-tune sandbox rules
- [Client Protocol](CLIENT_SPEC.md) — Build custom TUI clients

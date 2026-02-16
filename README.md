# RustyClaw 🦀🦞

**A lightweight, secure agentic AI runtime written in Rust.**

<p align="center">
  <img src="logo.svg" alt="RustyClaw Logo" width="200"/>
</p>

<p align="center">
  <a href="https://crates.io/crates/rustyclaw"><img src="https://img.shields.io/crates/v/rustyclaw.svg" alt="crates.io"></a>
  <a href="https://github.com/rexlunae/RustyClaw/actions"><img src="https://github.com/rexlunae/RustyClaw/workflows/CI/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
  <a href="https://discord.com/invite/clawd"><img src="https://img.shields.io/discord/1234567890?label=discord" alt="Discord"></a>
</p>

RustyClaw is a drop-in Rust implementation of [OpenClaw](https://github.com/openclaw/openclaw) — the agentic AI assistant that lives in your terminal. It brings the same powerful 30-tool ecosystem with improved security, lower memory footprint, and native performance.

## Why RustyClaw?

| Feature | RustyClaw | OpenClaw (Node.js) |
|---------|-----------|-------------------|
| **Memory usage** | ~15 MB | ~150 MB |
| **Startup time** | <50 ms | ~500 ms |
| **Binary size** | ~8 MB | ~200 MB (with node) |
| **Sandbox modes** | 6 (Landlock+bwrap/Docker/macOS/etc.) | External only |
| **Defense-in-depth** | ✅ Combined kernel + namespace | ❌ |
| **Container isolation** | ✅ Docker with resource limits | ❌ |
| **Secrets vault** | AES-256-GCM + TOTP + WebAuthn | External (1Password, etc.) |
| **Language** | Rust 🦀 | TypeScript |

### Security-First Design 🔒

RustyClaw was built with the assumption that **AI agents can't always be trusted**. The multi-layer security model includes:

#### Defense-in-Depth Sandboxing

RustyClaw offers **6 sandbox modes** with automatic fallback for maximum security:

1. **Landlock+Bubblewrap** (Linux) — Combined kernel LSM + namespace isolation for defense-in-depth
2. **Landlock** (Linux 5.13+) — Kernel-enforced filesystem access control
3. **Bubblewrap** (Linux) — User namespace isolation with mount/network restrictions
4. **Docker** (Cross-platform) — Container isolation with resource limits (2GB memory, 1 CPU)
5. **macOS Sandbox** (macOS) — Apple's sandbox-exec with TinyScheme profiles
6. **Path Validation** (Fallback) — Allowlist-based path checking

The sandbox automatically selects the strongest available mode for your platform.

👉 **[Sandbox Documentation →](docs/SANDBOX.md)**

#### Secrets Management

- **Encrypted secrets vault** — AES-256-GCM encryption for API keys, credentials, SSH keys
- **TOTP two-factor authentication** — Optional 2FA for vault access with recovery codes
- **Per-credential access policies** — Always, WithApproval, WithAuth, SkillOnly
- **Credentials directory protection** — Sandboxed tools cannot access `~/.rustyclaw/secrets/`
- **WebAuthn support** — Hardware key authentication (YubiKey, etc.)

👉 **[Complete Security Model →](docs/SECURITY.md)**

## Quick Start

### Install from crates.io

```bash
cargo install rustyclaw
```

### With optional features

```bash
# Matrix messenger support
cargo install rustyclaw --features matrix

# Browser automation (CDP)
cargo install rustyclaw --features browser

# All publishable features
cargo install rustyclaw --features full
```

> 📝 **Signal messenger** requires building from source. See [BUILDING.md](BUILDING.md).

### Or build from source

```bash
git clone https://github.com/rexlunae/RustyClaw.git
cd RustyClaw
cargo build --release
```

### Run the interactive setup

```bash
rustyclaw onboard
```

### Start chatting

```bash
rustyclaw tui
```

## Recent Enhancements ✨

**February 2026** — Major security and architecture improvements:

- 🛡️ **Defense-in-Depth Sandboxing** — Combined Landlock+Bubblewrap mode for kernel-enforced + namespace isolation
- 🐳 **Docker Container Support** — Cross-platform sandboxing with Alpine Linux, resource limits, and credential injection
- 📋 **Development Roadmap** — 3-phase plan covering multi-provider failover, WASM sandboxing, hybrid search, and more
- 🔧 **15 Feature Issues Created** — [View all planned features](https://github.com/aecs4u/RustyClaw/issues?q=is%3Aissue+is%3Aopen+label%3Aenhancement) (#51-#65)
- 💬 **Messenger Integrations** — Slack, Discord, Telegram, Matrix support with dedicated branches

See [ROADMAP.md](ROADMAP.md) for the complete development plan and feature analysis based on IronClaw review.

## Features

### 30 Agentic Tools

RustyClaw implements the complete OpenClaw tool ecosystem:

| Category | Tools |
|----------|-------|
| **File Operations** | `read_file`, `write_file`, `edit_file`, `list_directory`, `search_files`, `find_files` |
| **Code Execution** | `execute_command`, `process`, `apply_patch` |
| **Web Access** | `web_fetch`, `web_search` |
| **Memory** | `memory_search`, `memory_get` |
| **Scheduling** | `cron` |
| **Multi-Agent** | `sessions_list`, `sessions_spawn`, `sessions_send`, `sessions_history`, `session_status`, `agents_list` |
| **Secrets** | `secrets_list`, `secrets_get`, `secrets_store` |
| **System** | `gateway`, `message`, `tts` |
| **Devices** | `browser`, `canvas`, `nodes`, `image` |

### Skills System

Load skills from the [OpenClaw ecosystem](https://clawhub.com) or write your own:

```markdown
---
name: my-skill
description: A custom skill
metadata: {"openclaw": {"requires": {"bins": ["git"]}}}
---

# Instructions for the agent

Do something useful with git.
```

Skills support **gating** — require binaries, environment variables, or specific operating systems.

### Multi-Provider Support

Connect to any major AI provider:

- **Anthropic** (Claude 4, Claude Sonnet)
- **OpenAI** (GPT-4, GPT-4o)
- **Google** (Gemini Pro, Gemini Ultra)
- **GitHub Copilot** (with subscription)
- **xAI** (Grok)
- **Ollama** (local models)
- **OpenRouter** (any model)

### Terminal UI

A beautiful TUI with:

- Syntax-highlighted code blocks
- Markdown rendering
- Tab completion
- Slash commands (`/help`, `/clear`, `/model`, `/secrets`)
- Streaming responses

### Gateway Mode

Run as a daemon for integration with other tools:

```bash
rustyclaw gateway start
```

### Messenger Integrations 💬

RustyClaw can be integrated with multiple messaging platforms, making your AI assistant accessible wherever your team communicates:

| Platform | Status | Setup |
|----------|--------|-------|
| **Slack** | ✅ Available | [Quick Start](docs/MESSENGER_SLACK.md) |
| **Discord** | ✅ Available | [Quick Start](docs/MESSENGER_DISCORD.md) |
| **Telegram** | ✅ Available | [Quick Start](docs/MESSENGER_TELEGRAM.md) |
| **Matrix** | ✅ Available | [Quick Start](docs/MESSENGER_MATRIX.md) |

Each messenger integration is available on its own feature branch for easy testing and deployment:

```bash
# Checkout and test Slack integration
git checkout feature/messenger-slack
cargo build --features messenger-slack
rustyclaw gateway start

# Or try Discord
git checkout feature/messenger-discord
cargo build --features messenger-discord
```

**Learn more**: [Messenger Integrations Overview](docs/MESSENGERS.md)

Supports WebSocket connections, heartbeats, and multi-session management.

## Configuration

Configuration lives at `~/.rustyclaw/config.toml`:

```toml
settings_dir = "/Users/myuser/.rustyclaw"
messengers = []
use_secrets = true
secrets_password_protected = true
totp_enabled = true
agent_access = false
agent_name = "A Rusty Little Crab"
message_spacing = 1
tab_width = 5

[model]
provider = "openrouter"
model = "gpt-4.1"
base_url = "https://openrouter.ai/api/v1"

[sandbox]
# Mode: "auto" (default), "landlock+bwrap", "docker", "landlock", "bwrap", "macos", "path", "none"
mode = "auto"
deny_paths = ["/etc/passwd", "/etc/shadow"]
allow_paths = ["/tmp", "/var/tmp"]

# Docker-specific settings (when mode = "docker")
docker_image = "alpine:latest"
docker_memory_limit_mb = 2048
docker_cpu_shares = 1024
```

See [docs/SANDBOX.md](docs/SANDBOX.md) for detailed sandbox configuration options.

## Documentation

### Getting Started
- **[Building](BUILDING.md)** — Feature flags, Signal support, cross-compilation
- **[Getting Started](docs/getting-started.md)** — Installation and first run
- **[Configuration](docs/configuration.md)** — Settings and environment setup

### Security & Architecture
- **[Security Model](docs/SECURITY.md)** — Comprehensive security architecture
- **[Sandbox Modes](docs/SANDBOX.md)** — 6 sandbox isolation strategies explained
- **[Development Roadmap](ROADMAP.md)** — 3-phase feature plan with 15+ enhancements

### Features & Integration
- **[Tools Reference](docs/tools.md)** — All 30 agentic tools explained
- **[Skills Guide](docs/skills.md)** — Writing and using skills
- **[Gateway Protocol](docs/gateway.md)** — WebSocket API reference
- **[Messenger Integrations](docs/MESSENGERS.md)** — Slack, Discord, Telegram, Matrix setup

## Testing

RustyClaw has comprehensive test coverage:

```bash
# Run all tests (330+)
cargo test

# Run specific test suites
cargo test --test tool_execution
cargo test --test gateway_protocol
cargo test --test skill_execution
```

## Community

- 💬 [Discord](https://discord.com/invite/clawd) — Join the OpenClaw community
- 🐛 [Issues](https://github.com/rexlunae/RustyClaw/issues) — Bug reports and feature requests
- 🔧 [ClawhHub](https://clawhub.com) — Find and share skills

## Contributing

Contributions welcome! We have a detailed roadmap and active development:

- 📋 **[Development Roadmap](ROADMAP.md)** — 3-phase plan with 15+ planned features
- 🐛 **[Open Issues](https://github.com/aecs4u/RustyClaw/issues)** — Bug reports, feature requests, and tasks
- 🏷️ **Good First Issues** — Look for `p3-medium` and `quick-win` labels (#51-#65)
- 📖 **[Contributing Guide](CONTRIBUTING.md)** — Development guidelines and PR process

**High-priority features from the roadmap:**
- Multi-provider failover (Phase 1) — #52
- Safety layer consolidation (Phase 1) — #53
- WASM sandbox (Phase 3) — #60
- Hybrid search with BM25+Vector (Phase 2) — #56

See individual issues for implementation details and acceptance criteria.

## License

MIT License — See [LICENSE](LICENSE) for details.

## Acknowledgments

- [OpenClaw](https://github.com/openclaw/openclaw) — The original project and inspiration
- The Rust community for excellent crates

---

<p align="center">
  <i>Built with 🦀 by the RustyClaw contributors</i>
</p>

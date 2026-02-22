<p align="center">
  <img src="logo.svg" alt="RustyClaw" width="200"/>
</p>

<h1 align="center">RustyClaw 🦀🦞</h1>

<p align="center">
  <strong>The secure, open-source operating system for AI agents</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/rustyclaw"><img src="https://img.shields.io/crates/v/rustyclaw.svg" alt="crates.io"></a>
  <a href="https://github.com/rexlunae/RustyClaw/actions"><img src="https://github.com/rexlunae/RustyClaw/workflows/CI/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
  <a href="https://discord.com/invite/clawd"><img src="https://img.shields.io/badge/Discord-Community-5865F2?logo=discord&logoColor=white" alt="Discord"></a>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="#why-rustyclaw">Why RustyClaw</a> •
  <a href="#features">Features</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a>
</p>

---

## What is RustyClaw?

RustyClaw is an **agentic AI operating system** — a complete runtime for deploying, orchestrating, and securing AI agents. It provides everything agents need: tools, memory, isolation, scheduling, multi-agent coordination, and secure credential management.

Think of it as **Linux for AI agents**: a stable, secure foundation that handles the hard infrastructure problems so you can focus on what your agents actually do.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         YOUR AI AGENTS                              │
├─────────────────────────────────────────────────────────────────────┤
│  Tools     │  Memory    │  Channels  │  Sessions  │  Scheduling    │
│  (30+)     │  (files,   │  (Signal,  │  (spawn,   │  (cron,        │
│            │   search)  │   Matrix)  │   steer)   │   heartbeat)   │
├─────────────────────────────────────────────────────────────────────┤
│                    SECURITY & ISOLATION LAYER                       │
│   PromptGuard · LeakDetector · Sandbox · Encrypted Vault · SSRF    │
├─────────────────────────────────────────────────────────────────────┤
│                     RUSTYCLAW RUNTIME (Rust)                        │
│            ~15MB RAM · <50ms startup · Single binary                │
└─────────────────────────────────────────────────────────────────────┘
```

## Why RustyClaw?

### 🔒 Security-First Design

AI agents are powerful but risky. They can be tricked into leaking secrets, executing malicious commands, or exfiltrating data. RustyClaw is built with the assumption that **agents can't always be trusted**.

| Defense Layer | What It Does |
|---------------|--------------|
| **PromptGuard** | Detects prompt injection attacks (system override, role confusion, jailbreaks) |
| **LeakDetector** | Blocks credential exfiltration via API keys, tokens, SSH keys in outputs |
| **Sandbox Isolation** | Bubblewrap (Linux), Landlock (5.13+), sandbox-exec (macOS) |
| **SSRF Protection** | Blocks requests to private IPs, metadata endpoints |
| **Encrypted Vault** | AES-256 secrets with optional TOTP 2FA |
| **HTTP Request Scanning** | Validates URLs, headers, and bodies before outbound requests |

No other agent framework in the ecosystem has this level of built-in security. Most have **zero prompt injection defense**.

### ⚡ Lightweight & Fast

| Metric | RustyClaw | OpenClaw (Node.js) | Python Agents |
|--------|-----------|-------------------|---------------|
| **Memory** | ~15 MB | ~150 MB | ~100+ MB |
| **Startup** | <50 ms | ~500 ms | ~1s+ |
| **Binary** | ~8 MB | ~200 MB (w/ node) | N/A |
| **Dependencies** | 0 (single binary) | node_modules | venv |

Run on a $10 Raspberry Pi or a $500/month cloud instance. Same binary.

### 🔌 Provider Agnostic

Connect to any LLM provider without code changes:

- **Anthropic** (Claude Opus, Sonnet, Haiku)
- **OpenAI** (GPT-4o, o1, o3)
- **Google** (Gemini Pro, Ultra)
- **GitHub Copilot** (with subscription)
- **xAI** (Grok)
- **Ollama** (local models)
- **OpenRouter** (200+ models)
- **Any OpenAI-compatible endpoint**

### 🤖 Multi-Agent Orchestration

Spawn sub-agents, steer them mid-task, coordinate across sessions:

```rust
// Spawn a research agent
let research = spawn_agent("Summarize the latest papers on RLHF", AgentConfig {
    model: "claude-sonnet",
    timeout: Duration::minutes(10),
    ..default()
});

// Spawn a coding agent in parallel
let coder = spawn_agent("Implement the algorithm from the research", AgentConfig {
    model: "gpt-4o",
    ..default()
});

// Steer mid-execution
research.steer("Focus specifically on Constitutional AI approaches");
```

## Quick Start

### Install

```bash
cargo install rustyclaw
```

Or download a pre-built binary from [Releases](https://github.com/rexlunae/RustyClaw/releases).

### Configure

```bash
rustyclaw onboard
```

This interactive wizard sets up:
- API key for your preferred provider
- Encrypted secrets vault
- Workspace directory
- Channel connections (optional)

### Run

```bash
# Interactive terminal UI
rustyclaw tui

# Or run as a daemon for integrations
rustyclaw gateway start
```

## Features

### 🛠️ 30+ Agentic Tools

Everything an agent needs to be useful:

| Category | Tools |
|----------|-------|
| **Files** | `read_file`, `write_file`, `edit_file`, `list_directory`, `search_files` |
| **Execution** | `execute_command`, `process`, `apply_patch` |
| **Web** | `web_fetch`, `web_search`, `browser` |
| **Memory** | `memory_search`, `memory_get` |
| **Scheduling** | `cron`, heartbeat system |
| **Multi-Agent** | `sessions_spawn`, `sessions_send`, `sessions_steer` |
| **Secrets** | `secrets_list`, `secrets_get`, `secrets_store` |
| **Devices** | `canvas`, `nodes`, `tts` |

### 📚 Skills System

Extend capabilities with skills — markdown files that teach agents new abilities:

```yaml
---
name: github
description: GitHub operations via gh CLI
requires:
  bins: [gh]
  env: [GITHUB_TOKEN]
---

# GitHub Skill

You can use the `gh` CLI to manage issues, PRs, and repos...
```

Skills support **dependency gating**: if requirements aren't met, the agent sees what's missing and can try to install it.

Browse community skills at [ClawHub](https://clawhub.com).

### 💬 Multi-Channel Support

Connect agents to the platforms where work happens:

- **Signal** (secure messaging)
- **Matrix** (federated chat)
- **Telegram** (bot API)
- **Discord** (bot API)
- **Slack** (with app tokens)
- **WhatsApp** (QR code pairing)
- **HTTP webhooks** (custom integrations)

### 🧠 Memory & Context

Two-layer memory system for long-running agents:

- **MEMORY.md** — Long-term facts (LLM-curated)
- **HISTORY.md** — Grep-searchable event log

Memory consolidation runs automatically, keeping context windows manageable while preserving important information.

### ⏰ Scheduling & Automation

Built-in cron system for recurring tasks:

```json
{
  "schedule": { "kind": "cron", "expr": "0 9 * * MON" },
  "payload": {
    "kind": "agentTurn",
    "message": "Check email and summarize anything urgent"
  }
}
```

Heartbeat system for proactive monitoring without explicit schedules.

## Security

RustyClaw's security model is documented in detail:

- **[SECURITY.md](docs/SECURITY.md)** — Full security architecture
- **[THREAT_MODEL.md](docs/THREAT_MODEL.md)** — Known threats and mitigations
- **[AUDIT.md](docs/AUDIT.md)** — Audit log and findings

### Quick Overview

```
User Input
    │
    ▼
┌───────────────┐
│ InputValidator│ ─── Length, encoding, padding attacks
└───────┬───────┘
        │
        ▼
┌───────────────┐
│  PromptGuard  │ ─── 6 injection categories, configurable sensitivity
└───────┬───────┘
        │
        ▼
┌───────────────┐
│    Agent      │ ─── Sandboxed execution
└───────┬───────┘
        │
        ▼
┌───────────────┐
│ LeakDetector  │ ─── Blocks secrets in outputs/requests
└───────┬───────┘
        │
        ▼
┌───────────────┐
│ SSRF Validator│ ─── Blocks private IPs, metadata endpoints
└───────────────┘
```

### Encrypted Secrets Vault

API keys, tokens, and credentials are stored encrypted:

- AES-256-GCM encryption
- Optional TOTP 2FA for vault access
- Per-credential access policies (Always, WithApproval, WithAuth, SkillOnly)
- Agent tools cannot read the vault directory

## Architecture

RustyClaw follows a **trait-driven architecture** — core systems are pluggable:

```rust
// Swap providers without changing agent code
trait LlmProvider {
    async fn chat(&self, messages: &[Message]) -> Response;
}

// Swap channels without changing agent code
trait Channel {
    async fn receive(&self) -> InboundMessage;
    async fn send(&self, msg: OutboundMessage);
}

// Swap runtimes for different isolation levels
trait RuntimeAdapter {
    async fn execute(&self, command: Command) -> Output;
}
```

### Core Components

| Component | Responsibility |
|-----------|----------------|
| **Gateway** | Daemon process, session management, heartbeats |
| **Agent Loop** | LLM calls, tool execution, context management |
| **Tool Registry** | Dynamic tool registration, validation, execution |
| **Session Manager** | Multi-agent coordination, history, spawn/steer |
| **Security Layer** | PromptGuard, LeakDetector, SSRF, sandbox |
| **Secrets Vault** | Encrypted credential storage, access policies |

## Comparison

| Feature | RustyClaw | OpenClaw | ZeroClaw | nanobot |
|---------|-----------|----------|----------|---------|
| **Language** | Rust | TypeScript | Rust | Python |
| **Memory** | ~15 MB | ~150 MB | <5 MB | ~100 MB |
| **Startup** | <50 ms | ~500 ms | <10 ms | ~1s |
| **PromptGuard** | ✅ | ❌ | ❌ | ❌ |
| **LeakDetector** | ✅ | ❌ | ❌ | ❌ |
| **Encrypted Vault** | ✅ | External | ✅ | ❌ |
| **Multi-Agent** | ✅ | ✅ | ✅ | ✅ |
| **Skills** | ✅ | ✅ | ✅ | ✅ |

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

Key areas we're focused on:

- **Security hardening** — More detection patterns, sandbox improvements
- **New channels** — iMessage, Teams, Zulip
- **Performance** — Even lower memory, faster startup
- **Skills ecosystem** — More community skills

## License

MIT License. See [LICENSE](LICENSE).

## Acknowledgments

RustyClaw builds on ideas from:

- [OpenClaw](https://github.com/openclaw/openclaw) — The original agentic AI assistant
- [IronClaw](https://github.com/nearai/ironclaw) — Security patterns and HTTP scanning
- [nanobot](https://github.com/HKUDS/nanobot) — Memory consolidation and progressive skill loading
- [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) — RuntimeAdapter and observability patterns

---

<p align="center">
  <strong>Built with 🦀 by the RustyClaw community</strong>
</p>

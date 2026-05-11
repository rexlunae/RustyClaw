---
name: testing-rustyclaw-cli
description: Test RustyClaw CLI commands end-to-end. Use when verifying CLI changes, swarm commands, or new subcommands.
---

# Testing RustyClaw CLI

## Build

The CLI binary is in the `rustyclaw` package (crate path: `crates/rustyclaw-cli`).

```bash
# Minimal build (TUI only, no desktop/GPU deps):
cargo build -p rustyclaw --no-default-features --features tui

# Binary location:
target/debug/rustyclaw
```

**Feature flags:**
- `tui` — terminal UI (ratatui). Use this for CLI-only testing.
- `desktop` — Dioxus desktop UI. Requires GTK/webkit libs; skip on headless VMs.
- Default features include both; use `--no-default-features --features tui` to avoid desktop build deps.

## Swarm Commands

The `swarm` subcommand manages multi-agent orchestration:

```bash
rustyclaw swarm templates    # List available swarm templates
rustyclaw swarm list         # List active swarms (empty by default)
rustyclaw swarm create [TEMPLATE]  # Create swarm (default: openswarm)
rustyclaw swarm send <SWARM> <MESSAGE>  # Send message to swarm
rustyclaw swarm status <SWARM>   # Show swarm status
rustyclaw swarm stop <SWARM>     # Stop a swarm
```

## Architectural Notes

- **In-memory state:** The swarm manager uses `OnceLock<Arc<Mutex<SwarmManager>>>`. State does NOT persist across CLI invocations. Each command runs in its own process.
- **Multi-step lifecycle testing:** You cannot create a swarm in one command and send to it in another. Test each command independently, or test lifecycle within the TUI/desktop (long-running process).
- **Error handling:** Invalid templates exit 1 with "Unknown template"; non-existent swarms exit 1 with "not found"; empty messages exit 1 with "Message cannot be empty".

## Test Strategy

1. **Happy paths:** `templates` (8 agents listed), `create openswarm` (success + agent list), `list` (empty state message)
2. **Error paths:** `create nonexistent` (exit 1), `send nonexistent "msg"` (exit 1), `stop nonexistent` (exit 1), `send openswarm` with no message (exit 1)
3. **Help text:** `--help` for each subcommand — verify args, defaults, options
4. **Desktop UI:** SwarmPanel compiles but requires Dioxus desktop runtime. Verify via `cargo check -p rustyclaw-desktop` on CI.

## CI Notes

- Linux x86_64 checks (default, no-default, full features) and aarch64-apple-darwin should pass.
- Cross-compilation targets (ort-sys on macOS x86_64, openssl-sys on ARM) may have pre-existing failures unrelated to swarm changes.

## Devin Secrets Needed

No secrets required for CLI testing. LLM tool integration testing would require an API provider key (e.g., `OPENAI_API_KEY`), but this is not needed for basic CLI verification.

---
name: testing-rustyclaw-cli
description: Test RustyClaw CLI commands end-to-end. Use when verifying CLI changes, swarm commands, new subcommands, or gateway error handling.
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

## Unit Tests

```bash
# Full test suite:
cargo test -p rustyclaw-core

# Focused error module tests:
cargo test -p rustyclaw-core -- gateway::errors::tests --nocapture

# Clippy (treat warnings as errors):
cargo clippy -p rustyclaw-core -- -D warnings -A clippy::needless_option_as_deref
```

## Swarm Commands

The `swarm` subcommand manages multi-agent orchestration:

```bash
rustyclaw swarm templates    # List available swarm templates
rustyclaw swarm list         # List active swarms (empty by default)
rustyclaw swarm create [TEMPLATE]  # Create swarm (default: swarm)
rustyclaw swarm send <SWARM> <MESSAGE>  # Send message to swarm
rustyclaw swarm status <SWARM>   # Show swarm status
rustyclaw swarm stop <SWARM>     # Stop a swarm
```

## Gateway Error Handling Architecture

The gateway error system lives in `crates/rustyclaw-core/src/gateway/errors.rs`.

### GatewayError Enum

All gateway errors are unified into a `GatewayError` enum with variants:
- `Auth { provider, message }` — authentication failures (401/403)
- `Provider { message }` — generic model provider errors
- `TokenLimit` — context window exceeded
- `ToolLoopExhausted { rounds }` — tool call loop limit
- `ContextCompaction { message }` — non-fatal compaction failure
- `Cancelled` — user cancellation
- `Vault { message }` — vault/secret storage errors
- `DeviceFlow { message }` — OAuth device flow failures
- `Config { message }` — configuration errors
- `TokenRefresh { message }` — OAuth token refresh failures
- `UnexpectedFinish { reason }` — model finished with unusual finish_reason

### Frame Type Routing (Critical)

Different error variants route through different frame types:
- **`send_info`**: `UnexpectedFinish`, `ContextCompaction` (non-fatal/informational)
- **`send_error`**: `Provider`, `Vault`, `TokenRefresh` (actual errors)
- **Special handling**: `Auth` triggers credential request or device flow, `DeviceFlow` triggers device flow dialog

**Important**: The TUI treats `ServerFrameType::Info` and `ServerFrameType::Error` differently — Info frames go through `GwEvent::Info()` while Error frames go through `GwEvent::error()`. Changing the frame type is a client-visible behavioral change.

### Device Flow Authentication

Providers using OAuth device flow (e.g., GitHub Copilot) have `auth_method == DeviceFlow`. When these providers encounter auth errors OR token refresh failures:
1. Gateway calls `handle_device_flow()` which calls `start_device_flow()`
2. A `DeviceFlowStart` frame is sent to the TUI with verification URL + user code
3. TUI shows `DeviceFlowDialog` component for user to complete OAuth flow
4. Gateway polls for token completion

API key providers get the `CredentialRequest` dialog instead.

### Testing Error Handling Changes

When verifying error handling changes without a live gateway:

1. **Unit tests**: Run `cargo test -p rustyclaw-core -- gateway::errors::tests` to verify:
   - `ErrorKind` string tags (`as_str()` method)
   - `Display` impl output for each variant
   - `is_non_fatal()` classification
   - `into_traced()` carries correct anyhow-tracing fields

2. **Code contract verification**: Grep to confirm routing:
   ```bash
   # Verify which handler uses send_info vs send_error:
   grep -n 'send_info\|send_error' crates/rustyclaw-core/src/gateway/errors.rs
   
   # Verify call site uses correct variant:
   grep -n 'GatewayError::' crates/rustyclaw-core/src/gateway/mod.rs
   
   # Verify device flow is called from both Auth and TokenRefresh handlers:
   grep -n 'handle_device_flow' crates/rustyclaw-core/src/gateway/errors.rs
   ```

3. **Build verification**: TUI build confirms all call sites compile:
   ```bash
   cargo build -p rustyclaw --no-default-features --features tui
   ```

### Borrow Checker Patterns

The `handle()` function takes `&mut ResolvedModel`. When accessing fields like `resolved.provider` for lookups before passing `&mut resolved` to a sub-handler, clone the field first to avoid borrow conflicts:
```rust
let provider_id = resolved.provider.clone();
let secret_name = provider_by_id(&provider_id)...;
handle_device_flow(writer, resolved, ...)  // now safe to pass &mut resolved
```

## Architectural Notes

- **In-memory state:** The swarm manager uses `OnceLock<Arc<Mutex<SwarmManager>>>`. State does NOT persist across CLI invocations. Each command runs in its own process.
- **Multi-step lifecycle testing:** You cannot create a swarm in one command and send to it in another. Test each command independently, or test lifecycle within the TUI/desktop (long-running process).
- **Error handling:** Invalid templates exit 1 with "Unknown template"; non-existent swarms exit 1 with "not found"; empty messages exit 1 with "Message cannot be empty".

## Test Strategy

1. **Happy paths:** `templates` (8 agents listed), `create swarm` (success + agent list), `list` (empty state message)
2. **Error paths:** `create nonexistent` (exit 1), `send nonexistent "msg"` (exit 1), `stop nonexistent` (exit 1), `send swarm` with no message (exit 1)
3. **Help text:** `--help` for each subcommand — verify args, defaults, options
4. **Gateway errors:** Run focused error module tests, verify frame type routing via grep, build TUI binary
5. **Desktop UI:** SwarmPanel compiles but requires Dioxus desktop runtime. Verify via `cargo check -p rustyclaw-desktop` on CI.

## CI Notes

- Linux x86_64 checks (default, no-default, full features) and aarch64-apple-darwin should pass.
- Cross-compilation targets (ort-sys on macOS x86_64, openssl-sys on ARM) may have pre-existing failures unrelated to changes.
- Expect 14 CI checks total. Lint typically completes first.

## Devin Secrets Needed

No secrets required for CLI testing or gateway error module testing. Full E2E testing of the device flow would require a running gateway with a GitHub Copilot OAuth token, but unit-level and structural verification works without credentials.

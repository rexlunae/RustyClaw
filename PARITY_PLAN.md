# RustyClaw ↔ OpenClaw Parity Plan

## Current State (RustyClaw)

### ✅ Implemented Tools (30 total)
1. `read_file` — read file contents with line ranges; auto-extracts text from .docx/.doc/.rtf/.pdf via textutil
2. `write_file` — create/overwrite files
3. `edit_file` — search-and-replace edits
4. `list_directory` — list directory contents
5. `search_files` — grep-like content search (case-insensitive)
6. `find_files` — find files by name/glob (keyword mode + glob mode, case-insensitive)
7. `execute_command` — run shell commands (with timeout, background support)
8. `web_fetch` — fetch URL and extract readable text
9. `web_search` — search the web via Brave Search API
10. `process` — background process management (list, poll, log, write, kill)
11. `memory_search` — BM25 keyword search over MEMORY.md + memory/*.md
12. `memory_get` — snippet retrieval with line ranges
13. `cron` — scheduled job management (at, every, cron expressions)
14. `sessions_list` — list active sessions with filters
15. `sessions_spawn` — spawn sub-agent background tasks
16. `sessions_send` — send messages to other sessions
17. `sessions_history` — fetch session message history
18. `session_status` — usage/cost tracking and session info
19. `agents_list` — list available agents for spawning
20. `apply_patch` — multi-hunk unified diff patches
21. `secrets_list` — list secrets from encrypted vault
22. `secrets_get` — retrieve secret by key
23. `secrets_store` — store/update encrypted secret
24. `gateway` — config get/apply/patch, restart, update
25. `message` — cross-platform messaging (send, broadcast)
26. `tts` — text-to-speech conversion (functional with API key, graceful fallback without)
27. `image` — vision model image analysis (functional with OpenAI/Anthropic/Google API keys)
28. `nodes` — paired device discovery and control (SSH/ADB backends)
29. `browser` — web browser automation (real CDP with `browser` feature; stub without)
30. `canvas` — node canvas UI presentation (stub — requires canvas integration)

### ✅ Implemented Features
- Multi-provider support (OpenAI, Anthropic, Google, GitHub Copilot, xAI, OpenRouter, Ollama, custom)
- Tool-calling loop (up to 25 rounds)
- Context compaction (auto-summarize at 75% of model context window)
- Token usage extraction from all providers (OpenAI, Anthropic, Google)
- Model context window lookup table (per-model token limits)
- TOTP 2FA authentication with rate limiting and lockout
- Secrets vault with typed credentials and access policies
- TUI interface with slash-commands and tab-completion
- Skills loading (JSON/YAML definitions) with enable/disable
- SOUL.md personality system
- Conversation history persistence (cross-session memory, startup replay)
- WebSocket gateway architecture with ping/pong heartbeat
- Gateway daemon management (spawn, PID tracking, restart, kill)
- Config migration from legacy flat layout
- CLI commands: setup, gateway, configure, secrets, doctor, tui, command, status, version, skill
- Messenger backends: Webhook, Console, Discord, Telegram, Signal (optional)

---

## Phase 0 — Discovery & Baseline

| Task | Status | Notes |
|------|--------|-------|
| Capture OpenClaw CLI help output and flag list | ✅ Done | CLI commands aligned: setup, gateway, configure, secrets, doctor, tui, command, status, version, skill |
| Capture OpenClaw config schema and default paths | ✅ Done | Config schema implemented in config.rs, matching OpenClaw layout |
| Capture OpenClaw gateway/WebSocket protocol | ✅ Done | Handshake, message types (chat, chunk, response_done, tool_call, tool_result, error, info, status, auth_*), ping/pong |
| Capture OpenClaw skills format and runtime behavior | ✅ Done | JSON/TOML/YAML/YML skill loading implemented |
| Capture OpenClaw messenger integrations and config requirements | ✅ Done | Trait + 5 backends (Webhook, Console, Discord, Telegram, Signal) |
| Capture OpenClaw TUI screens, commands, and shortcuts | ✅ Done | 12+ slash-commands, tab-completion, pane navigation |
| Capture OpenClaw secrets approval/permissions flow | ✅ Done | Full policy enforcement (Always/WithAuth/SkillOnly), TOTP, lockout |
| Build a parity matrix mapping features to RustyClaw coverage | ✅ Done | This document |

## Phase 1 — CLI Parity

| Task | Status | Notes |
|------|--------|-------|
| Align top-level commands/subcommands with OpenClaw | ✅ Done | setup, gateway, configure, secrets, doctor, tui, command, status, version, skill |
| Align CLI flags and env vars | ⚠️ Partial | Core flags present, env var precedence not fully audited |
| Match exit codes and error formatting | ✅ Done | tests/exit_codes.rs |
| Add CLI conformance tests (golden help output + behavior) | ✅ Done | tests/cli_conformance.rs, tests/golden_files.rs |

## Phase 2 — Gateway Parity

| Task | Status | Notes |
|------|--------|-------|
| Implement OpenClaw handshake and auth requirements | ✅ Done | TOTP challenge/response, rate limiting, lockout |
| Implement OpenClaw message types, streaming, and errors | ✅ Done | All message types + OpenAI/Anthropic SSE streaming |
| Implement ping/pong or keepalive rules | ✅ Done | WebSocket ping→pong handler |
| Add gateway compliance tests and fixtures | ✅ Done | tests/gateway_protocol.rs |

## Phase 3 — Skills Parity

| Task | Status | Notes |
|------|--------|-------|
| Implement OpenClaw skill metadata schema and validation | ✅ Done | JSON/TOML/YAML/YML support |
| Match skill discovery rules (paths, recursion, file types) | ✅ Done | Walks skills_dir recursively |
| Implement skill execution model (I/O, timeouts, concurrency) | ✅ Done | Full gating + prompt injection |
| Match error reporting and logging for skill failures | ✅ Done | Gate check results with missing items |

## Phase 4 — Messenger Parity

| Task | Status | Notes |
|------|--------|-------|
| Implement required messenger interfaces and config fields | ✅ Done | Full trait + 5 backends |
| Match connection lifecycle, retries, and message formatting | ✅ Done | Webhook, Console, Discord, Telegram, Signal backends |
| Match inbound/outbound event handling | ✅ Done | send_message + receive_messages trait methods |
| Add WhatsApp and Slack messenger backends | ⚠️ Missing | OpenClaw supports WhatsApp and Slack; RustyClaw does not |

## Phase 5 — TUI Parity

| Task | Status | Notes |
|------|--------|-------|
| Match TUI views, navigation, and shortcuts | ✅ Done | Pane navigation, ESC/TAB, scrolling |
| Match available commands and help text | ✅ Done | /help, /clear, /provider, /model, /gateway, /secrets, /quit, etc. |
| Match log view formatting and session state | ⚠️ Partial | Messages pane with roles; no dedicated log view |

## Phase 6 — Secrets Parity

| Task | Status | Notes |
|------|--------|-------|
| Match secrets storage backends and key namespaces | ✅ Done | Typed credentials (API key, SSH key, password, secure note, payment, form, passkey) |
| Match approval/consent flows and caching rules | ✅ Done | Policy enforcement (Always/WithAuth/SkillOnly), agent access control |
| Add migration support for existing OpenClaw secrets | ⚠️ Partial | Legacy flat-layout migration exists; cross-tool secret import not tested |

## Phase 7 — Config & Migration

| Task | Status | Notes |
|------|--------|-------|
| Implement config migration from OpenClaw paths and schema | ✅ Done | migrate_legacy_layout() moves files to new directory hierarchy |
| Provide validation and diagnostics for incompatible settings | ⚠️ Partial | Doctor command exists with --repair; not all edge cases covered |
| Add a migration guide and sample configs | ⚠️ Partial | config.example.toml exists; no dedicated migration guide |

## Phase 8 — Validation & Release

| Task | Status | Notes |
|------|--------|-------|
| Run parity matrix review and close remaining gaps | ⚠️ In progress | This document tracks status |
| Add integration tests for CLI + gateway + skills + messengers | ✅ Done | 7 integration test files, 200+ tests |
| Update README and QUICKSTART with parity status | ✅ Done | README.md updated |
| Publish versioned parity notes and changelog | ✅ Done | CHANGELOG.md created |

---

## Remaining Gaps

### ⚠️ Incomplete Items (from phases above)

1. **CLI env var precedence audit** — env var override behavior not fully audited against OpenClaw (Phase 1)
2. **Dedicated TUI log view** — messages pane exists but no separate log/debug view (Phase 5)
3. **Cross-tool secret import** — legacy migration works but OpenClaw→RustyClaw secret import not tested (Phase 6)
4. **Doctor command edge cases** — `--repair` exists but doesn't cover all invalid config states (Phase 7)
5. **Dedicated migration guide** — only config.example.toml exists; no step-by-step migration doc (Phase 7)

### ⚠️ Stub / Partial Implementations

6. **Canvas tool** — accepts parameters and returns descriptive text but has no actual canvas rendering integration (`src/tools/devices.rs:1135`)
7. **Browser tool (without `browser` feature)** — returns stub descriptions of what would happen; real CDP is behind the `browser` feature flag (`src/tools/browser.rs:530`)
8. **TTS tool (without API key)** — returns a descriptive fallback; functional when OPENAI_API_KEY is set (`src/tools/gateway_tools.rs:370`)
9. **Process tool: `send-keys`** — not implemented; `write` action exists for stdin but no PTY/send-keys support

### ⚠️ Missing OpenClaw Features

10. **WhatsApp messenger backend** — OpenClaw supports WhatsApp; RustyClaw does not
11. **Slack messenger backend** — OpenClaw supports Slack; RustyClaw does not
12. **Gateway WSS/TLS support** — OpenClaw supports `wss://`; RustyClaw only supports `ws://`
13. **Sandbox enforcement** — Landlock and PathValidation modes are stubs; only bwrap provides real isolation
14. **SECURITY.md accuracy** — document references wrong crate (`keyring` instead of `securestore`) and lists outdated dependency versions

### ✅ Previously Missing, Now Implemented

The following items were listed as "Not implemented" in the original Gap Analysis but have since been completed:

- Process management (list, poll, log, write, kill) — `src/process_manager.rs`, `src/tools/runtime.rs`
- Memory system (memory_search BM25, memory_get) — `src/memory.rs`, `src/tools/memory_tools.rs`
- Session/multi-agent tools (list, spawn, send, history, status) — `src/sessions.rs`, `src/tools/sessions_tools.rs`
- Cron/scheduling (at, every, cron expressions) — `src/cron.rs`, `src/tools/cron_tool.rs`
- Message tool (send, broadcast) — `src/tools/gateway_tools.rs`
- Node/device control (SSH/ADB: camera, screen, location, run, notify) — `src/tools/devices.rs`
- Image analysis (OpenAI/Anthropic/Google vision APIs) — `src/tools/gateway_tools.rs:441`
- TTS (OpenAI TTS API) — `src/tools/gateway_tools.rs:348`
- Apply patch (multi-hunk unified diff) — `src/tools/patch.rs`
- Gateway control tool (config get/apply/patch, restart) — `src/tools/gateway_tools.rs`
- True streaming from providers (OpenAI SSE + Anthropic SSE) — `src/streaming.rs`, `src/gateway/providers.rs`

---

## Progress Summary

| Category | Status | Coverage |
|----------|--------|----------|
| File tools (read, write, edit, list, search, find) | ✅ Complete | 6/6 |
| Web tools (fetch, search) | ✅ Complete | 2/2 |
| Shell execution | ✅ Complete | 1/1 (with background) |
| Process management | ✅ Complete | list, poll, log, write, kill |
| Memory system | ✅ Complete | search + get |
| Cron/scheduling | ✅ Complete | at, every, cron |
| Multi-session / multi-agent | ✅ Complete | list, spawn, send, history, status |
| Secrets vault & policies | ✅ Complete | list, get, store |
| Gateway control | ✅ Complete | config get/apply/patch, restart |
| Message tool | ✅ Complete | send, broadcast |
| TTS | ✅ Complete | functional with API key |
| Apply patch | ✅ Complete | multi-hunk diff |
| Image analysis | ✅ Complete | OpenAI/Anthropic/Google vision |
| Browser automation | ⚠️ Partial | Real CDP behind `browser` feature; stub without |
| Node/device control | ✅ Complete | SSH/ADB backends |
| Canvas | ⚠️ Stub | Parameter handling only; no rendering integration |
| Context management (compaction, token tracking) | ✅ Complete | — |
| Conversation memory (persistence, replay) | ✅ Complete | — |
| Gateway (auth, heartbeat, message types) | ✅ Complete | — |
| CLI commands | ✅ Complete | 10 subcommands |
| TUI commands | ✅ Complete | 12+ slash-commands |
| Skills (loading, format support) | ✅ Complete | Load + gate checks + prompt injection |
| Messengers | ⚠️ Partial | Webhook, Console, Discord, Telegram, Signal (missing WhatsApp, Slack) |
| Provider streaming | ✅ Complete | OpenAI SSE + Anthropic SSE |
| Gateway TLS (WSS) | ❌ Missing | Only ws:// supported |
| Sandbox enforcement | ⚠️ Partial | Only bwrap works; Landlock/PathValidation are stubs |

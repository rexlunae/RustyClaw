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

#### Messenger Channels (RustyClaw has 5, OpenClaw has 13)
10. **WhatsApp messenger** — OpenClaw has full Baileys integration; RustyClaw missing
11. **Slack messenger** — OpenClaw has Bolt integration; RustyClaw missing
12. **Google Chat messenger** — OpenClaw has Chat API integration; RustyClaw missing
13. **BlueBubbles (iMessage)** — OpenClaw has iMessage via BlueBubbles API (recommended); RustyClaw missing
14. **iMessage (legacy)** — OpenClaw has legacy imsg integration; RustyClaw missing
15. **Microsoft Teams messenger** — OpenClaw extension support; RustyClaw missing
16. **Matrix messenger** — OpenClaw extension support; RustyClaw has partial implementation (optional feature)
17. **Zalo messenger** — OpenClaw extension; RustyClaw missing
18. **Zalo Personal messenger** — OpenClaw extension; RustyClaw missing
19. **WebChat** — OpenClaw serves WebChat UI from Gateway; RustyClaw missing

#### Voice & Speech Features
20. **Voice Wake** — OpenClaw has always-on speech with ElevenLabs (macOS/iOS/Android); RustyClaw missing
21. **Talk Mode** — OpenClaw has continuous conversation overlay with ElevenLabs; RustyClaw missing

#### Visual & UI Features
22. **Live Canvas** — OpenClaw has agent-driven visual workspace with A2UI; RustyClaw canvas is stub
23. **Control UI / Web Dashboard** — OpenClaw serves Control UI directly from Gateway; RustyClaw missing
24. **macOS menu bar app** — OpenClaw has companion app with Voice Wake/PTT/Talk Mode; RustyClaw TUI only
25. **iOS node app** — OpenClaw has Canvas/Voice/camera/screen recording; RustyClaw missing
26. **Android node app** — OpenClaw has Canvas/Talk/camera/screen recording/SMS; RustyClaw missing

#### Security & Access Features
27. **DM pairing system** — OpenClaw has pairing codes for unknown senders with allowlist; RustyClaw missing
28. **Tailscale Serve/Funnel** — OpenClaw auto-configures Tailscale for remote access; RustyClaw missing
29. **Remote Gateway support** — OpenClaw designed for Linux server deployment with node pairing; RustyClaw local only
30. **Gateway WSS/TLS support** — OpenClaw supports `wss://` + Tailscale HTTPS; RustyClaw only `ws://`
31. **Token/password auth** — OpenClaw has gateway authentication modes; RustyClaw TOTP only

#### Automation & Integration
32. **Gmail Pub/Sub** — OpenClaw has Gmail webhook automation; RustyClaw missing
33. **Presence & typing indicators** — OpenClaw updates channel presence/typing; RustyClaw missing
34. **Session activation modes** — OpenClaw has mention gating, reply tags, group routing; RustyClaw basic sessions
35. **Elevated bash toggle** — OpenClaw has `/elevated on|off` per-session control; RustyClaw missing

#### Platform Features
36. **Node mode (device-local actions)** — OpenClaw routes device actions to paired nodes; RustyClaw executes locally only
37. **macOS TCC permission routing** — OpenClaw routes actions based on TCC permissions; RustyClaw missing
38. **Bonjour/mDNS pairing** — OpenClaw auto-discovers nodes on local network; RustyClaw missing
39. **Companion app debug tools** — OpenClaw macOS app has debug panel; RustyClaw CLI only

#### Developer & Ops Features
40. **Nix mode** — OpenClaw has declarative config via Nix; RustyClaw missing
41. **Development channels** — OpenClaw has stable/beta/dev with `openclaw update --channel`; RustyClaw missing
42. **Control UI skill management** — OpenClaw manages skills via web UI with install gating; RustyClaw CLI only

#### Core Fixes Needed in RustyClaw
43. **Sandbox enforcement** — ~~Landlock and PathValidation modes are stubs~~ PathValidation now enforced (Phase 1 complete); Landlock still stub; only bwrap provides real isolation
44. **SECURITY.md accuracy** — document references wrong crate (`keyring` instead of `securestore`) and lists outdated dependency versions

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

### Core Tooling (✅ Strong Parity)
| Category | RustyClaw Status | OpenClaw Comparison |
|----------|------------------|---------------------|
| File tools (read, write, edit, list, search, find) | ✅ Complete (6/6) | ✅ Full parity |
| Web tools (fetch, search) | ✅ Complete (2/2) | ✅ Full parity |
| Shell execution | ✅ Complete | ✅ Full parity + background support |
| Process management | ✅ Complete | ✅ Full parity (list, poll, log, write, kill) |
| Memory system | ✅ Complete | ✅ Full parity (BM25 search + get) |
| Cron/scheduling | ✅ Complete | ✅ Full parity (at, every, cron expressions) |
| Multi-session / multi-agent | ✅ Complete | ✅ Full parity (list, spawn, send, history, status) |
| Secrets vault & policies | ✅ Complete | ✅ Full parity (typed credentials, access policies) |
| Gateway control | ✅ Complete | ✅ Full parity (config get/apply/patch, restart) |
| Message tool | ✅ Complete | ✅ Full parity (send, broadcast) |
| TTS | ✅ Complete | ✅ Full parity (OpenAI TTS API) |
| Apply patch | ✅ Complete | ✅ Full parity (multi-hunk unified diff) |
| Image analysis | ✅ Complete | ✅ Full parity (OpenAI/Anthropic/Google vision) |
| Context management | ✅ Complete | ✅ Full parity (compaction, token tracking) |
| Conversation memory | ✅ Complete | ✅ Full parity (persistence, replay) |
| Provider support | ✅ Complete | ✅ Full parity (OpenAI, Anthropic, Google, xAI, Ollama, custom) |
| Provider streaming | ✅ Complete | ✅ Full parity (OpenAI SSE + Anthropic SSE) |

### Platform Features (⚠️ Partial Parity)
| Category | RustyClaw Status | OpenClaw Comparison | Gap |
|----------|------------------|---------------------|-----|
| CLI commands | ✅ Complete | ✅ Full parity | 10 subcommands aligned |
| TUI interface | ✅ Complete | ⚠️ Partial parity | RustyClaw has TUI, OpenClaw has Control UI + WebChat + macOS app |
| Skills system | ✅ Complete | ✅ Full parity | Load + gate checks + prompt injection |
| Browser automation | ⚠️ Partial | ⚠️ Partial parity | RustyClaw has CDP (optional); OpenClaw has dedicated browser profiles |
| Node/device control | ✅ Complete | ⚠️ Partial parity | RustyClaw has SSH/ADB backends; OpenClaw has node pairing + TCC routing |
| Canvas | ⚠️ Stub | ❌ Major gap | RustyClaw stub only; OpenClaw has A2UI + visual workspace |
| Messengers | ⚠️ Partial (5/13) | ❌ Major gap | RustyClaw missing 8 channels: WhatsApp, Slack, Google Chat, iMessage, Teams, Zalo, WebChat |
| Gateway architecture | ✅ WebSocket + daemon | ⚠️ Partial parity | RustyClaw ws:// only; OpenClaw has wss:// + Tailscale |
| Sandbox enforcement | ⚠️ In progress | ⚠️ Partial parity | RustyClaw fixing C1 issue; OpenClaw has sandboxing |

### Missing Platform Features (❌ No Implementation)
| Category | RustyClaw Status | OpenClaw Has | Priority |
|----------|------------------|--------------|----------|
| Voice features | ❌ Missing | Voice Wake + Talk Mode (ElevenLabs) | Medium |
| Companion apps | ❌ Missing | macOS/iOS/Android apps | Low |
| Control UI / Web Dashboard | ❌ Missing | Served from Gateway | Medium |
| DM pairing security | ❌ Missing | Pairing codes + allowlist | High (security) |
| Tailscale integration | ❌ Missing | Serve/Funnel auto-config | Low |
| Remote Gateway | ❌ Missing | Linux server deployment pattern | Low |
| Gmail Pub/Sub | ❌ Missing | Email webhook automation | Low |
| Presence/typing | ❌ Missing | Channel presence updates | Low |
| Elevated bash | ❌ Missing | Per-session privilege toggle | Low |
| Nix mode | ❌ Missing | Declarative config | Very Low |

### Summary Statistics
- **Core tools parity**: ~95% (30/30 tools, excellent coverage)
- **Messenger parity**: ~38% (5/13 channels)
- **Platform features parity**: ~40% (missing voice, apps, UI, remote access)
- **Security parity**: ~70% (strong vault, fixing sandbox, missing DM pairing/TLS)
- **Overall estimated parity**: ~60%

### RustyClaw Advantages
- ✅ **Native Rust implementation** — faster startup, lower memory, better cross-compilation
- ✅ **Feature-gated builds** — headless/TUI/full builds for different deployment scenarios
- ✅ **Raspberry Pi optimized** — ARM cross-compilation + CI/CD for Pi deployment
- ✅ **Simpler architecture** — fewer dependencies, easier to audit and maintain

---

## PicoClaw Comparison

### Overview
[PicoClaw](https://github.com/sipeed/picoclaw) is an ultra-lightweight AI assistant implementation in Go, inspired by nanobot and designed for extremely resource-constrained hardware. It represents a third approach to the Claw architecture, optimized for minimal footprint.

### Implementation Comparison

| Metric | RustyClaw | OpenClaw | PicoClaw |
|--------|-----------|----------|----------|
| **Language** | Rust | TypeScript (Node.js) | Go |
| **RAM Required** | ~50-200MB (estimated) | >1GB | <10MB |
| **Startup Time (0.8GHz)** | ~2-5s (estimated) | >500s | <1s |
| **Binary Size** | ~15-30MB (stripped) | N/A (interpreted) | Single self-contained binary |
| **Target Hardware** | Raspberry Pi 3B+ (~$35) | Mac Mini ($599+) | LicheeRV-Nano (~$10) |
| **Architectures** | x64, ARM64, ARMv7 | x64, ARM64 | x64, ARM64, RISC-V |

### PicoClaw Feature Set

**✅ Core Features**
- Single self-contained binary deployment
- Multi-platform messenger support (Telegram, Discord, QQ, DingTalk, LINE)
- Web search (DuckDuckGo, Brave Search)
- Memory management system
- Scheduled task automation (cron-like)
- Security sandbox (workspace-restricted file/command access)
- Custom skills and tools support

**❌ Notable Limitations (vs OpenClaw/RustyClaw)**
- Minimal tool ecosystem compared to OpenClaw/RustyClaw's 30+ tools
- No TUI interface (daemon + messenger only)
- No gateway WebSocket architecture (simpler direct integration)
- No multi-provider support (focused on single LLM backend)
- No voice/visual features (Canvas, TTS, Voice Wake, etc.)
- No advanced security features (typed secrets vault, TOTP, access policies)
- No session management or multi-agent spawning
- No browser automation or device control tools

### Design Philosophy

| Project | Philosophy | Target Use Case |
|---------|-----------|-----------------|
| **RustyClaw** | Performance + feature balance | Raspberry Pi/SBC, self-hosted with strong parity |
| **OpenClaw** | Feature-complete agent platform | Desktop/server, full-featured AI assistant |
| **PicoClaw** | Ultra-minimal footprint | $10 hardware, embedded systems, IoT devices |

### When to Choose Each

**Choose RustyClaw if:**
- You want strong parity with OpenClaw's tools (30/30 tools implemented)
- You're deploying to Raspberry Pi or similar ARM SBCs (~$35)
- You need native performance with reasonable memory usage
- You want a Rust codebase for security/maintainability

**Choose OpenClaw if:**
- You need the full feature set (voice, visual, companion apps)
- You have >1GB RAM available
- You want the most mature, feature-complete platform
- You need Control UI, WebChat, or macOS/iOS/Android apps

**Choose PicoClaw if:**
- You have <10MB RAM budget
- You're deploying to ultra-cheap hardware ($10 devices)
- You only need basic AI assistant features via messengers
- You need RISC-V support
- Boot time and memory are critical constraints

### PicoClaw Development Model
PicoClaw is notable for being **95% agent-generated** with human-in-the-loop refinement, demonstrating the viability of AI-driven development for creating minimal, production-ready systems.

---

## Three-Way Ecosystem Summary

The Claw ecosystem now spans three implementations optimized for different deployment scenarios:

1. **RustyClaw (Rust)** — Performance-optimized with strong tool parity for SBCs ($35+ hardware)
2. **OpenClaw (TypeScript)** — Full-featured platform for desktop/server ($599+ hardware)
3. **PicoClaw (Go)** — Ultra-minimal for embedded/IoT deployment ($10+ hardware)

**RustyClaw** occupies the middle ground: more capable than PicoClaw (30 tools vs basic set), more efficient than OpenClaw (~50-200MB vs >1GB RAM), targeting the sweet spot of self-hosted Raspberry Pi deployments with near-complete OpenClaw tool compatibility.

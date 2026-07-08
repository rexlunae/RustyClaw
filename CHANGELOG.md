# Changelog

All notable changes to RustyClaw will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Engines dialog: per-engine tabs and live install output.** The Local
  Engines & Models dialog now renders one tab per detected engine (←/→ or
  Tab to switch in the TUI; a Bulma tab strip on desktop), so each
  engine's status, models, and actions have their own focused view instead
  of one long combined list. Engine installs — which previously ran
  silently — now stream their output live: `execute`-style installers
  (`curl … | sh`, `brew install`, `cargo install …`) are read line by
  line via a new `stream_shell` helper in core, forwarded over a new
  `EngineActionProgress` frame (mirroring how model-pull progress already
  streams), and folded into the installing engine's tab as a bounded,
  live-updating log that ends with "install complete" / "install failed".
  The install output is tracked per engine and survives the frequent
  engine-list refreshes.

- **Live display and inline controls for long-running processes.** While
  a tool call executes, the gateway now streams a `ToolStatus` frame
  every second (after a 2s grace period so fast tools stay silent)
  carrying the call's elapsed time and — when the tool is waiting on a
  child process — that process's CPU usage, resident memory, and
  scheduler state (running, sleeping, blocked on I/O, paused, …),
  sampled via a new exec-status registry that every foreground
  `execute_command` child registers with. The TUI renders this as a
  live line inside the inline tool panel (`⏳ 12s · running · cpu 87% ·
  mem 145 MB · pid 4242`), the desktop shows the same line under the
  running tool call, and the CLI prints periodic status to stderr.
  When the status carries a PID the process is controllable inline from
  the chat — Ctrl+Z pauses/resumes (SIGSTOP/SIGCONT, with the exec
  timeout clock frozen while paused so a paused command can't time
  out), Ctrl+T sends SIGTERM, Ctrl+K sends SIGKILL — via a new
  `ProcessControl` client frame that the gateway's reader task handles
  even while the tool loop is blocked on that very process. Controls
  are allowlisted: only PIDs the gateway itself spawned for the current
  tool call can be signalled, and exec children now lead their own
  process group so signals reach the whole shell pipeline.
- **Live tool activity.** Running commands now show their output as it
  happens, inside the same panel as the tool call. `execute_command`
  reads the child's pipes incrementally and the gateway forwards each
  chunk over the previously-stubbed `ToolOutputDelta` frame; both
  clients fold the chunks into the running call's panel, which stays
  open while running (running work is what you want to watch) and
  collapses to the compact one-liner when it finishes. Output is
  rendered the way a terminal would: `\r`-redrawing progress bars
  overwrite their line in place instead of stacking hundreds of lines,
  ANSI color/cursor escapes are stripped, and the live tail is bounded
  (last 40 lines). The desktop also stops emitting a separate "tool
  result" bubble per call — the invocation, live progress, duration,
  and final result are one component now, halving transcript noise for
  agentic work.

- **The agent explains itself: visible reasoning, compact tool activity,
  timings.** The gateway has always streamed the model's reasoning text
  over the wire, but the client event layer discarded it — users only
  ever saw a spinner. `GatewayEvent::ThinkingDelta` now carries the
  text, and both clients accumulate it into a collapsible 💭 block:
  compact by default (a one-line gist under a "Thought for 4.2s"
  header) and fully expandable (Ctrl+E in the TUI; the desktop renders
  a step-per-paragraph reasoning timeline). Reasoning also folds the
  moment answer text starts streaming instead of at stream end.
  Tool calls in the TUI drop the raw-JSON peek for a semantic one-liner
  (`read src/main.rs:10–80 · ✓ 0.4s · 71 lines`, `$ cargo test · ✓
  12s`) with argument/result detail on expand, matching the desktop's
  hint panels; both clients stamp every tool call and thinking block
  with its client-measured wall-clock duration.

- **Usage analytics and logs panels backed by real telemetry.** The
  gateway now installs a stats-collecting observer at startup (it
  previously passed `None`, so the observability layer recorded
  nothing): every LLM call is recorded with provider, model, token
  counts (the genai backend's captured usage now actually reaches the
  telemetry — it was a `TODO`), latency, and outcome, alongside a
  human-readable ring of LLM/tool/channel/error events. `/analytics
  [day|week|month|all]` and `/logs [source] [n]` in the TUI and the
  View-menu Usage Analytics / Logs dialogs on desktop query it; the
  logs panel also serves managed-service logs by service name.
- **Desktop custom-provider management.** Settings gains a Custom
  Providers section — list/remove existing `[[custom_providers]]`
  entries and add new ones (id, name, base URL, API format, key secret,
  static models) with validation; saving updates the provider catalogue
  so the model bar picks the change up immediately. Also: the Skills
  menu opens a real skills manager (was "coming soon"), the secrets
  dialog's Add Secret flow works (with auto-refresh after every vault
  mutation and a real 2FA indicator via the new `SecretsHasTotp`
  client command), the Services dialog populates on open, and System
  Info fetches host/load data on open.
- **The cron, memory, MCP, channels, and tool-config panels are real.**
  The gateway's panel handler previously returned stub/empty responses
  for every panel request even though the backing subsystems existed.
  Panels now operate on the same backends the AI tools use: cron
  list/add/pause/resume/remove against the persistent `.cron` store,
  memory list/add/edit/delete against `MEMORY.md` (bullets as entries,
  `##` headings as categories) plus `HISTORY.md` search, MCP server
  list/connect/disconnect via a shared `McpManager` (the documented
  `[mcp.servers.*]` config section is now actually loaded, and ad-hoc
  connects persist to it), tool enable/disable via
  `config.tool_permissions`, and messenger channel status/pair/unpair
  via the `[[messengers]]` config.
- **TUI:** `/cron`, `/memory`, `/mcp`, and `/channels` now open live
  panels (they used to tab-complete and then report "Unknown command"),
  with subcommands for mutations (`/cron add <name> | <schedule> |
  <message>`, `/memory add [category ::] <content>`, `/mcp connect
  <name> [command…]`, `/channels pair|unpair <name>`, …) and
  auto-refresh after every change.
- **Desktop:** the Tools menu gains Scheduled Jobs, Memory, MCP
  Servers, Channels, and Tool Permissions dialogs — the last is the
  desktop's first tool-management surface.

- **User-configured custom model providers.** New `[[custom_providers]]`
  config section (id, display name, base URL, API format, optional API-key
  secret, optional static model list). Entries are registered into the
  provider catalogue at load time (`providers::set_custom_providers`), so
  they appear alongside the built-ins in the TUI `/provider` selector, the
  onboarding wizard, the desktop settings/model bar, tab completion, and
  every credential/base-URL resolution path. Chat dispatch maps each custom
  provider's `api_format` (`openai` | `anthropic` | `gemini` | `xai`) onto
  the matching genai adapter, and model listing honours the format (with a
  static-list fallback when the endpoint is unreachable). New TUI commands:
  `/provider add <id> <base_url> [format=…] [key=…] [models=…] [name=…]`,
  `/provider remove <id>`, `/provider list`.
- **Joshua local inference engine.** [Joshua](https://github.com/rexlunae/joshua)
  (pure-Rust GGUF server) is now a first-class engine and provider:
  detect/install (`cargo install`), start/stop (`joshua serve --model … --addr
  127.0.0.1:8331`), GGUF model scan of `~/.rustyclaw/models/joshua` (or the
  configured `models_dir`), Hugging Face pulls (GGUF + `tokenizer.json`), and
  load/unload by restarting the single-model server. `EngineConfig` gains a
  `default_model` field for single-model-per-process engines, and engine
  auto-start (`engine_service_defs`) resolves the GGUF to serve.
- **`/engines` panel in the TUI.** The previously stubbed engines dialog is
  now wired end-to-end: `/engines` opens a live panel showing each engine's
  install/run state, endpoint, and models; ↑/↓ selects an engine, Enter lists
  its models, `s` starts/stops, `i` installs, `r` refreshes, and pull progress
  renders in-panel. Subcommands: `/engines start|stop|install <engine>`,
  `/engines models <engine>`, `/engines pull <engine> <model>`,
  `/engines load|unload|remove <engine> <model>`.

### Fixed

- **TUI commands that printed success and did nothing.** `/clear` now
  actually clears the display (and says thread history is unaffected
  rather than claiming memory was cleared); `/gateway` reports the real
  connection status; `/gateway start|stop|restart` and `/download`
  no longer pretend — they explain what to use instead. Multi-select
  agent prompts only ever recorded one selection: Space now toggles
  per-option checkboxes (seeded from prompt defaults) and Enter submits
  all checked options. `/help` documents the previously hidden keyboard
  shortcuts, `/quit`, and the full `/engines` subcommand list; the
  unimplemented `/analytics`, `/logs`, and `/approvals` commands no
  longer tab-complete.
- Switching providers no longer carries a stale `base_url` override from the
  previous provider into the new selection (it is kept only when the new
  provider has no catalogue URL, e.g. `custom` / `copilot-proxy`).

### Changed

- **Structured-error follow-up: restored ~100 context sites lost to the
  revert probe, dropped redundant conversions.** The pass that introduced
  `ToolError::Context` had collaterally reverted convertible call sites
  back to `format!` flattening (an earlier `cargo fix` had stripped
  then-unused `ToolError` imports in 21 files, so re-conversion attempts
  failed to resolve the type and were misclassified as non-convertible).
  Those imports are repaired and the sites re-converted — the tool layer
  now preserves typed sources at 140 context sites, with `format!` left
  only on genuine third-party leaf errors (chromiumoxide, zip, image, …).
  Also removed the extraneous conversions the same pass left behind:
  tail `.map_err(ToolError::from)` no-ops replaced by constructing
  `ToolError::msg` / `missing_param` directly, and gateway handler
  imports trimmed to what they use.

- **Structured-error preservation pass over the tool layer.**
  `ToolError` gains a `Context` variant plus `ToolError::context(ctx, e)`:
  it renders identically to the previous `format!("ctx: {e}")` flattening
  but keeps the typed error reachable via `source()`. 37 convertible
  context sites now preserve their sources; `format!` remains only for
  third-party leaf errors with no `ToolError` conversion. A new `Ssrf`
  variant propagates `SsrfError` verdicts through `web_fetch` untouched,
  and the gateway model/task handlers use plain `?` instead of
  `.map_err(|e| e.to_string())` for registry/service/task errors.
  Unit tests assert the source chain survives `Context` wrapping.

- **AI-tool layer moved to typed errors (`ToolError` / `ToolResult`).**
  All `exec_*` tool implementations (~45 files in `core/src/tools/**` and
  the gateway tool handlers) now return `ToolResult` instead of
  `Result<String, String>`. `ToolError`'s `Display` is the exact
  model-facing message; per-module typed errors (`SandboxError`,
  `ProcessError`, `TaskError`, `ServiceError`, `RegistryError`,
  `CronError`, `ConsolidationError`, `MemoryIndexError`, `SessionError`,
  `SwarmError`, `SteelMemoryError`, `io`/`serde_json`/`reqwest`) propagate
  into it with plain `?`, bespoke messages route through `ToolError::Msg`,
  and the gateway tool executor is the single point where the error is
  flattened to the model-payload string. Also typed in the same pass: the
  tool-call rate limiter (`RateLimitError`), `read_memory_file`
  (`MemoryIndexError::{InvalidPath, NotFound}`), `SubconsciousError` and
  `SyncError` (now enums preserving `anyhow` cause chains), and the
  subtask closure contract (`Result<T, SubtaskError>`). The dead
  `tools::{ToolCall, ToolResult}` wire structs (zero users) were removed,
  and STYLE_GUIDE §5 now describes the `ToolError` pattern instead of the
  `Result<String, String>` exception.

- **Completed the typed-error migration started in #303.** Remaining
  internal `Result<_, String>` plumbing now uses per-module `thiserror`
  enums, with strings only at the documented display boundaries:
  `SteelMemoryError` (steel_memory.rs — audit follow-up #1),
  `SandboxError` (sandbox + command-safety helpers — audit follow-up #2,
  policy verdicts distinguishable from execution failures),
  `TaskError` (tasks/manager.rs), `SubtaskError` (threads/subtask.rs —
  replaces the `"Cancelled"` sentinel-string comparison), `ReceiptError`
  (protocols/receipt.rs), `CustomProviderError` (providers/custom.rs),
  `MissingRequestField` (gateway resolve_request),
  `ProcessManager::spawn` returns `ProcessError`, the SSH bare-frame
  fallback returns `FrameCodecError`, and the desktop swarm helpers
  propagate `SwarmError` via `anyhow` instead of pre-flattening.
  `docs/RUST_IDIOMS_AUDIT.md` follow-ups #1 and #2 are marked fixed.
- **Provider backend migrated to the `genai` crate.** The gateway's hand-rolled
  OpenAI / Anthropic / Google HTTP clients
  (`rustyclaw-gateway/src/providers/{openai,anthropic,google}.rs`) are replaced
  by a single [`genai`](https://crates.io/crates/genai)-backed dispatch in
  **`rustyclaw-core`** (`providers/genai_backend.rs`). It lives in core so the
  gateway and the client crates share one genai instance. Request building, tool
  calling, and SSE streaming (including Anthropic extended-thinking deltas) are
  now handled by genai; RustyClaw still owns provider selection, credentials /
  Copilot session tokens, and the binary streaming frame protocol. Each provider
  id maps onto a genai adapter; all OpenAI-compatible providers (OpenRouter,
  Ollama, LM Studio, exo, OpenCode, GitHub Copilot, custom) use the OpenAI
  adapter at their configured base URL. The gateway's
  `providers::call_{openai,anthropic,google}_with_tools` re-export the core
  implementation, so dispatch / messenger / thread / compaction call sites are
  unchanged.

### Notes

- Tool-loop continuation messages now use a single provider-agnostic canonical
  encoding (`providers::encode_assistant_message` / `encode_tool_result`)
  instead of per-provider JSON shapes.
- The previous automatic fallback to the OpenAI *Responses API* (for models that
  reject `/chat/completions`) is not reproduced; genai selects the Responses API
  adapter from the model name instead.

## [0.1.0] - 2026-02-12

### 🎉 Initial Release - Full OpenClaw Parity

This release achieves complete feature parity with OpenClaw's agentic capabilities.

### Added

#### Tools (30 total)
- **File tools**: read_file, write_file, edit_file, list_directory, search_files, find_files
- **Runtime tools**: execute_command, process (background management)
- **Web tools**: web_fetch (URL content extraction), web_search (Brave Search API)
- **Memory tools**: memory_search (BM25 keyword search), memory_get (snippet retrieval)
- **Scheduling**: cron (at, every, cron expressions)
- **Session tools**: sessions_list, sessions_spawn, sessions_send, sessions_history, session_status, agents_list
- **Editing**: apply_patch (multi-hunk unified diff)
- **Secrets tools**: secrets_list, secrets_get, secrets_store
- **System tools**: gateway (config/restart/update), message (send/broadcast), tts
- **Media**: image (vision model analysis)
- **Devices**: nodes (camera, screen, location, remote exec)
- **Browser**: browser (Playwright/CDP automation)
- **Canvas**: canvas (A2UI presentation)

#### Skills System
- SKILL.md parsing with YAML frontmatter
- Gate checking: bins, anyBins, env, config, os
- Prompt context injection for eligible skills
- `{baseDir}` placeholder substitution
- Directory precedence: workspace > local > bundled

#### Messenger Backends
- WebhookMessenger - POST to any URL
- ConsoleMessenger - stdout for testing
- DiscordMessenger - bot API integration
- TelegramMessenger - bot API integration

#### Provider Streaming
- OpenAI SSE streaming with tool call support
- Anthropic SSE streaming with content blocks
- mpsc channel-based chunk delivery

#### Gateway
- WebSocket server with ping/pong keepalive
- TOTP 2FA authentication
- Rate limiting and lockout
- Multi-provider support (OpenAI, Anthropic, Google, GitHub Copilot, xAI, Ollama, OpenRouter)
- Context compaction at 75% window

#### TUI
- Slash commands: /help, /clear, /provider, /model, /gateway, /secrets, /skills, /status, /quit
- Tab completion
- Pane navigation (ESC/TAB)
- Message scrolling

#### Secrets Vault
- AES-256 encrypted storage
- Access policies (Always, WithAuth, SkillOnly, Never)
- TOTP 2FA protection
- Rate limiting and lockout

#### Testing
- 152+ unit tests
- 200+ integration tests
- CLI conformance tests
- Gateway protocol tests
- Skill execution tests
- Tool execution tests
- Exit code tests
- Golden file tests
- Streaming tests

#### CLI Commands
- setup, onboard, configure
- config get/set/unset
- doctor --repair
- tui
- command (one-shot)
- status
- gateway start/stop/restart/status
- skills list/enable/disable

### Project Logo
- Half gear / half lobster claw design (logo.svg)

---

## Future Roadmap

### Planned for 0.2.0
- [ ] Full Playwright/CDP browser implementation
- [ ] Real vision model integration
- [ ] Real TTS service integration (ElevenLabs)
- [ ] Slack messenger backend
- [ ] WhatsApp messenger backend
- [ ] Signal messenger backend
- [ ] Google Gemini streaming

### Planned for 0.3.0
- [ ] Plugin system
- [ ] Tool profiles and policies
- [ ] Remote node execution
- [ ] macOS app bundle

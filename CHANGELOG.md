# Changelog

All notable changes to RustyClaw will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

- Switching providers no longer carries a stale `base_url` override from the
  previous provider into the new selection (it is kept only when the new
  provider has no catalogue URL, e.g. `custom` / `copilot-proxy`).

### Changed

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

# Changelog

All notable changes to RustyClaw will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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

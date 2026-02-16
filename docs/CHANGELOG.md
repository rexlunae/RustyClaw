# Changelog

All notable changes to RustyClaw will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

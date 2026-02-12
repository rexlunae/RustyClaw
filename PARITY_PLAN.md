# RustyClaw ↔ OpenClaw Parity Plan

## Current State (RustyClaw)

### ✅ Implemented Tools (7 total)
1. `read_file` — read file contents with line ranges
2. `write_file` — create/overwrite files
3. `edit_file` — search-and-replace edits
4. `list_directory` — list directory contents
5. `search_files` — grep-like content search
6. `find_files` — find files by name/glob
7. `execute_command` — run shell commands (with timeout)

### ✅ Implemented Features
- Multi-provider support (OpenAI, Anthropic, Google, GitHub Copilot)
- Tool-calling loop (up to 25 rounds)
- Context compaction (auto-summarize when context gets large)
- TOTP 2FA authentication
- Secrets vault with access policies
- TUI interface
- Skills loading (JSON/YAML definitions)
- SOUL.md personality system
- Conversation history persistence
- WebSocket gateway architecture

---

## Gap Analysis: Missing OpenClaw Capabilities

### 🔴 Critical (Core Agentic Features)

#### 1. Process Management (`process` tool)
OpenClaw has backgrounded process management:
- `list`, `poll`, `log`, `write`, `send-keys`, `kill`, `clear`, `remove`
- PTY support for interactive CLIs
- Session persistence across tool rounds

**RustyClaw status**: `execute_command` blocks until completion, no background support.

#### 2. Web Tools
- `web_search` — Brave Search API integration
- `web_fetch` — URL → markdown extraction

**RustyClaw status**: Not implemented.

#### 3. Memory System
- `memory_search` — semantic search over MEMORY.md + memory/*.md
- `memory_get` — snippet retrieval with line ranges

**RustyClaw status**: Not implemented. No memory recall mechanism.

#### 4. Session/Multi-Agent Tools
- `sessions_list` — list active sessions
- `sessions_history` — fetch transcript history
- `sessions_send` — cross-session messaging
- `sessions_spawn` — spawn sub-agent tasks
- `session_status` — usage/cost tracking
- `agents_list` — list available agents for spawning

**RustyClaw status**: Single-session only. No multi-agent support.

### 🟡 Important (Extended Capabilities)

#### 5. Browser Automation (`browser` tool)
- Multi-profile browser control
- Snapshot (aria/ai accessibility tree)
- Screenshot
- UI actions (click/type/press/hover/drag)
- Chrome extension relay support

**RustyClaw status**: Not implemented.

#### 6. Cron/Scheduling (`cron` tool)
- Scheduled jobs (at, every, cron expressions)
- System events and agent turns
- Job management (add/update/remove/run/runs)
- Wake events

**RustyClaw status**: Not implemented.

#### 7. Message Tool (`message`)
- Cross-platform messaging (Discord/Telegram/WhatsApp/Signal/Slack/etc.)
- Polls, reactions, threads, search
- Media attachments

**RustyClaw status**: Messenger abstraction exists but no tool exposure.

#### 8. Node/Device Control (`nodes` tool)
- Paired device discovery
- Camera/screen capture
- Location services
- Remote command execution
- Notifications

**RustyClaw status**: Not implemented.

#### 9. Canvas (`canvas` tool)
- Present/hide/navigate/eval
- Snapshot rendering
- A2UI (accessibility-to-UI)

**RustyClaw status**: Not implemented.

### 🟢 Nice-to-Have

#### 10. Gateway Control (`gateway` tool)
- Config get/apply/patch
- In-place restart
- Self-update

**RustyClaw status**: Partial (config exists, no tool exposure).

#### 11. Image Analysis (`image` tool)
- Vision model integration
- Image understanding

**RustyClaw status**: Not implemented.

#### 12. TTS (`tts` tool)
- Text-to-speech generation

**RustyClaw status**: Not implemented.

#### 13. Apply Patch (`apply_patch` tool)
- Multi-hunk structured patches

**RustyClaw status**: Not implemented (edit_file handles single replacements).

---

## Implementation Priority

### Phase 1: Core Tool Parity (Weeks 1-2)
1. **Process management** — background exec, session tracking, PTY
2. **Web tools** — web_search (Brave), web_fetch (readability extraction)
3. **Memory system** — memory_search, memory_get with semantic search

### Phase 2: Extended Tools (Weeks 3-4)
4. **Cron/scheduling** — job management, scheduled agent turns
5. **Message tool** — expose messenger abstraction to agent
6. **Session tools** — multi-session awareness (sessions_list, sessions_send)

### Phase 3: Advanced Features (Weeks 5-6)
7. **Browser automation** — Playwright/CDP integration
8. **Node control** — device pairing, remote execution
9. **Canvas** — A2UI rendering

### Phase 4: Polish (Week 7+)
10. Image analysis, TTS, apply_patch
11. Gateway self-management
12. Tool profiles and policies

---

## Architecture Notes

### Tool Registration
Current: Static `all_tools()` returns a fixed vec.
Needed: Dynamic registry supporting:
- Core tools (always available)
- Optional tools (web, browser, etc.)
- Plugin tools (extensible)
- Tool policies (allow/deny lists)

### Async Execution
Current: Tools execute synchronously.
Needed: Async tool execution for:
- Background processes
- Long-running web fetches
- Browser automation

### Configuration
Current: `config.toml` for basic settings.
Needed: Extended config for:
- `tools.web.search.enabled`, `tools.web.fetch.enabled`
- `browser.enabled`, `browser.defaultProfile`
- `tools.allow`, `tools.deny`
- Tool-specific settings

---

## Next Steps

1. Start with **web_fetch** — relatively simple, high value
2. Add **web_search** — requires Brave API key handling
3. Implement **memory_search** — needs embedding/semantic search
4. Add **process** tool — background execution model

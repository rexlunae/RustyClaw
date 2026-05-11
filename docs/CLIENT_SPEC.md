# RustyClaw Client Specification

> Version 1.0 — defines the requirements for a feature-complete RustyClaw client.

A **client** is any frontend that connects to a RustyClaw gateway and presents
the AI agent experience to a user. The reference client is `rustyclaw-tui`
(terminal UI). Other planned clients include web, mobile, and headless/bot.

---

## 1. Architecture Overview

```text
┌──────────────────────────────────────────────────────────────┐
│                      rustyclaw-core                          │
│  config · gateway protocol · secrets · skills · providers    │
│  commands · streaming · soul · tools · sessions · messengers │
└──────────────┬───────────────────────────────┬───────────────┘
               │                               │
    ┌──────────┴──────────┐         ┌──────────┴──────────┐
    │   rustyclaw-tui     │         │   future-client     │
    │  (reference client) │         │  (web, mobile, …)   │
    └─────────────────────┘         └─────────────────────┘
```

Clients depend on `rustyclaw-core` for all shared logic. They **never**
duplicate gateway protocol handling, config parsing, secrets management,
or tool dispatch.

---

## 2. Required Capabilities

A feature-complete client MUST implement the following capabilities.

### 2.1 Gateway Connection

| Requirement | Description |
| --- | --- |
| **SSH connect** | Connect to the gateway at a configured `ssh://` URL using the binary frame protocol defined in `rustyclaw-core::gateway`. |
| **Wire framing** | Send and receive length-prefixed bincode `WireFrame<T>` envelopes. Stream `0` is reserved for connection-level control; chat requests SHOULD use nonzero client-allocated stream IDs. |
| **Hello handshake** | Receive and process the `Hello` server frame (provider, model, version, capabilities). |
| **Auth challenge** | Handle `AuthChallenge` frames — prompt the user for a TOTP code and send `AuthResponse`. |
| **Auth result** | Process `AuthResult` (ok/fail/retry). Display errors. Allow retry on failure. |
| **Vault unlock** | When gateway status is `VaultLocked`, prompt for a vault password and send `VaultUnlock`. |
| **Reconnection** | Detect disconnection and attempt automatic reconnection with exponential backoff. |
| **Graceful close** | Send a close frame on shutdown. |

### 2.2 Chat / Conversation

| Requirement | Description |
| --- | --- |
| **Send messages** | Accept user text input and send `Chat` client frames to the gateway. |
| **Receive responses** | Process `Delta` (streaming token), `Done`, and `Error` server frames. |
| **Streaming display** | Display assistant responses incrementally as `Delta` frames arrive. |
| **Conversation history** | Maintain an ordered list of `ChatMessage` entries (role + content). |
| **Message roles** | Visually distinguish messages by role: `User`, `Assistant`, `Info`, `Success`, `Warning`, `Error`, `System`, `ToolCall`, `ToolResult`, `Thinking`. |
| **Markdown rendering** | Render assistant markdown (headings, code blocks, lists, tables, links). |
| **Clear history** | Support a `/clear` command to reset conversation state. |

### 2.3 Tool Approval

| Requirement | Description |
| --- | --- |
| **Tool call display** | Show tool invocations (name, arguments) before execution. |
| **Approval prompt** | When the gateway sends `ToolApproval`, present approve/deny/always-approve UI. |
| **Approval response** | Send `ToolApprovalResponse` with the user's decision. |
| **Tool result display** | Show tool results and errors after execution. |
| **Permission memory** | Remember "always approve" decisions for the session. |

### 2.4 Secrets Management

| Requirement | Description |
| --- | --- |
| **List secrets** | Display stored secret names (never values) via `/secrets list`. |
| **Store secret** | Prompt for name + value and send `SecretStore` to the gateway. |
| **Delete secret** | Support `/secrets delete <name>`. |
| **Vault lock/unlock** | Support locking/unlocking the secrets vault. |
| **Password change** | Support changing the vault password. |

### 2.5 Model & Provider Selection

| Requirement | Description |
| --- | --- |
| **Show current model** | Display the active provider + model from the `Hello` frame. |
| **Switch model** | Support `/model` command to change the active model. |
| **Provider selection** | List available providers and allow switching. |
| **API key entry** | Prompt for and store API keys during onboarding. |
| **Reload** | Send `Reload` frame to hot-reload gateway config. |

### 2.6 Sessions

| Requirement | Description |
| --- | --- |
| **List sessions** | Show saved conversation sessions via `/sessions list`. |
| **Resume session** | Resume a previous session by key via `/sessions resume <key>`. |
| **Save session** | Save the current conversation via `/sessions save`. |
| **Delete session** | Delete a session via `/sessions delete <key>`. |

### 2.7 Skills

| Requirement | Description |
| --- | --- |
| **List skills** | Show installed skills with enabled/disabled status. |
| **Skill info** | Display skill details (name, description, tools, required secrets). |
| **Enable/disable** | Toggle skills on/off. |

### 2.8 Slash Commands

The client MUST handle these slash commands (delegating to `rustyclaw-core::commands`):

| Command | Description |
| --- | --- |
| `/help` | Show available commands |
| `/clear` | Clear conversation history |
| `/status` | Show system status |
| `/model [provider/model]` | View or change model |
| `/config get\|set\|unset` | Config management |
| `/secrets list\|store\|delete` | Secrets management |
| `/sessions list\|save\|resume\|delete` | Session management |
| `/skills list\|info` | Skills management |
| `/compact` | Compact conversation context |
| `/cost` | Show token usage / cost estimate |
| `/doctor` | Health checks |
| `/quit` or `/exit` | Exit the client |

### 2.9 Configuration

| Requirement | Description |
| --- | --- |
| **Load config** | Load from `~/.rustyclaw/config.toml` (or `--config` override). |
| **Settings dir** | Respect `--settings-dir` override. |
| **No-color mode** | Support `--no-color` / `NO_COLOR` env var. |
| **Gateway URL** | Accept `--url` override for gateway WebSocket address. |

### 2.10 Onboarding (Optional but Recommended)

| Requirement | Description |
| --- | --- |
| **First-run wizard** | Detect missing config and guide the user through setup. |
| **Provider selection** | Let the user choose a model provider. |
| **API key entry** | Securely accept and store API keys. |
| **Gateway setup** | Configure gateway connection (local or remote). |
| **Workspace init** | Create the workspace directory structure. |
| **SOUL.md creation** | Generate or import the system prompt file. |

---

## 3. Core Types (from `rustyclaw-core`)

Clients MUST use these types from the core library — they should **not**
redefine protocol or domain types.

### 3.1 Gateway Protocol

- `ClientFrame`, `ClientFrameType`, `ClientPayload` — outgoing frames
- `ServerFrame`, `ServerFrameType`, `ServerPayload` — incoming frames
- `WireFrame<T>` — multiplexing envelope containing protocol version, stream ID, sequence, flags, and the application frame
- `serialize_frame()`, `deserialize_frame()` — binary codec
- `serialize_wire_frame()`, `deserialize_wire_frame()` — binary codec for multiplexed SSH/stdin payloads
- `ChatMessage` — conversation entry (role + content)
- `ModelContext` — resolved model configuration

### 3.2 Configuration

- `Config` — full application configuration
- `CommonArgs` — shared CLI arguments
- `ModelProvider` — provider + model + base_url

### 3.3 Display Types

- `MessageRole` — enum of message roles (User, Assistant, Info, etc.)
- `GatewayStatus` — enum of connection states (Connected, Disconnected, etc.)
- `InputMode` — enum of input states (Normal, Input)

### 3.4 Commands

- `handle_command()` — parse and execute slash commands
- `CommandContext` — dependencies for command execution
- `CommandResponse` — result of a command
- `CommandAction` — action to take after command (Quit, ClearMessages, etc.)

### 3.5 Services

- `SecretsManager` — encrypted vault operations
- `SkillManager` — skill loading and registry
- `SoulManager` — SOUL.md system prompt management
- `MessengerManager` — multi-platform messaging

---

## 4. Client Registration (Future)

In a future version, clients will register via a manifest:

```toml
[client]
name = "rustyclaw-tui"
version = "0.1.0"
description = "Terminal UI client for RustyClaw"
capabilities = ["chat", "tools", "secrets", "sessions", "skills", "onboarding"]
```

---

## 5. Testing Requirements

A feature-complete client SHOULD:

1. Pass the **gateway protocol conformance tests** in `tests/gateway_protocol.rs`
2. Handle all `ServerFrameType` variants without panicking
3. Correctly serialize all `ClientFrameType` variants
4. Survive gateway disconnection and reconnection
5. Handle concurrent tool calls (multiple `ToolApproval` frames in flight)

---

## 6. Reference Implementation

The reference client is `crates/rustyclaw-tui/`. Study it for:

- Gateway WebSocket lifecycle (`app/app.rs`)
- Message rendering (`panes/messages.rs`)
- Tool approval flow (`dialogs/tool_approval.rs`)
- Onboarding wizard (`onboard.rs`)
- Keyboard/mouse event handling (`app/handlers/`)
- Theme and styling (`theme::tui_palette`)

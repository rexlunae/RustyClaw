# Messenger Architecture

**Document Status**: Implementation Guide
**Last Updated**: 2026-02-16
**Related Issues**: #95-#102 (Messenger integrations)

---

## Overview

RustyClaw uses a **channel-adapter architecture** with a unified `Messenger` trait. This design provides a consistent interface for integrating with 16+ messaging platforms while allowing protocol-specific implementations.

## Architecture Principles

1. **Unified Interface**: All messengers implement the same `Messenger` trait
2. **Factory Pattern**: Single `create_messenger()` function handles instantiation
3. **Normalized Config**: Shared config schema with protocol-specific extensions
4. **Protocol Adapters**: Each channel has its own module (e.g., `irc.rs`, `whatsapp.rs`)
5. **Backward Compatibility**: Extensions don't break existing gateway loop

---

## Core Components

### 1. Messenger Trait

Location: `src/messengers/mod.rs`

```rust
#[async_trait]
pub trait Messenger: Send + Sync {
    /// Get the messenger display name
    fn name(&self) -> &str;

    /// Get the messenger type (telegram, discord, irc, etc.)
    fn messenger_type(&self) -> &str;

    /// Initialize the messenger (connect, authenticate)
    async fn initialize(&mut self) -> Result<()>;

    /// Send a simple message
    async fn send_message(&self, recipient: &str, content: &str) -> Result<String>;

    /// Send a message with options (reply, media, etc.)
    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String>;

    /// Receive pending messages (non-blocking poll)
    async fn receive_messages(&self) -> Result<Vec<Message>>;

    /// Check connection status
    fn is_connected(&self) -> bool;

    /// Disconnect gracefully
    async fn disconnect(&mut self) -> Result<()>;

    /// Optional: Set typing indicator
    async fn set_typing(&self, _channel: &str, _typing: bool) -> Result<()> {
        Ok(()) // Default no-op
    }

    /// Optional: Update presence status
    async fn set_presence(&self, _status: &str) -> Result<()> {
        Ok(()) // Default no-op
    }
}
```

**Key Design Decisions**:
- `async_trait` for async support across all implementations
- Default implementations for optional features (typing, presence)
- `Send + Sync` for thread-safe use in tokio runtime
- Return type flexibility (message IDs as `String`)

### 2. Messenger Config Schema

Location: `src/config.rs`

```rust
pub struct MessengerConfig {
    /// Display name for this messenger instance
    pub name: String,

    /// Messenger type: telegram, discord, slack, whatsapp, google-chat,
    /// teams, mattermost, irc, xmpp, signal, matrix, webhook, gmail, etc.
    pub messenger_type: String,

    /// Whether this messenger is enabled
    pub enabled: bool,

    /// Path to external config file (optional)
    pub config_path: Option<PathBuf>,

    // ── Shared authentication fields ──
    pub token: Option<String>,                  // Bot/API tokens
    pub webhook_url: Option<String>,            // Webhook endpoints
    pub password: Option<String>,               // Matrix, IRC auth

    // ── API configuration ──
    pub base_url: Option<String>,               // API base URL
    pub api_version: Option<String>,            // API version (e.g., "v20.0")

    // ── Channel/Space identifiers ──
    pub channel_id: Option<String>,             // Default channel
    pub team_id: Option<String>,                // Teams workspace ID
    pub space: Option<String>,                  // Google Chat space
    pub phone_number_id: Option<String>,        // WhatsApp phone number

    // ── IRC-specific fields ──
    pub server: Option<String>,                 // IRC server hostname
    pub port: Option<u16>,                      // IRC server port
    pub nickname: Option<String>,               // IRC nickname
    pub username: Option<String>,               // IRC username
    pub realname: Option<String>,               // IRC real name

    // ── Matrix-specific fields ──
    pub homeserver: Option<String>,             // Matrix homeserver URL
    pub user_id: Option<String>,                // Matrix user ID
    pub access_token: Option<String>,           // Matrix access token

    // ── Gmail/OAuth fields ──
    pub client_id: Option<String>,              // OAuth client ID
    pub client_secret: Option<String>,          // OAuth client secret

    // ── Misc ──
    pub from: Option<String>,                   // Generic sender ID
    pub default_recipient: Option<String>,      // Default recipient
    pub phone: Option<String>,                  // Signal phone number
}
```

**Design Philosophy**:
- **Flat schema**: All fields at root level for simplicity
- **Optional fields**: Only populate what each protocol needs
- **Environment fallback**: Factory checks env vars if config fields missing
- **Extensible**: Easy to add new fields for future protocols

### 3. Messenger Factory

Location: `src/gateway/messenger_handler.rs`

```rust
/// Create a messenger manager from config
pub async fn create_messenger_manager(config: &Config) -> Result<MessengerManager> {
    let mut manager = MessengerManager::new();

    for messenger_config in &config.messengers {
        if !messenger_config.enabled {
            continue;
        }
        match create_messenger(messenger_config).await {
            Ok(messenger) => {
                eprintln!("[messenger] Initialized {} ({})",
                    messenger.name(), messenger.messenger_type());
                manager.add_messenger(messenger);
            }
            Err(e) => {
                eprintln!("[messenger] Failed to initialize {}: {}",
                    messenger_config.messenger_type, e);
            }
        }
    }

    Ok(manager)
}

/// Create a single messenger from config
async fn create_messenger(config: &MessengerConfig) -> Result<Box<dyn Messenger>> {
    let name = config.name.clone();
    let mut messenger: Box<dyn Messenger> = match config.messenger_type.as_str() {
        "telegram" => { /* ... */ },
        "discord" => { /* ... */ },
        "slack" => { /* ... */ },
        "whatsapp" => { /* ... */ },
        "google-chat" => { /* ... */ },
        "teams" => { /* ... */ },
        "mattermost" => { /* ... */ },
        "irc" => { /* ... */ },
        "xmpp" => { /* ... */ },
        "signal" => { /* ... */ },
        "matrix" => { /* ... */ },
        "gmail" => { /* ... */ },
        "webhook" => { /* ... */ },
        "console" => { /* ... */ },
        _ => anyhow::bail!("Unknown messenger type: {}", config.messenger_type),
    };

    messenger.initialize().await?;
    Ok(messenger)
}
```

**Factory Responsibilities**:
1. **Config resolution**: Check config fields, then environment variables
2. **Validation**: Ensure required fields are present
3. **Protocol-specific config**: Build channel-specific config structs
4. **Initialization**: Call `initialize()` before returning
5. **Error handling**: Return descriptive errors for debugging

---

## Protocol Adapter Patterns

### Pattern 1: Native TCP (IRC, XMPP)

**Example**: `src/messengers/irc.rs`

```rust
pub struct IrcMessenger {
    name: String,
    config: IrcConfig,
    stream: Arc<Mutex<Option<TcpStream>>>,  // Direct TCP connection
    pending: Arc<Mutex<String>>,             // Line buffer
    connected: AtomicBool,
}

impl IrcMessenger {
    async fn send_raw_line(&self, line: &str) -> Result<()> {
        let mut guard = self.stream.lock().await;
        let stream = guard.as_mut().context("IRC not connected")?;
        stream.write_all(format!("{}\r\n", line).as_bytes()).await?;
        stream.flush().await?;
        Ok(())
    }
}
```

**Characteristics**:
- Direct socket management
- Protocol-specific line parsing (IRC uses `\r\n`)
- Handle protocol commands (PING/PONG for IRC)
- Manual buffering for incomplete messages

### Pattern 2: HTTP API + Webhook (WhatsApp, Google Chat, Teams)

**Example**: `src/messengers/whatsapp.rs`

```rust
pub struct WhatsAppMessenger {
    name: String,
    config: WhatsAppConfig,
    http: reqwest::Client,              // HTTP client
    connected: bool,
}

impl WhatsAppMessenger {
    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        let url = self.api_url("messages");
        let resp = self.http
            .post(&url)
            .bearer_auth(&self.config.token)
            .json(&serde_json::json!({
                "messaging_product": "whatsapp",
                "to": recipient,
                "type": "text",
                "text": { "body": content }
            }))
            .send()
            .await?;

        // Return message ID from response
        let data: SendResponse = resp.json().await?;
        Ok(data.messages.first().map(|m| m.id.clone())
            .unwrap_or_else(|| "unknown".to_string()))
    }
}
```

**Characteristics**:
- `reqwest::Client` for HTTP requests
- Bearer token or API key authentication
- JSON request/response bodies
- **Inbound messages via webhook** (not polling)
- `receive_messages()` returns empty (webhook-driven)

### Pattern 3: WebSocket (Discord, Slack real-time)

**Example**: `src/messengers/discord.rs`

```rust
pub struct DiscordMessenger {
    name: String,
    token: String,
    http: reqwest::Client,              // HTTP for sending
    gateway: Arc<Mutex<Option<GatewayConnection>>>,  // WS for receiving
    intents: GatewayIntents,
}

impl DiscordMessenger {
    async fn connect_gateway(&mut self) -> Result<()> {
        let ws_url = self.get_gateway_url().await?;
        let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await?;

        // Spawn background task for heartbeat + event processing
        let gateway = GatewayConnection::new(ws_stream, self.token.clone());
        gateway.start_heartbeat();

        *self.gateway.lock().await = Some(gateway);
        Ok(())
    }
}
```

**Characteristics**:
- HTTP for outbound (POST messages)
- WebSocket for inbound (event stream)
- Background tasks for heartbeat/reconnection
- Event parsing (JSON payloads)
- More complex lifecycle management

### Pattern 4: SDK/Library (Matrix, Signal)

**Example**: `src/messengers/matrix.rs`

```rust
#[cfg(feature = "matrix")]
pub struct MatrixMessenger {
    name: String,
    client: matrix_sdk::Client,        // Use official SDK
    config: MatrixConfig,
}

impl MatrixMessenger {
    async fn initialize(&mut self) -> Result<()> {
        self.client = Client::new(self.config.homeserver.parse()?).await?;

        if let Some(token) = &self.config.access_token {
            self.client.restore_login(token).await?;
        } else if let Some(password) = &self.config.password {
            self.client.login(&self.config.user_id, password, None, None).await?;
        }

        self.client.sync_once(SyncSettings::default()).await?;
        Ok(())
    }
}
```

**Characteristics**:
- Feature-gated (`#[cfg(feature = "matrix")]`)
- Use official SDK (e.g., `matrix-sdk`, `libsignal`)
- SDK handles protocol complexity
- Wrap SDK methods in `Messenger` trait

---

## Configuration Examples

### IRC

```toml
[[messengers]]
name = "irc-libera"
messenger_type = "irc"
enabled = true
server = "irc.libera.chat"
port = 6667
nickname = "rustyclaw"
username = "rustyclaw"
realname = "RustyClaw Agent"
channel_id = "#rustyclaw"
password = "$NICKSERV_PASSWORD"  # Optional NickServ auth
```

### WhatsApp Cloud API

```toml
[[messengers]]
name = "whatsapp-support"
messenger_type = "whatsapp"
enabled = true
token = "$WHATSAPP_ACCESS_TOKEN"
phone_number_id = "1234567890"
api_version = "v20.0"
```

### Google Chat

```toml
[[messengers]]
name = "gchat-team"
messenger_type = "google-chat"
enabled = true
token = "$GOOGLE_CHAT_BOT_TOKEN"
space = "spaces/AAAA..."
base_url = "https://chat.googleapis.com/v1"
```

### Teams

```toml
[[messengers]]
name = "teams-sales"
messenger_type = "teams"
enabled = true
token = "$TEAMS_BOT_TOKEN"
team_id = "12345678-1234-1234-1234-123456789012"
channel_id = "19:abcd..."
base_url = "https://graph.microsoft.com/v1.0"
```

### Mattermost

```toml
[[messengers]]
name = "mattermost-dev"
messenger_type = "mattermost"
enabled = true
token = "$MATTERMOST_TOKEN"
base_url = "https://mattermost.example.com"
channel_id = "channel-id-here"
```

### XMPP

```toml
[[messengers]]
name = "xmpp-internal"
messenger_type = "xmpp"
enabled = true
from = "rustyclaw@example.com"
password = "$XMPP_PASSWORD"
server = "xmpp.example.com"
port = 5222
```

---

## Adding a New Messenger

### Step 1: Create Adapter Module

Create `src/messengers/mynew.rs`:

```rust
use super::{Message, Messenger, SendOptions};
use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct MyNewConfig {
    pub api_key: String,
    pub base_url: String,
    // ... protocol-specific fields
}

pub struct MyNewMessenger {
    name: String,
    config: MyNewConfig,
    client: reqwest::Client,
    connected: bool,
}

impl MyNewMessenger {
    pub fn new(name: String, config: MyNewConfig) -> Self {
        Self {
            name,
            config,
            client: reqwest::Client::new(),
            connected: false,
        }
    }
}

#[async_trait]
impl Messenger for MyNewMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "mynew"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Implement connection/authentication
        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        // Implement message sending
        Ok("message-id".to_string())
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // Implement message polling or return empty for webhook-driven
        Ok(Vec::new())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
}
```

### Step 2: Export from `mod.rs`

Add to `src/messengers/mod.rs`:

```rust
mod mynew;
pub use mynew::{MyNewConfig, MyNewMessenger};
```

### Step 3: Add to Factory

Add case to `create_messenger()` in `src/gateway/messenger_handler.rs`:

```rust
"mynew" => {
    let api_key = config
        .token
        .clone()
        .or_else(|| std::env::var("MYNEW_API_KEY").ok())
        .context("MyNew requires 'token' or MYNEW_API_KEY env var")?;

    let base_url = config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.mynew.com".to_string());

    let mynew_config = MyNewConfig {
        api_key,
        base_url,
    };

    Box::new(MyNewMessenger::new(name, mynew_config))
}
```

### Step 4: Add Import

Add to imports in `messenger_handler.rs`:

```rust
use crate::messengers::{
    // ... existing imports
    MyNewConfig, MyNewMessenger,
};
```

### Step 5: Document

Create `docs/MESSENGER_MYNEW.md` with:
- API documentation links
- Authentication setup
- Configuration examples
- Supported features
- Limitations

---

## Testing Strategy

### Unit Tests

Each messenger module should have tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_messenger_type() {
        let m = MyNewMessenger::new("test".to_string(), MyNewConfig {
            api_key: "test".to_string(),
            base_url: "https://test.com".to_string(),
        });
        assert_eq!(m.messenger_type(), "mynew");
    }

    #[tokio::test]
    async fn test_send_message() {
        // Mock HTTP client or use test API
        // Test message sending
    }
}
```

### Integration Tests

Location: `tests/messengers/`

```rust
#[tokio::test]
#[ignore] // Only run with real credentials
async fn test_mynew_real_connection() {
    let api_key = std::env::var("MYNEW_API_KEY").unwrap();
    let config = MyNewConfig {
        api_key,
        base_url: "https://api.mynew.com".to_string(),
    };

    let mut messenger = MyNewMessenger::new("test".to_string(), config);
    assert!(messenger.initialize().await.is_ok());
    assert!(messenger.is_connected());
}
```

---

## Performance Considerations

### Polling Efficiency

```rust
async fn receive_messages(&self) -> Result<Vec<Message>> {
    // Use short timeouts to avoid blocking gateway loop
    tokio::time::timeout(
        Duration::from_millis(100),
        self.poll_api()
    ).await.unwrap_or_else(|_| Ok(Vec::new()))
}
```

### Connection Pooling

```rust
// Reuse HTTP clients (already pooled by reqwest)
pub struct MyMessenger {
    client: reqwest::Client,  // ✅ Reused across requests
}

// Avoid creating new clients per request
async fn send_message(&self) -> Result<()> {
    let client = reqwest::Client::new();  // ❌ Don't do this
    // ...
}
```

### Rate Limiting

```rust
use tokio::time::{sleep, Duration};

async fn send_with_rate_limit(&self, msg: &str) -> Result<()> {
    // Implement per-channel rate limits
    self.rate_limiter.wait().await;
    self.send_message_internal(msg).await
}
```

---

## Security Best Practices

### 1. Credential Storage

```rust
// ✅ Use encrypted secrets vault (issue #86)
let token = vault.get_secret("mynew_api_key").await?;

// ❌ Don't hardcode credentials
let token = "sk-abc123...";  // NEVER do this
```

### 2. Input Sanitization

```rust
async fn send_message(&self, content: &str) -> Result<String> {
    // Sanitize content to prevent injection
    let safe_content = content
        .replace('\r', " ")
        .replace('\n', " ")
        .trim();

    // Validate length
    if safe_content.len() > MAX_MESSAGE_LENGTH {
        anyhow::bail!("Message too long");
    }

    self.send_internal(safe_content).await
}
```

### 3. TLS Verification

```rust
// ✅ Enable TLS verification by default
let client = reqwest::Client::builder()
    .https_only(true)
    .build()?;

// ❌ Don't disable certificate verification
let client = reqwest::Client::builder()
    .danger_accept_invalid_certs(true)  // NEVER in production
    .build()?;
```

---

## Related Documentation

- **[Configuration Guide](./configuration.md)** — Config file format
- **[Gateway Protocol](./gateway.md)** — WebSocket gateway API
- **[Messenger Integrations](./MESSENGERS.md)** — Setup guides for each platform
- **[Security Model](./SECURITY.md)** — Security architecture

---

## Messenger Status Matrix

| Messenger | Status | Module | Protocol | Config Complexity |
|-----------|--------|--------|----------|-------------------|
| Telegram | ✅ Implemented | `telegram.rs` | HTTP + Long polling | Low |
| Discord | ✅ Implemented | `discord.rs` | HTTP + WebSocket | Medium |
| Slack | ✅ Implemented | `slack.rs` | HTTP + WebSocket | Medium |
| WhatsApp | ✅ Implemented | `whatsapp.rs` | HTTP (Cloud API) | Medium |
| Google Chat | ✅ Implemented | `google_chat.rs` | HTTP + Webhook | Medium |
| Teams | ✅ Implemented | `teams.rs` | HTTP (Graph API) | Medium |
| Mattermost | ✅ Implemented | `mattermost.rs` | HTTP + Webhook | Low |
| IRC | ✅ Implemented | `irc.rs` | Native TCP | Low |
| XMPP | ✅ Implemented | `xmpp.rs` | Native TCP | Medium |
| Signal | ✅ Implemented | `signal.rs` | libsignal (feature-gated) | High |
| Matrix | ✅ Implemented | `matrix.rs` | matrix-sdk (feature-gated) | Medium |
| Gmail | ✅ Implemented | `gmail.rs` | Gmail API + OAuth | High |
| Webhook | ✅ Implemented | `webhook.rs` | HTTP POST | Low |
| Console | ✅ Implemented | `console.rs` | Terminal I/O | Low |
| **BlueBubbles/iMessage** | 📋 Planned (#95) | — | HTTP (BlueBubbles API) | Medium |
| **Nextcloud Talk** | 📋 Planned (#96) | — | HTTP (Nextcloud API) | Medium |
| **Nostr** | 📋 Planned (#97) | — | WebSocket (Relays) | Low |
| **Urbit/Tlon** | 📋 Planned (#98) | — | HTTP (Airlock SSE) | High |
| **Twitch** | 📋 Planned (#99) | — | IRC protocol | Low |
| **Zalo Official** | 📋 Planned (#100) | — | HTTP (Zalo OA API) | Medium |
| **Zalo Personal** | 📋 Planned (#101) | — | Browser automation | Very High |
| **WeChat/WeCom** | 📋 Planned (#102) | — | HTTP (WeChat API) | High |
| **Feishu/Lark** | 📋 Planned (#79) | — | HTTP (Feishu API) | Medium |
| **LINE** | 📋 Planned (#80) | — | HTTP (LINE Bot API) | Medium |

**Legend**:
- ✅ **Implemented** - Production-ready, tested
- 📋 **Planned** - GitHub issue created, design complete
- 🚧 **In Progress** - Currently being developed
- ⚠️ **Deprecated** - Legacy, not recommended

---

**Last Updated**: 2026-02-16
**Maintainers**: [@aecs4u](https://github.com/aecs4u)
**Questions?**: [Open a discussion](https://github.com/aecs4u/RustyClaw/discussions)

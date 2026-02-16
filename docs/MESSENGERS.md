# Messenger Channel Integrations

RustyClaw can be integrated with multiple messaging platforms, allowing your AI assistant to be accessible wherever your team communicates.

## Supported Platforms

RustyClaw supports 4 major messaging platforms, each available as an optional feature branch:

| Platform | Status | Feature Branch | Documentation |
|----------|--------|----------------|---------------|
| **Slack** | ✅ Ready | `feature/messenger-slack` | [Setup Guide](./MESSENGER_SLACK.md) |
| **Discord** | ✅ Ready | `feature/messenger-discord` | [Setup Guide](./MESSENGER_DISCORD.md) |
| **Telegram** | ✅ Ready | `feature/messenger-telegram` | [Setup Guide](./MESSENGER_TELEGRAM.md) |
| **Matrix** | ✅ Ready | `feature/messenger-matrix` | [Setup Guide](./MESSENGER_MATRIX.md) |

## Quick Comparison

### Platform Overview

| Feature | Slack | Discord | Telegram | Matrix |
|---------|-------|---------|----------|--------|
| **Setup Difficulty** | Medium | Medium | Easy | Medium |
| **Open Source** | ❌ No | ❌ No | ⚠️ Partial | ✅ Yes |
| **Self-Hosted** | ❌ No | ❌ No | ❌ No | ✅ Yes |
| **Free Tier** | Limited | Good | Unlimited | Unlimited |
| **API Quality** | Excellent | Excellent | Excellent | Good |
| **E2EE** | ⚠️ Enterprise | ⚠️ Partial | ⚠️ Secret Chats | ✅ Built-in |
| **Rate Limits** | Strict | Medium | Generous | Generous |
| **Group Support** | Excellent | Excellent | Excellent | Excellent |

### Technical Comparison

| Feature | Slack | Discord | Telegram | Matrix |
|---------|-------|---------|----------|--------|
| **Connection** | Socket Mode / Webhooks | Gateway WebSocket | Long Polling / Webhooks | Client SDK |
| **Authentication** | Bot Token + App Token | Bot Token | Bot Token | Username + Password/Token |
| **Message Format** | Block Kit JSON | Plain text / Embeds | Markdown / HTML | Markdown / HTML |
| **File Uploads** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes |
| **Threading** | ✅ Yes | ✅ Reply chains | ✅ Reply chains | ✅ Threading |
| **Reactions** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes |
| **Commands** | ✅ Slash commands | ✅ Slash commands | ✅ Bot commands | ⚠️ Limited |

### Use Cases

**Slack** - Best for:
- Corporate/enterprise teams
- Professional environments
- Integration-heavy workflows
- Apps already using Slack

**Discord** - Best for:
- Gaming communities
- Developer communities
- Real-time collaboration
- Voice channel integration

**Telegram** - Best for:
- Quick setup
- High-volume messaging
- Mobile-first users
- International teams

**Matrix** - Best for:
- Privacy-focused teams
- Self-hosted deployments
- Federated communities
- Open source projects

## Getting Started

### 1. Choose Your Platform(s)

You can enable one or multiple messenger integrations. Each is independent and can be configured separately.

### 2. Checkout Feature Branch

```bash
# For Slack
git checkout feature/messenger-slack

# For Discord
git checkout feature/messenger-discord

# For Telegram
git checkout feature/messenger-telegram

# For Matrix
git checkout feature/messenger-matrix
```

### 3. Build with Messenger Support

```bash
# Slack
cargo build --release --features messenger-slack

# Discord
cargo build --release --features messenger-discord

# Telegram
cargo build --release --features messenger-telegram

# Matrix
cargo build --release --features matrix
```

### 4. Configure

Add configuration to `~/.rustyclaw/config.toml`:

**Slack**:
```toml
[slack]
bot_token = "xoxb-your-token"
app_token = "xapp-your-token"
signing_secret = "your-secret"
socket_mode = true
```

**Discord**:
```toml
[discord]
bot_token = "your-token"
application_id = "your-app-id"
command_prefix = "!"
```

**Telegram**:
```toml
[telegram]
bot_token = "123456:ABC-DEF..."
poll_interval_secs = 1
```

**Matrix**:
```toml
[matrix_messenger]
homeserver_url = "https://matrix.org"
username = "@botname:matrix.org"
password = "your-token"
```

### 5. Start Gateway

```bash
rustyclaw gateway start
```

## Architecture

### Common Messenger Trait

All messenger integrations implement a common trait:

```rust
#[async_trait]
pub trait Messenger: Send + Sync {
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn send_message(&self, channel_id: &str, message: &str) -> Result<()>;
    fn platform_name(&self) -> &str;
}
```

### Message Flow

```
User Message (Platform)
    ↓
Messenger Integration
    ↓
MessengerEvent
    ↓
Gateway Event Handler
    ↓
AI Assistant Processing
    ↓
Response Generation
    ↓
Messenger Integration
    ↓
Platform Message (User)
```

### Event Types

```rust
pub enum MessengerEvent {
    Message(MessengerMessage),
    Reaction { message_id: String, emoji: String, user_id: String },
    Join { channel_id: String, user_id: String },
    Leave { channel_id: String, user_id: String },
}
```

## Security

### Token Storage

All messenger integrations support secure token storage:

```bash
# Store tokens in secrets vault
rustyclaw secrets set SLACK_BOT_TOKEN "xoxb-..."
rustyclaw secrets set DISCORD_BOT_TOKEN "..."
rustyclaw secrets set TELEGRAM_BOT_TOKEN "..."
rustyclaw secrets set MATRIX_PASSWORD "..."

# Reference in config
bot_token = "${SLACK_BOT_TOKEN}"
```

### Webhook Verification

- **Slack**: HMAC-SHA256 signature verification
- **Discord**: Bot token authentication
- **Telegram**: Bot token authentication
- **Matrix**: Access token authentication

### TLS/Encryption

- **Slack**: WSS (WebSocket Secure) or HTTPS webhooks
- **Discord**: WSS Gateway connection
- **Telegram**: HTTPS API calls
- **Matrix**: HTTPS + optional E2EE (Olm/Megolm)

## Multi-Platform Support

You can run multiple messenger integrations simultaneously:

```toml
# Enable all platforms
[slack]
enabled = true
bot_token = "..."

[discord]
enabled = true
bot_token = "..."

[telegram]
enabled = true
bot_token = "..."

[matrix_messenger]
enabled = true
homeserver_url = "..."
```

Build with all features:
```bash
cargo build --release --features messenger-slack,messenger-discord,messenger-telegram,matrix
```

## Performance

### Memory Usage

Per messenger connection:
- **Slack**: ~5-10MB
- **Discord**: ~10-20MB
- **Telegram**: ~5-10MB
- **Matrix**: ~15-30MB (with E2EE)

### Latency

Average response times:
- **Slack**: 100-300ms
- **Discord**: 100-300ms
- **Telegram**: 100-500ms (polling) / 50-200ms (webhooks)
- **Matrix**: 200-800ms

### Scalability

- **Concurrent connections**: Limited by system resources
- **Messages per second**: Depends on platform rate limits
- **Recommended**: 1 bot instance per 100-500 active users

## Best Practices

### 1. Use Appropriate Platform

- **Internal teams**: Slack or Matrix
- **Communities**: Discord or Telegram
- **Privacy-focused**: Matrix with E2EE
- **Mobile-first**: Telegram

### 2. Configure Rate Limiting

Respect platform rate limits to avoid throttling:
- Implement message queuing for high-traffic scenarios
- Use batching where supported
- Monitor metrics for rate limit errors

### 3. Secure Credentials

- Never commit tokens to version control
- Use secrets vault for production
- Rotate tokens periodically
- Enable 2FA on platform accounts

### 4. Monitor Health

Use RustyClaw's built-in metrics:
```bash
curl http://localhost:9090/metrics | grep messenger
```

Metrics available:
- `rustyclaw_messenger_connections` - Active connections
- `rustyclaw_messenger_messages_total` - Messages processed
- `rustyclaw_messenger_errors_total` - Error count

### 5. Implement Fallbacks

Consider having multiple messenger options:
- Primary: Slack (for team)
- Secondary: Discord (for community)
- Fallback: Telegram (for mobile)

## Troubleshooting

### Common Issues

**Bot not responding**:
1. Check bot is online/connected
2. Verify permissions in platform settings
3. Check gateway logs: `tail -f ~/.rustyclaw/logs/gateway.log`
4. Test with direct mention

**Connection issues**:
1. Verify token is valid
2. Check network connectivity
3. Ensure no firewall blocking
4. Check platform status pages

**Rate limiting**:
1. Reduce message frequency
2. Implement message queue
3. Use batching where possible
4. Monitor rate limit headers

### Debug Mode

Enable debug logging:
```toml
[logging]
level = "debug"
```

View messenger-specific logs:
```bash
grep "\[slack\]" ~/.rustyclaw/logs/gateway.log
grep "\[discord\]" ~/.rustyclaw/logs/gateway.log
grep "\[telegram\]" ~/.rustyclaw/logs/gateway.log
grep "\[matrix\]" ~/.rustyclaw/logs/gateway.log
```

## Examples

### Multi-Platform Bot

```toml
# Respond on all platforms
[slack]
bot_token = "${SLACK_BOT_TOKEN}"
socket_mode = true

[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
application_id = "123456789"

[telegram]
bot_token = "${TELEGRAM_BOT_TOKEN}"

[matrix_messenger]
homeserver_url = "https://matrix.org"
username = "@rustyclaw:matrix.org"
password = "${MATRIX_PASSWORD}"
```

### Bridge Platforms

Use Matrix bridges to connect platforms:
- matrix-appservice-slack
- matrix-appservice-discord
- mautrix-telegram

Bot becomes accessible across all platforms through Matrix.

## Migration

### From Single Platform

Moving from one platform to another is straightforward:

1. Set up new platform configuration
2. Test with small group
3. Migrate users gradually
4. Maintain both during transition
5. Deprecate old platform

### Export/Import

Bot conversation history can be exported:
```bash
rustyclaw messenger export --platform slack --output history.json
rustyclaw messenger import --platform discord --input history.json
```

(Feature planned for future release)

## Roadmap

### Planned Features

- [ ] **WhatsApp** integration (via Business API)
- [ ] **Microsoft Teams** integration
- [ ] **IRC** bridge support
- [ ] **Signal** integration (via libsignal)
- [ ] **Mattermost** integration
- [ ] Unified message formatting
- [ ] Cross-platform conversation linking
- [ ] Message history export/import
- [ ] Rich media support (images, files, voice)
- [ ] Slash command registration automation

### Contributions

Messenger integrations are on feature branches and ready for testing. To contribute:

1. Checkout feature branch
2. Test with your platform account
3. Report issues on GitHub
4. Submit improvements via PR

## Related Documentation

- [Slack Integration Guide](./MESSENGER_SLACK.md)
- [Discord Integration Guide](./MESSENGER_DISCORD.md)
- [Telegram Integration Guide](./MESSENGER_TELEGRAM.md)
- [Matrix Integration Guide](./MESSENGER_MATRIX.md)
- [Gateway Configuration](./HOT_RELOAD.md)
- [Security Features](../README.md#security)

## Support

For issues or questions:
- GitHub Issues: https://github.com/aecs4u/RustyClaw/issues
- Discussion: Use platform-specific channels
- Documentation: See individual messenger guides

---

**Note**: Messenger integrations are available on feature branches and will be merged to main after community testing and feedback.

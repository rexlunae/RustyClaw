# Slack Messenger Integration

RustyClaw can be integrated with Slack workspaces, allowing your AI assistant to respond to messages in Slack channels and direct messages.

## Features

- 🤖 **Slack Bot Integration** - Interact with RustyClaw through Slack
- 💬 **Channel & DM Support** - Works in channels and direct messages
- 🔒 **Secure Webhooks** - Request signature verification
- 🧵 **Thread Support** - Maintains conversation context in threads
- ⚡ **Socket Mode** - Real-time bi-directional communication
- 🔧 **Easy Configuration** - Simple TOML-based setup

## Setup

### 1. Create a Slack App

1. Go to https://api.slack.com/apps
2. Click **"Create New App"** → **"From scratch"**
3. Name your app (e.g., "RustyClaw") and select your workspace
4. Click **"Create App"**

### 2. Configure Bot Permissions

1. Go to **"OAuth & Permissions"** in the sidebar
2. Scroll to **"Scopes"** → **"Bot Token Scopes"**
3. Add these permissions:
   - `chat:write` - Send messages
   - `channels:history` - Read channel messages
   - `groups:history` - Read private channel messages
   - `im:history` - Read DM messages
   - `mpim:history` - Read group DM messages
   - `app_mentions:read` - Read mentions

### 3. Install App to Workspace

1. Scroll to **"OAuth Tokens for Your Workspace"**
2. Click **"Install to Workspace"**
3. Authorize the app
4. Copy the **"Bot User OAuth Token"** (starts with `xoxb-`)

### 4. Enable Events (HTTP Webhooks)

If using HTTP webhooks (not Socket Mode):

1. Go to **"Event Subscriptions"**
2. Toggle **"Enable Events"** to On
3. Set **Request URL** to: `https://your-domain.com/slack/events`
4. Subscribe to bot events:
   - `message.channels`
   - `message.groups`
   - `message.im`
   - `message.mpim`
   - `app_mention`
5. Save changes

### 5. Get Signing Secret

1. Go to **"Basic Information"**
2. Scroll to **"App Credentials"**
3. Copy the **"Signing Secret"**

### 6. Enable Socket Mode (Recommended)

Socket Mode avoids needing a public webhook URL:

1. Go to **"Socket Mode"**
2. Toggle **"Enable Socket Mode"** to On
3. Under **"Event Subscriptions"**, enable events if not already done
4. Generate an **App-Level Token** with `connections:write` scope
5. Copy the token (starts with `xapp-`)

## Configuration

Add to `~/.rustyclaw/config.toml`:

```toml
[slack]
# Bot token (required)
bot_token = "xoxb-your-bot-token-here"

# App token for Socket Mode (recommended)
app_token = "xapp-your-app-token-here"

# Signing secret for webhook verification (required for HTTP mode)
signing_secret = "your-signing-secret"

# Use Socket Mode (recommended, no public URL needed)
socket_mode = true

# HTTP webhook address (only if socket_mode = false)
webhook_addr = "0.0.0.0:3000"
```

### Socket Mode (Recommended)

```toml
[slack]
bot_token = "xoxb-YOUR-BOT-TOKEN-HERE"
app_token = "xapp-YOUR-APP-TOKEN-HERE"
signing_secret = "your-signing-secret-here"
socket_mode = true
```

### HTTP Webhook Mode

Requires public URL with TLS:

```toml
[slack]
bot_token = "xoxb-YOUR-BOT-TOKEN-HERE"
signing_secret = "your-signing-secret-here"
socket_mode = false
webhook_addr = "0.0.0.0:3000"
```

Then set up reverse proxy (nginx, Caddy) to route `https://your-domain.com/slack/events` → `http://localhost:3000/slack/events`

## Building with Slack Support

Slack integration is an optional feature. Build with:

```bash
cargo build --release --features messenger-slack
```

Or add to your default features in `Cargo.toml`:

```toml
[features]
default = ["tui", "web-tools", "messenger-slack"]
```

## Usage

### Start Gateway with Slack

```bash
rustyclaw gateway start --features messenger-slack
```

The gateway will:
1. Connect to Slack using Socket Mode or start webhook server
2. Listen for messages mentioning the bot or in channels it's invited to
3. Process messages through RustyClaw's AI assistant
4. Respond in the same channel/thread

### Invite Bot to Channels

In Slack:
1. Go to any channel
2. Type `/invite @RustyClaw` (or your bot name)
3. Start messaging!

### Direct Messages

1. Open DMs in Slack
2. Search for your bot name
3. Start a conversation

### Using in Threads

The bot automatically maintains context in threads:
- Reply to a message to start a thread
- Bot will respond in the same thread
- Each thread maintains separate conversation context

## Message Format

### Mentioning the Bot

```
@RustyClaw what's the weather today?
```

### Direct Messages

```
Can you help me write a Python script?
```

### In Channels

Bot responds when:
- Directly mentioned with `@BotName`
- In DMs
- (Optional) All channel messages if configured

## Advanced Configuration

### Custom Response Formatting

Messages are formatted using Slack's Block Kit. You can customize:

```rust
// In your custom messenger implementation
let blocks = vec![
    Block::Section(SectionBlock {
        text: Text::Markdown("*Response:*\n{}".to_string()),
        ..Default::default()
    }),
];
```

### Rate Limiting

Slack enforces rate limits:
- 1 request per second per channel (tier 2)
- Burst allowance of ~30 requests

RustyClaw automatically handles rate limiting with exponential backoff.

### User Context

Each message includes:
- `user_id` - Slack user ID (U0123456789)
- `channel_id` - Slack channel ID (C0123456789)
- `thread_ts` - Thread timestamp (for maintaining context)

## Security

### Request Verification

All incoming webhooks are verified using HMAC-SHA256:

1. Slack sends `X-Slack-Signature` and `X-Slack-Request-Timestamp` headers
2. RustyClaw computes HMAC signature using signing secret
3. Mismatched signatures are rejected

### Token Storage

Store tokens securely:
- Use environment variables
- Use RustyClaw secrets vault: `rustyclaw secrets set SLACK_BOT_TOKEN "xoxb-..."`
- Reference in config: `bot_token = "${SLACK_BOT_TOKEN}"`

### Private Channels

Bot can only access:
- Public channels it's invited to
- Private channels it's explicitly invited to
- Direct messages with users

## Troubleshooting

### Bot Not Responding

1. **Check bot is invited to channel**:
   ```
   /invite @RustyClaw
   ```

2. **Verify tokens are correct**:
   ```bash
   curl -H "Authorization: Bearer xoxb-your-token" \
        https://slack.com/api/auth.test
   ```

3. **Check gateway logs**:
   ```bash
   tail -f ~/.rustyclaw/logs/gateway.log
   ```

### Socket Mode Connection Issues

- Ensure `app_token` is valid and has `connections:write` scope
- Check firewall allows outbound WebSocket connections
- Verify no proxy blocking WSS traffic

### Webhook URL Verification Failed

- Ensure webhook URL is publicly accessible
- Must use HTTPS (not HTTP)
- Check reverse proxy configuration
- Verify signing secret matches

### Rate Limiting

If you see `429 Too Many Requests`:
- Reduce message frequency
- Use thread_ts to keep conversations in threads
- Implement message queuing

## Examples

### Simple Q&A

**User in Slack**: `@RustyClaw what's the capital of France?`

**Bot Response**: `The capital of France is Paris. It's the largest city in France and has been the capital since 987 CE.`

### Code Generation

**User**: `@RustyClaw write a Python function to calculate fibonacci`

**Bot Response**:
````python
def fibonacci(n):
    if n <= 1:
        return n
    return fibonacci(n-1) + fibonacci(n-2)

# Iterative version (more efficient):
def fibonacci_iter(n):
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a
````

### Multi-turn Conversation

**User**: `@RustyClaw I need help with my React app`

**Bot**: `I'd be happy to help! What specific issue are you facing with your React app?`

**User** (in thread): `Components aren't re-rendering when state changes`

**Bot** (in thread): `This usually happens when state is mutated directly. Make sure you're using setState or useState hooks. Can you share the relevant code?`

## API Reference

### SlackMessenger Methods

```rust
// Create messenger
let messenger = SlackMessenger::new(config, event_tx);

// Start listening
messenger.start().await?;

// Send message
messenger.send_message("C0123456789", "Hello!").await?;

// Stop
messenger.stop().await?;
```

### Configuration Types

```rust
pub struct SlackConfig {
    pub bot_token: String,
    pub app_token: Option<String>,
    pub signing_secret: String,
    pub socket_mode: bool,
    pub webhook_addr: String,
}
```

## Performance

### Memory Usage

- Base: ~5-10MB per Slack connection
- Per message: ~1-5KB
- Recommended: 50MB buffer for Slack integration

### Latency

- Socket Mode: ~50-200ms response time
- HTTP Webhooks: ~100-500ms (includes network roundtrip)

## Related

- [Discord Integration](./MESSENGER_DISCORD.md)
- [Telegram Integration](./MESSENGER_TELEGRAM.md)
- [Matrix Integration](./MESSENGER_MATRIX.md)
- [Gateway Configuration](./HOT_RELOAD.md)

## References

- [Slack API Documentation](https://api.slack.com/)
- [Slack Events API](https://api.slack.com/apis/connections/events-api)
- [Slack Socket Mode](https://api.slack.com/apis/connections/socket)
- [Block Kit](https://api.slack.com/block-kit)

# Telegram Messenger Integration

RustyClaw can be integrated with Telegram, allowing your AI assistant to respond to messages in Telegram chats, groups, and channels.

## Features

- 🤖 **Telegram Bot API** - Full bot functionality with Telegram API
- 💬 **Chats & Groups** - Works in private chats, groups, and supergroups
- 🔄 **Long Polling** - Real-time message updates
- 🌐 **Webhooks** - Efficient webhook-based updates (optional)
- 📎 **Rich Media** - Support for photos, documents, and files
- ⚡ **Fast & Reliable** - Telegram's robust infrastructure
- 🔧 **Simple Setup** - Easy configuration with @BotFather

## Setup

### 1. Create a Telegram Bot

1. Open Telegram and search for **@BotFather**
2. Start a chat and send `/newbot`
3. Follow the prompts:
   - **Bot name**: Enter a display name (e.g., "RustyClaw Assistant")
   - **Bot username**: Enter a unique username ending in `bot` (e.g., "rustyclaw_bot")
4. @BotFather will send you a **bot token** - save this securely!

Example token format: `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`

⚠️ **Important**: Never share your bot token publicly!

### 2. Configure Bot Settings (Optional)

Send commands to @BotFather to customize your bot:

```
/setdescription  - Set bot description
/setabouttext    - Set "about" text
/setuserpic      - Upload bot avatar
/setcommands     - Set bot command list
```

Example commands to set:
```
help - Get help with using the bot
start - Start conversation
ask - Ask a question
```

### 3. Enable Privacy Mode (Recommended)

For group chats, by default bots only see messages starting with `/` or mentioning them.

To allow bot to see all messages (use with caution):
```
/setprivacy → Disable
```

### 4. Get Your Bot Token

If you lost your token, you can regenerate it:
```
@BotFather → /token → Select your bot
```

## Configuration

Add to `~/.rustyclaw/config.toml`:

```toml
[telegram]
# Bot token from @BotFather (required)
bot_token = "123456789:ABCdefGHIjklMNOpqrsTUVwxyz"

# Webhook URL (optional, for webhook mode)
webhook_url = "https://your-domain.com/telegram/webhook"

# Webhook listen address (if using webhooks)
webhook_addr = "127.0.0.1:8443"

# Poll interval in seconds (for long polling mode)
poll_interval_secs = 1
```

### Long Polling Mode (Recommended for Development)

```toml
[telegram]
bot_token = "123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
poll_interval_secs = 1
```

### Webhook Mode (Recommended for Production)

Requires public URL with TLS:

```toml
[telegram]
bot_token = "123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
webhook_url = "https://your-domain.com/telegram/webhook"
webhook_addr = "0.0.0.0:8443"
```

### Secure Token Storage

For production, use secrets vault:

```bash
# Store token securely
rustyclaw secrets set TELEGRAM_BOT_TOKEN "your-token-here"

# Reference in config
[telegram]
bot_token = "${TELEGRAM_BOT_TOKEN}"
```

## Building with Telegram Support

Telegram integration is an optional feature. Build with:

```bash
cargo build --release --features messenger-telegram
```

Or add to default features:

```toml
[features]
default = ["tui", "web-tools", "messenger-telegram"]
```

## Usage

### Start Gateway with Telegram

```bash
rustyclaw gateway start --features messenger-telegram
```

The gateway will:
1. Verify bot token with Telegram API
2. Start long polling or webhook server
3. Listen for incoming messages
4. Process messages through RustyClaw's AI assistant
5. Reply in the same chat

### Start a Conversation

**Private Chat**:
1. Search for your bot username (e.g., @rustyclaw_bot)
2. Click "Start" or send `/start`
3. Send any message!

**Group Chat**:
1. Add bot to group
2. Mention bot: `@rustyclaw_bot what's the weather?`
3. Or use commands: `/ask what's the weather?`

### Message Format

**Direct Message**:
```
What's the capital of France?
```

**Mention in Group**:
```
@rustyclaw_bot explain quantum computing
```

**Command**:
```
/ask write a Python function
```

## Advanced Features

### Reply to Messages

Bot automatically replies to your messages:
- Creates a reply chain
- Maintains conversation context
- Easy to follow in busy groups

### Send Photos

Bot can send photos:

```rust
messenger.send_photo(
    "123456789",  // chat_id
    "https://example.com/image.jpg",
    Some("Here's the image you requested")
).await?;
```

### Markdown Formatting

Messages support Markdown:
```
*bold* _italic_ `code` [link](url)
```

Example response:
```
*RustyClaw Response*

Here's a _formatted_ message with `code`:

```python
def hello():
    print("Hello, World!")
```
```

### Bot Commands

Register commands with @BotFather:
```
/setcommands

help - Get help
ask - Ask a question
reset - Reset conversation
status - Check bot status
```

### Inline Keyboards

Add interactive buttons:

```json
{
  "inline_keyboard": [[
    {"text": "Option 1", "callback_data": "opt1"},
    {"text": "Option 2", "callback_data": "opt2"}
  ]]
}
```

## Message Types

Telegram supports various message types:

### Text Messages
Standard text communication

### Photos
Send/receive images

### Documents
Send/receive files (up to 50MB)

### Voice Messages
Audio messages (future support)

### Location
Share locations (future support)

### Stickers
Telegram stickers (future support)

## Rate Limits

Telegram rate limits:
- **Private chats**: 30 messages per second
- **Groups**: 20 messages per minute
- **Broadcast**: 30 messages per second

RustyClaw automatically handles rate limiting.

## Security

### Bot Token Security

- Store tokens in secrets vault
- Never commit to version control
- Regenerate if compromised
- Use HTTPS webhooks only

### Message Privacy

Bot can only:
- Read messages in chats it's added to
- Cannot read other bots' messages
- Cannot access user's private chats without invitation

### Webhook Security

For webhook mode:
- Use HTTPS only (not HTTP)
- Validate X-Telegram-Bot-Api-Secret-Token header
- Use fixed webhook path (not predictable)

## Troubleshooting

### Bot Not Responding

**1. Check bot token is valid**:
```bash
curl https://api.telegram.org/bot<YOUR_TOKEN>/getMe
```

**2. Check gateway logs**:
```bash
tail -f ~/.rustyclaw/logs/gateway.log
```

**3. Verify bot is running**:
```bash
ps aux | grep rustyclaw
```

### Connection Issues

**Error**: "Unauthorized"
- Token is invalid or revoked
- Regenerate token with @BotFather

**Error**: "Bad Gateway"
- Telegram API temporarily down
- Check https://telegram.org/status

**Error**: "Conflict: terminated by other getUpdates"
- Another instance is polling
- Stop other instances or use webhooks

### Privacy Mode Issues

If bot doesn't see group messages:
1. Go to @BotFather
2. Send `/setprivacy`
3. Select your bot
4. Choose "Disable"

### Webhook Issues

**Webhook not receiving updates**:
- Verify URL is publicly accessible
- Must use HTTPS (not HTTP)
- Check SSL certificate is valid
- Verify port is open (443, 80, 88, or 8443)

**Set webhook manually**:
```bash
curl -X POST https://api.telegram.org/bot<TOKEN>/setWebhook \
  -d url=https://your-domain.com/telegram/webhook
```

**Check webhook status**:
```bash
curl https://api.telegram.org/bot<TOKEN>/getWebhookInfo
```

## Examples

### Simple Q&A

**User**: `What's the capital of Spain?`

**Bot**: `The capital of Spain is Madrid. It's located in the center of the country and is both the largest city and political center of Spain.`

### Code Generation

**User**: `/ask write a Rust function to check if a number is prime`

**Bot**:
````rust
fn is_prime(n: u64) -> bool {
    if n <= 1 {
        return false;
    }
    if n <= 3 {
        return true;
    }
    if n % 2 == 0 || n % 3 == 0 {
        return false;
    }

    let mut i = 5;
    while i * i <= n {
        if n % i == 0 || n % (i + 2) == 0 {
            return false;
        }
        i += 6;
    }
    true
}

// Usage:
println!("{}", is_prime(17)); // true
println!("{}", is_prime(20)); // false
````

### Multi-turn Conversation

**User**: `I need help with my React app`

**Bot** (reply): `I'd be happy to help! What specific issue are you encountering?`

**User** (reply): `State isn't updating when I call setState`

**Bot** (reply): `This often happens if you're mutating state directly or if updates are asynchronous. Can you show me the relevant code?`

## API Reference

### TelegramMessenger Methods

```rust
// Create messenger
let messenger = TelegramMessenger::new(config, event_tx);

// Start polling/webhook
messenger.start().await?;

// Send text message
messenger.send_message("123456789", "Hello!").await?;

// Send photo
messenger.send_photo("123456789", "https://example.com/img.jpg", Some("Caption")).await?;

// Get bot info
let bot_info = messenger.get_me().await?;

// Stop
messenger.stop().await?;
```

### Configuration Types

```rust
pub struct TelegramConfig {
    pub bot_token: String,
    pub webhook_url: Option<String>,
    pub webhook_addr: Option<String>,
    pub poll_interval_secs: u64,
}
```

### Telegram API Types

```rust
pub struct TelegramUser {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}
```

## Performance

### Memory Usage

- Base: ~5-10MB per Telegram connection
- Per chat: ~100KB-1MB (depends on message history)
- Per message: ~500B-5KB

### Latency

- Long Polling: ~100-500ms response time
- Webhooks: ~50-200ms response time
- Message send: ~100-300ms

### Optimization

- Use webhooks for production (lower latency)
- Adjust poll_interval_secs based on traffic
- Implement message queue for high-traffic bots

## Comparison with Other Platforms

| Feature | Telegram | Slack | Discord |
|---------|----------|-------|---------|
| Setup Difficulty | Easy | Medium | Medium |
| API Simplicity | Excellent | Good | Good |
| Rate Limits | Generous | Strict | Medium |
| Rich Media | Excellent | Good | Excellent |
| Group Support | Excellent | Excellent | Excellent |
| Free Tier | Unlimited | Limited | Good |

## Related

- [Slack Integration](./MESSENGER_SLACK.md)
- [Discord Integration](./MESSENGER_DISCORD.md)
- [Matrix Integration](./MESSENGER_MATRIX.md)
- [Gateway Configuration](./HOT_RELOAD.md)

## References

- [Telegram Bot API Documentation](https://core.telegram.org/bots/api)
- [Telegram Bot Features](https://core.telegram.org/bots/features)
- [BotFather Guide](https://core.telegram.org/bots/tutorial)
- [Telegram Bot Samples](https://core.telegram.org/bots/samples)

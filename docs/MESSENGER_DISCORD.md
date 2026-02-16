# Discord Messenger Integration

RustyClaw can be integrated with Discord servers, allowing your AI assistant to respond to messages in Discord channels and direct messages.

## Features

- 🤖 **Discord Bot Integration** - Full bot functionality with Discord Gateway
- 💬 **Server & DM Support** - Works in servers, channels, and DMs
- 🎯 **Mentions & Commands** - Respond to mentions or command prefixes
- 📝 **Message Threading** - Reply to messages to maintain context
- ⚡ **Real-time Gateway** - WebSocket connection for instant responses
- 🎨 **Rich Embeds** - Support for Discord message embeds (planned)
- 🔧 **Slash Commands** - Modern Discord slash commands (planned)

## Setup

### 1. Create a Discord Application

1. Go to https://discord.com/developers/applications
2. Click **"New Application"**
3. Enter a name (e.g., "RustyClaw") and accept terms
4. Click **"Create"**

### 2. Create a Bot

1. Go to the **"Bot"** section in the sidebar
2. Click **"Add Bot"** → **"Yes, do it!"**
3. Configure bot settings:
   - **Username**: Set your bot's display name
   - **Icon**: Upload an avatar (optional)
   - **Public Bot**: Toggle off if you want private bot
4. Under **"Privileged Gateway Intents"**, enable:
   - ✅ **MESSAGE CONTENT INTENT** (required to read messages)
   - ✅ **SERVER MEMBERS INTENT** (for member info)
   - ✅ **PRESENCE INTENT** (optional, for user status)
5. Click **"Reset Token"** and copy the bot token

⚠️ **Important**: Never share your bot token! Treat it like a password.

### 3. Set Bot Permissions

1. Go to **"OAuth2"** → **"URL Generator"**
2. Select scopes:
   - ✅ `bot`
   - ✅ `applications.commands` (for slash commands)
3. Select bot permissions:
   - ✅ `Send Messages`
   - ✅ `Send Messages in Threads`
   - ✅ `Embed Links`
   - ✅ `Attach Files`
   - ✅ `Read Message History`
   - ✅ `Add Reactions`
   - ✅ `Use Slash Commands`
4. Copy the generated URL at the bottom

### 4. Invite Bot to Server

1. Paste the URL from step 3 into your browser
2. Select the server you want to add the bot to
3. Click **"Authorize"**
4. Complete the CAPTCHA if prompted

### 5. Get Application ID

1. Go back to **"General Information"**
2. Copy your **"APPLICATION ID"** (also called Client ID)

## Configuration

Add to `~/.rustyclaw/config.toml`:

```toml
[discord]
# Bot token (required)
bot_token = "your-bot-token-here"

# Application ID (required)
application_id = "your-application-id"

# Command prefix for text commands (optional)
command_prefix = "!"

# Respond to all messages (not just mentions/commands)
respond_to_all = false
```

### Example Configuration

```toml
[discord]
bot_token = "YOUR-DISCORD-BOT-TOKEN-HERE"
application_id = "1234567890123456789"
command_prefix = "!"
respond_to_all = false
```

### Secure Token Storage

For production, use secrets vault:

```bash
# Store token securely
rustyclaw secrets set DISCORD_BOT_TOKEN "your-token-here"

# Reference in config
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
application_id = "1234567890123456789"
```

## Building with Discord Support

Discord integration is an optional feature. Build with:

```bash
cargo build --release --features messenger-discord
```

Or add to your default features:

```toml
[features]
default = ["tui", "web-tools", "messenger-discord"]
```

## Usage

### Start Gateway with Discord

```bash
rustyclaw gateway start --features messenger-discord
```

The bot will:
1. Connect to Discord Gateway via WebSocket
2. Authenticate with bot token
3. Listen for messages mentioning the bot or using command prefix
4. Process messages through RustyClaw's AI assistant
5. Respond in the same channel/thread

### Mention the Bot

In any channel:
```
@RustyClaw what's the weather today?
```

### Use Command Prefix

If configured with `command_prefix = "!"`:
```
!help me write a Python function
```

### Direct Messages

1. Right-click the bot in server member list
2. Click "Message"
3. Start chatting!

### Reply to Messages

The bot can reply to specific messages:
1. Send a message to the bot
2. Bot replies to your message
3. This creates a reply thread maintaining context

## Message Handling

### Trigger Conditions

The bot responds when:
- **Mentioned**: `@BotName your question here`
- **Command Prefix**: `!your question here` (if configured)
- **Direct Message**: Any message in DM channel
- **All Messages**: If `respond_to_all = true` (use with caution!)

### Message Limits

Discord enforces message limits:
- **Content**: 2000 characters max
- **Embed Description**: 4096 characters max
- **Files**: 25MB max (50MB with Nitro)

Long responses are automatically split into multiple messages.

### Rate Limits

Discord rate limits:
- **Global**: 50 requests per second
- **Per Channel**: 5 messages per 5 seconds
- **Per Guild**: 10 messages per 10 seconds

RustyClaw handles rate limiting with exponential backoff.

## Advanced Features

### Reactions

Bot can add reactions to messages:

```rust
messenger.add_reaction(channel_id, message_id, "✅").await?;
```

### Rich Embeds

Create formatted embeds:

```json
{
  "embeds": [{
    "title": "RustyClaw Response",
    "description": "Your answer here",
    "color": 5814783,
    "footer": {
      "text": "Powered by RustyClaw"
    }
  }]
}
```

### Slash Commands

Register slash commands with Discord:

```rust
// Example: /ask command
{
  "name": "ask",
  "description": "Ask RustyClaw a question",
  "options": [{
    "name": "question",
    "description": "Your question",
    "type": 3,
    "required": true
  }]
}
```

### Thread Support

Bot can create and respond in threads:
- Reply to a message to start a thread
- Each thread maintains separate conversation context
- Bot tracks thread history for context-aware responses

## Permissions

### Required Permissions

Minimum permissions for bot to function:
- `Send Messages` (2048)
- `Read Message History` (65536)

### Recommended Permissions

For full functionality:
- `Send Messages` (2048)
- `Embed Links` (16384)
- `Attach Files` (32768)
- `Read Message History` (65536)
- `Add Reactions` (64)
- `Use Slash Commands` (2147483648)

### Permission Integer

Calculate permission integer: https://discordapi.com/permissions.html

For recommended permissions: `2147581952`

## Security

### Token Security

- Never commit tokens to version control
- Use environment variables or secrets vault
- Regenerate token if exposed
- Enable 2FA on Discord account

### Message Verification

Discord Gateway connection uses:
- WebSocket Secure (WSS)
- Token-based authentication
- Heartbeat for connection health

### User Privacy

- Bot only sees messages in channels it has access to
- Cannot read messages in channels it's not invited to
- Cannot access user's private servers without invite

## Troubleshooting

### Bot Not Responding

1. **Check bot is online**:
   - Look for green dot next to bot in member list
   - If offline, check gateway logs

2. **Verify bot has permissions**:
   ```
   Right-click channel → Edit Channel → Permissions → Check bot role
   ```

3. **Check Message Content Intent**:
   - Go to Developer Portal → Bot → Privileged Gateway Intents
   - Ensure "MESSAGE CONTENT INTENT" is enabled

4. **Test with direct mention**:
   ```
   @BotName test
   ```

### Connection Issues

**Error**: "Invalid token"
- Regenerate token in Developer Portal
- Update config with new token

**Error**: "Missing Access"
- Check bot has permission to view/send in channel
- Verify bot role in server settings

**Error**: "Gateway connection failed"
- Check internet connection
- Verify no firewall blocking WSS
- Check Discord status: https://discordstatus.com

### Rate Limiting

If you see `429 Too Many Requests`:
- Reduce message frequency
- Use message queue
- Respect rate limit headers
- Consider using embeds to consolidate info

### Message Not Sending

- Check message length < 2000 characters
- Verify bot has Send Messages permission
- Check if channel is text channel (not voice/category)
- Ensure bot isn't blocked by user

## Examples

### Simple Q&A

**User**: `@RustyClaw what's the capital of Japan?`

**Bot**: `The capital of Japan is Tokyo. It's the most populous metropolitan area in the world.`

### Code Generation

**User**: `!write a Rust function to reverse a string`

**Bot**:
````rust
fn reverse_string(s: &str) -> String {
    s.chars().rev().collect()
}

// Usage:
let original = "hello";
let reversed = reverse_string(original);
println!("{}", reversed); // Prints: "olleh"
````

### Multi-turn Conversation

**User**: `@RustyClaw I need help debugging my code`

**Bot** (reply): `I'd be happy to help! What programming language and what issue are you seeing?`

**User** (in thread): `Python, getting a KeyError`

**Bot** (in thread): `A KeyError means you're trying to access a dictionary key that doesn't exist. Can you share the relevant code?`

## API Reference

### DiscordMessenger Methods

```rust
// Create messenger
let messenger = DiscordMessenger::new(config, event_tx);

// Start Gateway connection
messenger.start().await?;

// Send message
messenger.send_message("1234567890", "Hello!").await?;

// Reply to message
messenger.reply_to_message(channel_id, message_id, "Response").await?;

// Add reaction
messenger.add_reaction(channel_id, message_id, "👍").await?;

// Stop
messenger.stop().await?;
```

### Configuration Types

```rust
pub struct DiscordConfig {
    pub bot_token: String,
    pub application_id: String,
    pub command_prefix: Option<String>,
    pub respond_to_all: bool,
}
```

## Performance

### Memory Usage

- Base: ~10-20MB per Discord connection
- Per server: ~1-5MB (depends on member count)
- Per message: ~1-3KB

### Latency

- Gateway connection: ~50-100ms
- Message send: ~100-300ms
- Command response: ~200-500ms (includes AI processing)

### Optimization

- Use message queue for high-traffic servers
- Implement caching for frequently accessed data
- Use Gateway compression (zlib-stream)

## Related

- [Slack Integration](./MESSENGER_SLACK.md)
- [Telegram Integration](./MESSENGER_TELEGRAM.md)
- [Matrix Integration](./MESSENGER_MATRIX.md)
- [Gateway Configuration](./HOT_RELOAD.md)

## References

- [Discord Developer Portal](https://discord.com/developers/docs)
- [Discord Gateway Documentation](https://discord.com/developers/docs/topics/gateway)
- [Discord API Reference](https://discord.com/developers/docs/reference)
- [Discord.js Guide](https://discordjs.guide/) (JavaScript, but concepts apply)

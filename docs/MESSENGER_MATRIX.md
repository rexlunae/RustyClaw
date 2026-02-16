# Matrix Messenger Integration

RustyClaw can be integrated with Matrix, the open standard for secure, decentralized, real-time communication. This allows your AI assistant to be accessible through Matrix clients like Element, FluffyChat, and others.

## Features

- 🌐 **Federated Protocol** - Decentralized, open standard communication
- 🔐 **End-to-End Encryption** - Optional E2EE with Olm/Megolm
- 💬 **Rooms & Spaces** - Works in rooms, direct messages, and spaces
- 🏠 **Self-Hosted** - Run your own homeserver or use matrix.org
- 📱 **Cross-Platform** - Available on all major platforms
- 🔧 **Rich Features** - Supports formatting, reactions, threads, and more
- 🤝 **Interoperable** - Bridges to other platforms (Slack, Discord, IRC, etc.)

## What is Matrix?

Matrix is an open standard for interoperable, decentralized, real-time communication over IP. Unlike proprietary platforms:
- **Open Source**: Specification and implementations are open
- **Decentralized**: No single point of control
- **Federated**: Different homeservers communicate seamlessly
- **Secure**: End-to-end encryption built-in
- **Bridges**: Connect to other platforms

## Setup

### Option 1: Use Existing Matrix Account

If you already have a Matrix account:
1. Your username (e.g., `@yourbot:matrix.org`)
2. Your password or access token
3. Your homeserver URL (e.g., `https://matrix.org`)

### Option 2: Create New Bot Account

#### Using matrix.org

1. Go to https://app.element.io
2. Click **"Create Account"**
3. Choose a username (e.g., `rustyclaw-bot`)
4. Complete registration
5. Your Matrix ID will be `@rustyclaw-bot:matrix.org`

#### Using Self-Hosted Homeserver

If running your own homeserver (Synapse, Dendrite, Conduit):

```bash
# Example with Synapse
register_new_matrix_user -c homeserver.yaml http://localhost:8008
```

Enter username and password for your bot.

### Get Access Token (Recommended)

Instead of storing passwords, use access tokens:

1. **Using Element**:
   - Settings → Help & About → Advanced → Access Token
   - Copy the token (starts with `syt_...`)

2. **Using curl**:
```bash
curl -X POST https://matrix.org/_matrix/client/v3/login \
  -H "Content-Type: application/json" \
  -d '{
    "type": "m.login.password",
    "identifier": {
      "type": "m.id.user",
      "user": "your-username"
    },
    "password": "your-password"
  }'
```

## Configuration

Add to `~/.rustyclaw/config.toml`:

```toml
[matrix_messenger]
# Matrix homeserver URL (required)
homeserver_url = "https://matrix.org"

# Bot Matrix ID (required)
username = "@rustyclaw:matrix.org"

# Password or access token (required)
password = "syt_yourAccessToken..."

# Device ID (optional, for E2EE)
device_id = "RUSTYCLAW"

# Device display name (optional)
device_name = "RustyClaw Bot"
```

### Example Configuration

**Using Access Token** (Recommended):
```toml
[matrix_messenger]
homeserver_url = "https://matrix.org"
username = "@rustyclaw:matrix.org"
password = "syt_bG9naW4_bG9naW4_bG9naW4_bG9naW4_..."
device_name = "RustyClaw Bot"
```

**Using Password**:
```toml
[matrix_messenger]
homeserver_url = "https://matrix-client.matrix.org"
username = "@rustyclaw:matrix.org"
password = "your-secure-password"
```

**Self-Hosted Homeserver**:
```toml
[matrix_messenger]
homeserver_url = "https://matrix.example.com"
username = "@rustyclaw:example.com"
password = "your-access-token"
```

### Secure Credentials Storage

For production, use secrets vault:

```bash
# Store credentials securely
rustyclaw secrets set MATRIX_USERNAME "@rustyclaw:matrix.org"
rustyclaw secrets set MATRIX_PASSWORD "syt_..."

# Reference in config
[matrix_messenger]
homeserver_url = "https://matrix.org"
username = "${MATRIX_USERNAME}"
password = "${MATRIX_PASSWORD}"
```

## Building with Matrix Support

Matrix integration uses the matrix-sdk. Build with:

```bash
cargo build --release --features matrix
```

Or add to default features:

```toml
[features]
default = ["tui", "web-tools", "matrix"]
```

## Usage

### Start Gateway with Matrix

```bash
rustyclaw gateway start --features matrix
```

The gateway will:
1. Connect to Matrix homeserver
2. Log in with credentials
3. Start syncing to receive events
4. Listen for messages mentioning the bot
5. Respond in the same room

### Invite Bot to Room

**Element Web/Desktop**:
1. Open a room or create new one
2. Click room info → **"Invite"**
3. Enter bot's Matrix ID: `@rustyclaw:matrix.org`
4. Send invite

**Command Line**:
```
/invite @rustyclaw:matrix.org
```

### Direct Message

1. Click **"Start Chat"**
2. Enter bot's Matrix ID
3. Start messaging!

### Message Format

**Direct Message**:
```
What's the weather today?
```

**In Room** (bot must be mentioned):
```
@rustyclaw:matrix.org explain quantum computing
```

**In E2EE Room**:
End-to-end encrypted rooms work if device_id is configured.

## Advanced Features

### Formatted Messages

Matrix supports rich formatting with HTML:

```rust
messenger.send_formatted_message(
    "!room:matrix.org",
    "**Bold** and *italic*",  // Plain text fallback
    "<strong>Bold</strong> and <em>italic</em>"  // HTML
).await?;
```

### Reactions

Add emoji reactions to messages:

```rust
messenger.add_reaction(
    "!room:matrix.org",
    "$event_id",
    "👍"
).await?;
```

### Join/Leave Rooms

```rust
// Join room
messenger.join_room("#community:matrix.org").await?;

// Leave room
messenger.leave_room("!room_id:matrix.org").await?;
```

### Room Aliases vs IDs

- **Room Alias**: `#community:matrix.org` (human-readable)
- **Room ID**: `!abcdefgh:matrix.org` (immutable)

Both work for most operations.

## End-to-End Encryption

Matrix supports E2EE using Olm/Megolm:

### Enable E2EE

```toml
[matrix_messenger]
homeserver_url = "https://matrix.org"
username = "@rustyclaw:matrix.org"
password = "syt_..."
device_id = "RUSTYCLAW"  # Required for E2EE
```

### Verify Device

Users may need to verify your bot's device:
1. Click bot's name in room
2. Go to **"Security"**
3. Verify the device keys

### Limitations

- E2EE adds complexity
- Requires device verification
- Some features may not work in E2EE rooms

## Bridges

Matrix can bridge to other platforms:

### Using Bridges

Connect Matrix to:
- **Slack**: matrix-appservice-slack
- **Discord**: matrix-appservice-discord
- **Telegram**: mautrix-telegram
- **IRC**: matrix-appservice-irc
- **WhatsApp**: mautrix-whatsapp

### Bot in Bridged Rooms

Your bot works in bridged rooms, appearing on both sides.

## Rate Limits

Matrix homeservers have rate limits:
- **matrix.org**: ~10 requests/second per user
- **Self-hosted**: Configurable

RustyClaw handles rate limiting automatically.

## Security

### Credential Security

- Use access tokens (not passwords)
- Store in secrets vault
- Never commit to version control
- Rotate tokens periodically

### Room Privacy

Bot can only:
- See messages in rooms it's invited to
- Cannot access encrypted history before joining
- Cannot read E2EE messages without device verification

### Permissions

Matrix uses power levels (0-100):
- **0**: Normal user
- **50**: Moderator
- **100**: Admin

Bot typically doesn't need elevated permissions.

## Troubleshooting

### Bot Not Connecting

**1. Check homeserver URL**:
```bash
curl https://matrix.org/_matrix/client/versions
```
Should return JSON with supported versions.

**2. Verify credentials**:
```bash
curl -X POST https://matrix.org/_matrix/client/v3/login \
  -H "Content-Type: application/json" \
  -d '{"type":"m.login.password","identifier":{"type":"m.id.user","user":"yourbot"},"password":"..."}'
```

**3. Check gateway logs**:
```bash
tail -f ~/.rustyclaw/logs/gateway.log
```

### Bot Not Responding

**1. Check bot is in room**:
- Look for bot in member list
- Re-invite if needed

**2. Check message format**:
- Bot must be mentioned: `@botname:server what's up?`
- Or use direct message

**3. Verify sync is working**:
- Check logs for "sync" messages
- Restart gateway if sync stalled

### E2EE Issues

**Error**: "Unable to decrypt"
- Device not verified
- Missing device_id in config
- Key backup not configured

**Solution**:
```toml
[matrix_messenger]
device_id = "RUSTYCLAW"  # Add this
```

Then re-login and verify device.

### Homeserver Issues

**Error**: "M_FORBIDDEN"
- Check credentials
- Verify account not deactivated
- Check rate limits

**Error**: "M_UNKNOWN_TOKEN"
- Access token expired
- Re-login to get new token

## Matrix vs. Other Platforms

| Feature | Matrix | Slack | Discord | Telegram |
|---------|--------|-------|---------|----------|
| Open Source | ✅ Yes | ❌ No | ❌ No | ❌ Partial |
| Decentralized | ✅ Yes | ❌ No | ❌ No | ❌ No |
| E2EE | ✅ Yes | ⚠️ Enterprise | ⚠️ Partial | ⚠️ Secret |
| Self-Hosted | ✅ Yes | ❌ No | ❌ No | ❌ No |
| Bridges | ✅ Many | ⚠️ Limited | ⚠️ Limited | ⚠️ Limited |
| Free | ✅ Yes | ⚠️ Limited | ✅ Yes | ✅ Yes |

## Popular Matrix Clients

- **Element**: Official client, web/desktop/mobile
- **FluffyChat**: Mobile-focused, cute UI
- **SchildiChat**: Element fork with extra features
- **Nheko**: Lightweight desktop client
- **Fractal**: GNOME-native client

All clients work with RustyClaw bot.

## Examples

### Simple Q&A

**User**: `@rustyclaw:matrix.org what's the capital of Germany?`

**Bot**: `The capital of Germany is Berlin. It has been the capital since reunification in 1990.`

### Code Generation

**User**: `@rustyclaw:matrix.org write a Python function to sort a list`

**Bot**:
````python
def sort_list(items, reverse=False):
    """Sort a list in ascending or descending order."""
    return sorted(items, reverse=reverse)

# Usage:
numbers = [3, 1, 4, 1, 5, 9, 2, 6]
print(sort_list(numbers))  # [1, 1, 2, 3, 4, 5, 6, 9]
print(sort_list(numbers, reverse=True))  # [9, 6, 5, 4, 3, 2, 1, 1]
````

### Formatted Response

**User**: `@rustyclaw:matrix.org format this text in markdown`

**Bot** (HTML formatted):
**Bold text** with *italic* and `code`

## API Reference

### MatrixMessenger Methods

```rust
// Create messenger
let messenger = MatrixMessenger::new(config, event_tx);

// Start connection
messenger.start().await?;

// Send plain message
messenger.send_message("!room:matrix.org", "Hello!").await?;

// Send formatted message
messenger.send_formatted_message(
    "!room:matrix.org",
    "Plain text",
    "<strong>HTML</strong>"
).await?;

// Join room
messenger.join_room("#community:matrix.org").await?;

// Add reaction
messenger.add_reaction("!room:matrix.org", "$event_id", "👍").await?;

// Stop
messenger.stop().await?;
```

### Configuration Types

```rust
pub struct MatrixConfig {
    pub homeserver_url: String,
    pub username: String,
    pub password: String,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
}
```

## Performance

### Memory Usage

- Base: ~15-30MB per Matrix connection
- With E2EE: +10-20MB
- Per room: ~500KB-2MB
- Per message: ~1-5KB

### Latency

- Message send: ~100-500ms
- Sync interval: ~1-5 seconds
- Response time: ~200-800ms (includes AI processing)

## Related

- [Slack Integration](./MESSENGER_SLACK.md)
- [Discord Integration](./MESSENGER_DISCORD.md)
- [Telegram Integration](./MESSENGER_TELEGRAM.md)
- [Gateway Configuration](./HOT_RELOAD.md)

## References

- [Matrix Specification](https://spec.matrix.org/)
- [Matrix SDK Documentation](https://docs.rs/matrix-sdk/)
- [Element Web](https://app.element.io)
- [Matrix.org](https://matrix.org)
- [Matrix Bridges](https://matrix.org/bridges/)

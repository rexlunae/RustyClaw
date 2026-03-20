# Signal CLI Messenger Configuration

This guide explains how to configure RustyClaw to work with Signal Private Messenger using the signal-cli tool.

## Prerequisites

1. **Install signal-cli**: Download and install signal-cli from the official repository:
   ```bash
   # On macOS with Homebrew
   brew install signal-cli
   
   # On Ubuntu/Debian
   wget https://github.com/AsamK/signal-cli/releases/download/v0.12.2/signal-cli-0.12.2.tar.gz
   tar xf signal-cli-0.12.2.tar.gz
   sudo mv signal-cli-0.12.2 /opt/signal-cli
   sudo ln -sf /opt/signal-cli/bin/signal-cli /usr/local/bin/
   
   # Or build from source
   git clone https://github.com/AsamK/signal-cli.git
   cd signal-cli
   ./gradlew installDist
   ```

2. **Register your phone number**:
   ```bash
   signal-cli -u +1234567890 register
   ```
   
3. **Verify with the received code**:
   ```bash
   signal-cli -u +1234567890 verify CODE_RECEIVED_VIA_SMS
   ```

## Configuration

Add the following to your RustyClaw configuration file (`~/.config/rustyclaw/config.toml`):

```toml
[messengers.signal]
type = "signal-cli"
phone_number = "+1234567890"  # Your registered Signal phone number
signal_cli_path = "/usr/local/bin/signal-cli"  # Optional, defaults to "signal-cli"
enabled = true
```

## Building with Signal Support

Build RustyClaw with Signal CLI support enabled:

```bash
cargo build --features signal-cli
```

Or add it to your default features in `Cargo.toml`:

```toml
[features]
default = ["signal-cli"]
```

## Usage

Once configured, you can send Signal messages through RustyClaw:

```rust
use rustyclaw_core::messengers::{SignalCliMessenger, Messenger};

let mut messenger = SignalCliMessenger::new(
    "my_signal".to_string(),
    "+1234567890".to_string(),
);

messenger.initialize().await?;
let message_id = messenger.send_message("+19876543210", "Hello from RustyClaw!").await?;
```

## Limitations

- **External dependency**: Requires signal-cli to be installed and configured
- **No native replies**: signal-cli doesn't provide easy reply-to functionality
- **Limited media support**: Media attachments are supported but may have limitations
- **Group chat**: Group messaging is supported but requires group IDs from signal-cli
- **Rate limiting**: Signal has rate limits; respect them to avoid blocking

## Troubleshooting

### Common Issues

1. **signal-cli not found**:
   - Ensure signal-cli is installed and in your PATH
   - Or specify the full path in `signal_cli_path`

2. **Account not registered**:
   ```bash
   signal-cli -u +1234567890 register
   signal-cli -u +1234567890 verify CODE
   ```

3. **Permission denied**:
   - Ensure signal-cli has proper permissions
   - Check that the Signal data directory is writable

4. **Network issues**:
   - Ensure internet connectivity
   - Check firewall settings
   - Verify Signal servers are accessible

### Testing

Test your setup manually:
```bash
# Test sending
signal-cli -u +1234567890 send -m "Test message" +19876543210

# Test receiving
signal-cli -u +1234567890 receive --timeout 10 --json
```

## Security Considerations

- **Data storage**: signal-cli stores encryption keys locally
- **Process security**: Messages may briefly exist in process memory
- **Log safety**: Avoid logging sensitive message content
- **Permission model**: signal-cli runs with user permissions

For production use, consider:
- Running signal-cli as a dedicated user
- Setting up proper file permissions on Signal data directory
- Regular backup of Signal keys (stored in `~/.local/share/signal-cli/`)
- Monitoring for signal-cli updates and security patches
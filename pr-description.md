# Add Signal messenger support via signal-cli

Fixes #115

## Summary

This PR implements Signal Private Messenger integration for RustyClaw using the `signal-cli` external tool. This follows the CLI-based messenger approach outlined in the issue, providing a clean integration without requiring complex native Signal protocol libraries.

## Changes Made

### Core Implementation
- **`crates/rustyclaw-core/src/messengers/signal_cli.rs`**: Complete SignalCliMessenger implementation
- **`crates/rustyclaw-core/Cargo.toml`**: Added `signal-cli` feature flag
- **`crates/rustyclaw-core/src/messengers/mod.rs`**: Exported SignalCliMessenger with feature gating

### Documentation
- **`docs/signal-cli-setup.md`**: Comprehensive setup and configuration guide

## Features

✅ **Send messages** to individual phone numbers  
✅ **Receive messages** with JSON parsing  
✅ **Phone number normalization** to E.164 format  
✅ **Media attachment support** via signal-cli  
✅ **Group messaging** support (requires group IDs)  
✅ **Proper error handling** with descriptive messages  
✅ **Connection management** and health checks  
✅ **Integration tests** for core functionality  

## Prerequisites

This implementation requires:

1. **signal-cli**: Must be installed separately from https://github.com/AsamK/signal-cli
2. **Signal registration**: Phone number must be registered with Signal
3. **Configuration**: Proper setup in RustyClaw config.toml

## Configuration Example

```toml
[messengers.signal]
type = "signal-cli"
phone_number = "+1234567890"
signal_cli_path = "/usr/local/bin/signal-cli"  # Optional
enabled = true
```

## Usage Example

```rust
use rustyclaw_core::messengers::{SignalCliMessenger, Messenger};

let mut messenger = SignalCliMessenger::new(
    "my_signal".to_string(),
    "+1234567890".to_string(),
);

messenger.initialize().await?;
let message_id = messenger.send_message("+19876543210", "Hello!").await?;
```

## Testing

Build with Signal support:
```bash
cargo build --features signal-cli
```

Run tests:
```bash
cargo test signal_cli --features signal-cli
```

## Architecture

This implementation follows the existing RustyClaw messenger pattern:
- Implements the `Messenger` trait
- Uses external process execution via `tokio::process::Command`
- Provides proper async/await support
- Includes comprehensive error handling
- Follows the feature-gated compilation model

## Limitations

- **External dependency**: Requires signal-cli installation
- **Process overhead**: Each operation spawns a signal-cli process
- **Limited native features**: Some Signal features may not be available
- **Rate limiting**: Subject to Signal's rate limits

## Backward Compatibility

This change is fully backward compatible:
- New feature is behind a feature flag (`signal-cli`)
- No changes to existing messenger APIs
- No breaking changes to configuration format
- Existing messengers remain unaffected

## Future Improvements

Potential enhancements for follow-up PRs:
- Process pooling for better performance
- Enhanced group chat management
- Better media handling and validation
- Signal sticker support
- Message reaction support

## Related Issues

- Closes #115: Messenger support with CLI-based approach
- Part of the two-tier messenger strategy discussed in #115

---

**Review Notes**: This implementation provides a solid foundation for Signal messaging in RustyClaw while maintaining the project's architectural principles. The CLI-based approach ensures reliability and easier maintenance compared to native protocol implementations.
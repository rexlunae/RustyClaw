# RustyClaw

A super-lightweight, super-capable agentic tool with improved security versus OpenClaw.

## Features

- **Written in Rust**: High-performance, memory-safe implementation
- **OpenClaw Compatible**: Architecture based on OpenClaw with inspiration from NanoBot and PicoClaw
- **Skills Support**: Able to use skills from OpenClaw
- **SOUL.md**: Configurable agent personality and behavior
- **Secure Secrets Storage**: Integrated secrets storage with user-controlled access
- **TUI Interface**: Terminal User Interface as the main interface
- **Messenger Support**: Support for the same messengers as OpenClaw

## Architecture

RustyClaw is designed with security and modularity in mind:

- **Configuration Management**: OpenClaw-compatible settings directory
- **Skills System**: Load and manage skills dynamically
- **SOUL Management**: Define agent personality through SOUL.md
- **Secrets Manager**: Secure keyring-based secrets storage with user approval
- **Messenger Abstraction**: Extensible messenger interface for multiple platforms

## Installation

### Prerequisites

- Rust 1.70 or later
- Cargo (comes with Rust)

### Building from Source

```bash
git clone https://github.com/rexlunae/RustyClaw.git
cd RustyClaw
cargo build --release
```

The binary will be available at `target/release/rustyclaw`.

## Usage

### Running RustyClaw

```bash
cargo run
```

Or if you built the release version:

```bash
./target/release/rustyclaw
```

### TUI Interface

The Terminal User Interface provides the following views:

- **F1**: Main view - Message display and command input
- **F2**: Skills view - View and manage loaded skills
- **F3**: Secrets view - Manage secrets and agent access
- **F4**: Config view - View configuration and SOUL content
- **ESC**: Return to Main view
- **q**: Quit (from Main view)

### Available Commands

In the input field, you can use the following commands:

- `help` - Display available commands
- `clear` - Clear message history
- `enable-access` - Enable agent access to secrets
- `disable-access` - Disable agent access to secrets
- `reload-skills` - Reload skills from disk
- `q` - Quit the application (from Main view)

## Configuration

RustyClaw uses a configuration file located at `~/.rustyclaw/config.toml`.

### Default Configuration

```toml
settings_dir = "/home/user/.rustyclaw"
use_secrets = true

[[messengers]]
name = "example"
enabled = false
```

### Configuration Options

- `settings_dir`: Directory for RustyClaw settings and data
- `soul_path`: Path to SOUL.md file (optional, defaults to `~/.rustyclaw/SOUL.md`)
- `skills_dir`: Directory containing skills (optional, defaults to `~/.rustyclaw/skills`)
- `use_secrets`: Whether to use the secrets storage system
- `messengers`: Array of messenger configurations

## SOUL.md

The SOUL.md file defines the agent's personality and behavior. RustyClaw creates a default SOUL.md on first run if one doesn't exist. You can customize it to define:

- Core Identity
- Principles
- Capabilities
- Limitations

## Skills

Skills are stored as JSON or YAML files in the skills directory (`~/.rustyclaw/skills` by default).

### Skill Format

```json
{
  "name": "example_skill",
  "description": "An example skill",
  "path": "/path/to/skill",
  "enabled": true
}
```

Skills are compatible with OpenClaw's skill format.

## Secrets Management

RustyClaw provides secure secrets storage with user control:

1. **Agent Access Control**: Secrets are only accessible to the agent when explicitly enabled
2. **System Keyring**: Uses the system's secure keyring for storage
3. **User Approval**: Individual secrets can require user approval before access

### Managing Secrets

From the Secrets view (F3), you can:
- Enable/disable agent access to all secrets
- Store new secrets
- Delete existing secrets

## Security Features

- **Secrets Isolation**: Agent cannot access secrets without user permission
- **Keyring Storage**: Secrets stored in system keyring (not plain text)
- **Minimal Permissions**: Follows principle of least privilege
- **User Control**: All security-sensitive operations require user approval

## Development

### Running Tests

```bash
cargo test
```

### Building Documentation

```bash
cargo doc --open
```

## OpenClaw Compatibility

RustyClaw is designed to be compatible with OpenClaw:

- Supports OpenClaw skills format
- Compatible settings directory structure
- SOUL.md support
- Messenger interface compatible with OpenClaw messengers

## License

MIT License - See LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.


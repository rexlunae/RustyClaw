# RustyClaw Quick Start Guide

Get up and running with RustyClaw in just a few minutes!

## Installation

### Prerequisites
- Rust 1.70 or later ([Install Rust](https://rustup.rs/))

### Build and Install

```bash
# Clone the repository
git clone https://github.com/rexlunae/RustyClaw.git
cd RustyClaw

# Build the project
cargo build --release

# The binary is now available at target/release/rustyclaw
```

## First Run

```bash
# Run RustyClaw
cargo run
```

Or if you built the release version:

```bash
./target/release/rustyclaw
```

## Interface Overview

When RustyClaw starts, you'll see the Terminal User Interface (TUI):

```
╔═══════════════════════════════════════════════════════════╗
║ RustyClaw - Lightweight Secure Agent                     ║
╚═══════════════════════════════════════════════════════════╝
╔═══════════════════════════════════════════════════════════╗
║ Messages                                                  ║
║ Welcome to RustyClaw!                                     ║
║ Type 'help' for available commands                       ║
╚═══════════════════════════════════════════════════════════╝
╔═══════════════════════════════════════════════════════════╗
║ Input                                                     ║
╚═══════════════════════════════════════════════════════════╝
```

## Basic Navigation

- **F1**: Main view (default) - Messages and commands
- **F2**: Skills view - View loaded skills
- **F3**: Secrets view - Manage secrets
- **F4**: Config view - View configuration
- **ESC**: Return to Main view
- **q**: Quit (from Main view only)

## First Steps

### 1. Check the Help

Type `help` and press Enter to see available commands:

```
help
```

### 2. View Configuration

Press **F4** to view your configuration:
- Settings directory location
- SOUL.md path
- Configuration status

### 3. Create a Skill

Create a skill file in `~/.rustyclaw/skills`:

```bash
mkdir -p ~/.rustyclaw/skills
cat > ~/.rustyclaw/skills/example.json << EOF
{
  "name": "example_skill",
  "description": "An example skill",
  "path": "/path/to/skill",
  "enabled": true
}
EOF
```

### 4. Reload Skills

In RustyClaw, type:
```
reload-skills
```

Then press **F2** to view your loaded skills.

### 5. Manage Secrets

Press **F3** to view the Secrets Management screen.

Enable agent access to secrets:
```
enable-access
```

Disable agent access:
```
disable-access
```

### 6. Customize Your SOUL

Edit the SOUL.md file to customize the agent's personality:

```bash
nano ~/.rustyclaw/SOUL.md
```

Press **F4** to view the SOUL content preview.

## Configuration

### Location

Default configuration location: `~/.rustyclaw/config.toml`

### Example Configuration

```toml
settings_dir = "/home/user/.rustyclaw"
use_secrets = true

[[messengers]]
name = "example"
enabled = false
```

### Custom Configuration Path

You can specify a custom configuration by modifying the code or setting environment variables (future feature).

## Common Tasks

### Adding Skills

1. Create a JSON or YAML file in `~/.rustyclaw/skills/`
2. Type `reload-skills` in RustyClaw
3. Press **F2** to verify the skill is loaded

### Managing Secrets

1. Press **F3** to access Secrets Management
2. Use commands to enable/disable agent access
3. Secrets are stored securely in your system keyring

### Viewing Logs

Currently, logs are displayed in the Messages view (Main view - **F1**).

## Tips

1. **Start Simple**: Begin with the Main view and explore other views as needed
2. **Use Keyboard Shortcuts**: Function keys (F1-F4) make navigation quick
3. **Check Help Often**: Type `help` to see available commands
4. **Customize SOUL**: Edit SOUL.md to define your agent's behavior
5. **Secure by Default**: Agent access to secrets is disabled by default for security

## Troubleshooting

### Build Issues

If you encounter build issues:
```bash
# Update Rust
rustup update

# Clean build
cargo clean
cargo build --release
```

### Configuration Issues

If configuration isn't loading:
```bash
# Check if directory exists
ls -la ~/.rustyclaw/

# Create default configuration
mkdir -p ~/.rustyclaw
cp config.example.toml ~/.rustyclaw/config.toml
```

### Skills Not Loading

1. Check file format (JSON or YAML)
2. Verify file is in `~/.rustyclaw/skills/`
3. Use `reload-skills` command
4. Check for syntax errors in skill files

## Next Steps

- Read the [README.md](README.md) for detailed documentation
- Check [ARCHITECTURE.md](ARCHITECTURE.md) to understand the design
- See [CONTRIBUTING.md](CONTRIBUTING.md) if you want to contribute
- Review [SECURITY.md](SECURITY.md) for security considerations

## Getting Help

- Open an issue on GitHub for bugs or feature requests
- Check existing documentation
- Review the source code (it's well-commented!)

## Example Session

```
# Start RustyClaw
cargo run

# In RustyClaw:
help                    # See available commands
reload-skills          # Load skills
enable-access          # Enable agent access to secrets
clear                  # Clear message history

# Navigate views:
F2                     # View skills
F3                     # Manage secrets
F4                     # View configuration
F1                     # Return to main view

# Quit
q                      # (from Main view)
```

Happy coding with RustyClaw! 🦞

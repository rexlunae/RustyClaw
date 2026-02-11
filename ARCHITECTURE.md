# RustyClaw Architecture

## Overview

RustyClaw is designed as a modular, secure, and lightweight agentic tool with clear separation of concerns.

## Core Components

### 1. Configuration Management (`config.rs`)
- OpenClaw-compatible configuration system
- TOML-based configuration files
- Default settings with override capability
- Location: `~/.rustyclaw/config.toml`

### 2. Skills System (`skills.rs`)
- Dynamic skill loading from directory
- JSON and YAML skill definitions
- Enable/disable individual skills
- OpenClaw skill format compatibility

### 3. SOUL Management (`soul.rs`)
- Agent personality and behavior definition
- Markdown-based configuration
- Automatic default SOUL generation
- Customizable identity and principles

### 4. Secrets Manager (`secrets.rs`)
- System keyring integration
- User-controlled access model
- Agent access toggle
- Individual secret approval
- Secure credential storage

### 5. Messenger System (`messenger.rs`)
- Abstract messenger trait
- Support for multiple messenger types
- Async/await support
- OpenClaw messenger compatibility

### 6. Terminal UI (`tui.rs`)
- Ratatui-based interface
- Multiple view modes
- Real-time command processing
- Keyboard navigation

## Data Flow

```
User Input → TUI → Command Handler → Core Components → Response → TUI → User
```

## Security Architecture

### Secrets Isolation
1. Secrets stored in system keyring (encrypted at OS level)
2. Agent access disabled by default
3. User must explicitly enable access
4. Individual secrets can require per-access approval

### Principle of Least Privilege
- Minimal default permissions
- Skills can be disabled individually
- Messengers must be explicitly enabled
- Configuration changes require user action

## Extension Points

### Adding a New Messenger

1. Implement the `Messenger` trait:
```rust
#[async_trait]
impl Messenger for MyMessenger {
    fn name(&self) -> &str;
    async fn initialize(&mut self) -> Result<()>;
    async fn send_message(&self, recipient: &str, content: &str) -> Result<()>;
    async fn receive_messages(&self) -> Result<Vec<Message>>;
    fn is_connected(&self) -> bool;
    async fn disconnect(&mut self) -> Result<()>;
}
```

2. Register in MessengerManager
3. Add configuration in `config.toml`

### Adding a New Skill

Create a skill definition file in the skills directory:

```json
{
  "name": "my_skill",
  "description": "Description of the skill",
  "path": "/path/to/skill/implementation",
  "enabled": true
}
```

### Customizing SOUL

Edit `~/.rustyclaw/SOUL.md` to define:
- Core Identity
- Principles
- Capabilities
- Limitations

## OpenClaw Compatibility

RustyClaw maintains compatibility with OpenClaw through:

1. **Configuration Structure**: Same directory layout
2. **Skills Format**: Compatible skill definitions
3. **SOUL.md**: Same personality definition format
4. **Messenger Interface**: Compatible messenger protocol

## Performance Considerations

- **Lazy Loading**: Skills loaded on-demand
- **Caching**: Secrets cached when agent access enabled
- **Async I/O**: Non-blocking messenger operations
- **Efficient TUI**: Minimal redraws, efficient rendering

## Future Enhancements

Potential areas for expansion:
- Plugin system for dynamic skill loading
- Remote configuration synchronization
- Multi-user support
- Encrypted configuration files
- Advanced skill scheduling
- Messenger message queuing
- WebSocket support for real-time updates

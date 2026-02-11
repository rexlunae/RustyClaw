# Contributing to RustyClaw

Thank you for your interest in contributing to RustyClaw! This document provides guidelines and information for contributors.

## Development Setup

### Prerequisites

- Rust 1.70 or later
- Git
- A text editor or IDE (VS Code, RustRover, etc.)

### Setup Steps

1. Fork the repository
2. Clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/RustyClaw.git
   cd RustyClaw
   ```
3. Build the project:
   ```bash
   cargo build
   ```
4. Run tests:
   ```bash
   cargo test
   ```

## Development Workflow

### Running in Development Mode

```bash
cargo run
```

### Building for Release

```bash
cargo build --release
```

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name
```

### Code Formatting

```bash
# Check formatting
cargo fmt -- --check

# Apply formatting
cargo fmt
```

### Linting

```bash
cargo clippy
```

## Project Structure

```
RustyClaw/
├── src/
│   ├── main.rs          # Entry point
│   ├── config.rs        # Configuration management
│   ├── secrets.rs       # Secrets storage
│   ├── skills.rs        # Skills system
│   ├── soul.rs          # SOUL management
│   ├── messenger.rs     # Messenger abstraction
│   └── tui.rs           # Terminal UI
├── Cargo.toml           # Dependencies and metadata
├── README.md            # User documentation
├── ARCHITECTURE.md      # Architecture overview
└── CONTRIBUTING.md      # This file
```

## Adding Features

### Adding a New Command

1. Add command parsing in `tui.rs` `handle_input()` method
2. Implement the command logic
3. Add help text
4. Add tests

Example:
```rust
match parts[0] {
    "mycommand" => {
        // Implementation
        self.messages.push("Command executed".to_string());
    }
    // ... other commands
}
```

### Adding a New View

1. Add new view variant to `View` enum
2. Implement `render_myview` method
3. Add keyboard shortcut in `run_app()`
4. Update help text

### Implementing a New Messenger

1. Create a new struct implementing the `Messenger` trait:
```rust
pub struct MyMessenger {
    name: String,
    connected: bool,
    // ... other fields
}

#[async_trait]
impl Messenger for MyMessenger {
    // Implement required methods
}
```

2. Add configuration support in `config.rs`
3. Add tests
4. Update documentation

## Testing Guidelines

### Unit Tests

Place tests in the same file as the code:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_feature() {
        // Test implementation
    }
}
```

### Integration Tests

Create files in `tests/` directory for integration tests.

### Test Coverage

Aim for:
- All public functions tested
- Edge cases covered
- Error conditions tested

## Code Style

### Rust Guidelines

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `rustfmt` for formatting
- Use `clippy` for linting
- Write idiomatic Rust code

### Documentation

- Add doc comments to public items:
```rust
/// Description of the function
///
/// # Arguments
///
/// * `arg` - Description of argument
///
/// # Returns
///
/// Description of return value
pub fn my_function(arg: Type) -> ReturnType {
    // Implementation
}
```

### Error Handling

- Use `anyhow::Result` for error propagation
- Use `thiserror` for custom error types
- Provide context with `.context()`

## Commit Guidelines

### Commit Message Format

```
<type>: <description>

[optional body]

[optional footer]
```

Types:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes
- `refactor`: Code refactoring
- `test`: Adding tests
- `chore`: Maintenance tasks

Example:
```
feat: Add skill filtering by category

Implemented filtering functionality to allow users to view skills
by category in the TUI interface.
```

## Pull Request Process

1. Create a feature branch:
   ```bash
   git checkout -b feature/my-feature
   ```

2. Make your changes
   - Write code
   - Add tests
   - Update documentation

3. Commit your changes:
   ```bash
   git add .
   git commit -m "feat: Add my feature"
   ```

4. Push to your fork:
   ```bash
   git push origin feature/my-feature
   ```

5. Create a Pull Request
   - Provide clear description
   - Reference related issues
   - Ensure CI passes

## Security Considerations

When contributing, please:

- Never commit secrets or credentials
- Follow security best practices
- Report security issues privately
- Consider privacy implications
- Test security-related changes thoroughly

### Reporting Security Issues

Please report security vulnerabilities to the maintainers privately before public disclosure.

## Questions and Support

- Open an issue for bugs or feature requests
- Use discussions for questions
- Join our community chat (if available)

## Code Review Process

All contributions go through code review:

1. Maintainers review the code
2. Feedback is provided
3. Changes are requested if needed
4. Once approved, code is merged

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

## Recognition

Contributors will be recognized in:
- README.md contributors section
- Release notes
- Project documentation

Thank you for contributing to RustyClaw!

# Contributing to RustyClaw

Thanks for your interest in contributing to RustyClaw! 🦀🦞

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/RustyClaw.git`
3. Create a branch: `git checkout -b my-feature`
4. Make your changes
5. Run tests: `cargo test`
6. Commit: `git commit -m "feat: add cool feature"`
7. Push: `git push origin my-feature`
8. Open a Pull Request

## Development Setup

### Prerequisites

- Rust 1.85+ (edition 2024)
- Cargo

### Build

```bash
cargo build
```

### Test

```bash
# All tests
cargo test

# Specific test file
cargo test --test tool_execution

# With output
cargo test -- --nocapture
```

### Lint

```bash
cargo clippy
cargo fmt --check
```

## Code Style

- Follow Rust conventions (rustfmt enforced)
- Use `///` doc comments for public items
- Add tests for new functionality
- Keep functions focused and small

## Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` New features
- `fix:` Bug fixes
- `docs:` Documentation only
- `test:` Adding/updating tests
- `refactor:` Code changes that don't add features or fix bugs
- `chore:` Maintenance tasks

## Adding Tools

1. Add tool definition to `all_tools()` in `src/tools.rs`
2. Create `*_params()` function for parameters
3. Create `exec_*()` function for execution
4. Add to `resolve_params()` match
5. Add integration tests in `tests/tool_execution.rs`
6. Update documentation

## Adding Tests

- Unit tests go in the module (`#[cfg(test)] mod tests`)
- Integration tests go in `tests/`
- Golden files in `tests/golden/` (update with `UPDATE_GOLDEN=1 cargo test`)

## Security

- Never log secrets
- Use the sandbox for command execution
- Report security issues privately (see [SECURITY.md](./SECURITY.md))

## Questions?

- Open a [Discussion](https://github.com/rexlunae/RustyClaw/discussions)
- Join the [Discord](https://discord.com/invite/clawd)

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

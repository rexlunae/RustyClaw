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

### Pre-PR Checklist

Before opening a PR, ensure your changes compile without warnings:

```bash
# Check library builds warning-free
cargo check --all-features 2>&1 | grep -E "warning:" && echo "Fix warnings before submitting" || echo "✓ No warnings"

# Check tests compile warning-free
cargo test --no-run 2>&1 | grep -E "warning:" && echo "Fix test warnings before submitting" || echo "✓ No test warnings"
```

**No PR should be considered finished while warnings are still present.** Warnings slow down compilation, make CI noisier, and often indicate real issues.

## Code Style

See **[`STYLE_GUIDE.md`](STYLE_GUIDE.md)** for the full project style guide, including
naming conventions, error handling, documentation requirements, and the Clippy baseline.

Key rules at a glance:
- `cargo fmt` is authoritative — no manual deviations.
- `cargo clippy --workspace --all-targets -- -D warnings` must pass.
- Every public item needs a `///` doc comment.
- Use `tracing::*` macros in library code — no bare `println!`.
- No `unwrap()` in library code without a descriptive `expect("…")` message.

## Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` New features
- `fix:` Bug fixes
- `docs:` Documentation only
- `test:` Adding/updating tests
- `refactor:` Code changes that don't add features or fix bugs
- `chore:` Maintenance tasks

## Adding Tools

1. Add tool definition to `all_tools()` in `crates/rustyclaw-core/src/tools/mod.rs`
2. Create `*_params()` function for parameters
3. Create `exec_*()` function for execution
4. Add to `resolve_params()` match
5. Add tests next to the tool, in `crates/rustyclaw-core/src/tools/tests_a.rs`
6. Update documentation

Step 4 is the one that bites: parameters are resolved from a match separate
from registration, and that match ends in a catch-all, so a tool can be
registered and still reach the model with an empty schema — registered,
described, and impossible to invoke with any argument. Four tools were in
that state at once.

`crates/rustyclaw-core/tests/tool_registry.rs` fails when a tool is offered
with no parameters and is not on its `PARAMETERLESS` list. If your tool
genuinely takes none, add it there; otherwise the test is telling you step 4
is missing.

## Adding Tests

This is a workspace with no root package, so Cargo only picks up `tests/`
inside a crate. A `tests/` directory at the repository root is never compiled
and never run — the repo carried eleven such files for months.

- Unit tests go in the module (`#[cfg(test)] mod tests`)
- Integration tests go in the owning crate: `crates/<crate>/tests/`
- CLI end-to-end tests go in `crates/rustyclaw-cli/tests/`, and reach the
  binary through `env!("CARGO_BIN_EXE_rustyclaw")` — Cargo builds it as a
  dependency of the test, so there is nothing to locate and no stale build to
  run against by accident
- Golden files in `crates/rustyclaw-cli/tests/golden/` (update with
  `UPDATE_GOLDEN=1 cargo test -p rustyclaw --test golden_files`)

Write tests that can fail. A test that builds a `json!` literal and asserts
the fields it just wrote are strings passes forever and covers nothing; it
reads as coverage to anyone deciding how carefully to review a change.

## Security

- Never log secrets
- Use the sandbox for command execution
- Report security issues privately (see [SECURITY.md](docs/SECURITY.md))

## Questions?

- Open a [Discussion](https://github.com/rexlunae/RustyClaw/discussions)
- Join the [Discord](https://discord.com/invite/clawd)

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

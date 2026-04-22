# RustyClaw Rust Style Guide

## 1. Purpose & Scope

This guide defines the coding conventions and quality standards for the RustyClaw workspace
(`rustyclaw-core`, `rustyclaw-cli`, `rustyclaw-tui`, `rustyclaw-desktop`). Rules here align
with and expand upon the project's existing [`CONTRIBUTING.md`](CONTRIBUTING.md),
[`PHILOSOPHY.md`](PHILOSOPHY.md), [`ARCHITECTURE.md`](ARCHITECTURE.md), and
[`SECURITY.md`](docs/SECURITY.md). This guide is informed by the community-curated
[`mre/idiomatic-rust`](https://github.com/mre/idiomatic-rust) index, the official
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/), the
[Rust Book](https://doc.rust-lang.org/book/), and
[Rust by Example](https://doc.rust-lang.org/rust-by-example/). Where those sources conflict,
project-specific rules take precedence.

---

## 2. Formatting & Tooling

### `cargo fmt` is authoritative

`cargo fmt` (rustfmt) defines canonical formatting. Never apply manual deviations.
A minimal `rustfmt.toml` may be added at the workspace root — keep it to stable,
uncontroversial options only.

```toml
# rustfmt.toml (minimal example)
edition = "2024"
max_width = 100
```

### Clippy must pass

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

This command must exit `0` before a PR is merged. The workspace `Cargo.toml` codifies the
baseline via `[workspace.lints.clippy]`. Intentional `#[allow(clippy::…)]` annotations require
a `// reason: …` comment on the same line.

### Zero-warning builds required

```bash
cargo check --all-features
```

No PR should introduce warnings. (Reiteration of the `CONTRIBUTING.md` mandate.)

### MSRV

Minimum Supported Rust Version (MSRV) is **`1.86`** (edition **`2024`**). Bumping the MSRV
requires a `CHANGELOG.md` entry. Do not use nightly-only features in production code.

### Useful one-liners

```bash
# Format everything
cargo fmt --all

# Check without formatting
cargo fmt --all --check

# Run all checks (what CI runs)
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
```

---

## 3. Naming (per [RFC 430](https://github.com/rust-lang/rfcs/blob/master/text/0430-finalizing-naming-conventions.md) and the [API Guidelines](https://rust-lang.github.io/api-guidelines/naming.html))

| Item | Convention | Example |
|------|-----------|---------|
| Crates, modules, functions, variables | `snake_case` | `config_dir`, `parse_url` |
| Types, traits, enum variants | `UpperCamelCase` | `ConfigError`, `Messenger` |
| Consts, statics | `SCREAMING_SNAKE_CASE` | `MAX_RETRIES`, `DEFAULT_PORT` |
| Type parameters | Single uppercase letter or `UpperCamelCase` | `T`, `Item` |
| Lifetimes | Short lowercase | `'a`, `'src` |

### Conversion methods

| Method prefix | Meaning | Self consumed? | Cost |
|--------------|---------|---------------|------|
| `as_` | Cheap borrow view | No | Free |
| `to_` | Expensive copy / conversion | No | Allocates |
| `into_` | Owning conversion | Yes | Moves |

✅ `fn as_str(&self) -> &str`  
✅ `fn to_string(&self) -> String`  
✅ `fn into_bytes(self) -> Vec<u8>`  

### Getters: no `get_` prefix

Rust idiom is `foo()` not `get_foo()`.

```rust
// ✅
pub fn name(&self) -> &str { &self.name }

// ❌
pub fn get_name(&self) -> &str { &self.name }
```

### Iterator methods

Expose as `iter()` → `&T`, `iter_mut()` → `&mut T`, `into_iter()` → `T`.

### Constructors

- `new(…) -> Self` for infallible construction.
- `try_new(…) -> Result<Self, E>` or `new(…) -> Result<Self, E>` for fallible.
- Builder methods return `Self` (or `&mut Self` for `&mut`-builder style).

---

## 4. Modules, Crates, and Project Layout

### Small, cohesive modules

Keep modules focused. Public API lives in `lib.rs` or a `pub mod` facade module;
implementation details are `pub(crate)` or private.

### 2018+ module style

Do **not** use `mod.rs`. Use file-per-module (`foo.rs`) and folder-per-submodule (`foo/`).

```
src/
  gateway.rs      # ✅ module file
  gateway/        # ✅ submodules
    transport.rs
```

```
src/
  gateway/
    mod.rs        # ❌ avoid
```

### Workspace metadata

All version pins and shared metadata belong in `[workspace.dependencies]` and
`[workspace.package]`. Crates consume them via `.workspace = true`.

```toml
# ✅ workspace-level
[workspace.dependencies]
tokio = { version = "1.35", features = ["full"] }

# ✅ crate-level
[dependencies]
tokio.workspace = true
```

### Re-exports

Stable public surface is re-exported at crate root; deep paths are implementation details.

```rust
// lib.rs
pub use messengers::{Messenger, Message};
```

---

## 5. Error Handling

### Libraries vs. binaries

| Crate | Rule |
|-------|------|
| `rustyclaw-core`, `rustyclaw-tui` | Use `thiserror` for typed errors. No `anyhow` in public API. |
| `rustyclaw-cli`, `rustyclaw-desktop` | `anyhow::Result` is fine at the top level. |

### No naked `unwrap()` in library code

```rust
// ❌ library code
let val = map.get("key").unwrap();

// ✅ use ? with a typed error
let val = map.get("key").ok_or(CoreError::MissingKey("key"))?;

// ✅ when invariant is genuinely unreachable, document it
let val = map.get("key")
    .expect("key is always present: inserted by constructor");
```

### `panic!` is for programmer errors only

Never `panic!` on user input, IO errors, or network failures. Those go through `Result`.

### Prefer `?` over manual match

```rust
// ✅
let result = something()?;

// ❌
let result = match something() {
    Ok(v) => v,
    Err(e) => return Err(e),
};
```

Use `map_err` to add context when bridging error types. Use `anyhow::Context` in binaries.

### Avoid `Box<dyn Error>` in new code

Prefer concrete error enums (via `thiserror`).

---

## 6. Option, Result, and Control Flow

### Combinators for short chains; `match`/`let else` for clarity

```rust
// ✅ short chain — combinators
let upper = name.map(|n| n.to_uppercase());

// ✅ complex — match
match config.provider() {
    Some(p) => p.connect().await?,
    None => return Err(CoreError::NoProvider),
}
```

### Flatten nesting with `if let` / `let … else`

```rust
// ✅
let Some(user) = session.user() else { return; };

// ❌
if let Some(user) = session.user() {
    // deeply nested
} else {
    return;
}
```

### Use `matches!` for boolean pattern checks

```rust
// ✅
if matches!(status, Status::Active | Status::Pending) { … }

// ❌
if status == Status::Active || status == Status::Pending { … }
```

---

## 7. Ownership, Borrowing, and Lifetimes

### Accept the most general parameter

```rust
// ✅
fn greet(name: &str) { … }
fn process(items: &[u8]) { … }
fn open(path: impl AsRef<Path>) { … }
fn collect(iter: impl IntoIterator<Item = String>) { … }

// ❌
fn greet(name: &String) { … }
fn process(items: &Vec<u8>) { … }
```

### Return owned types from constructors; borrows from accessors

```rust
fn new(name: &str) -> Self { Self { name: name.to_owned() } }
fn name(&self) -> &str { &self.name }
```

### Elide lifetimes

Only annotate lifetimes the compiler cannot infer. Avoid `'static` bounds unless genuinely
required.

### Don't clone to silence the borrow checker

Restructure ownership or use `Rc`/`Arc` with intention. Gratuitous `.clone()` calls hide
design issues.

---

## 8. Traits and Generics

### Standard derives — use them

Every public type should derive (where applicable):

| Trait | Rule |
|-------|------|
| `Debug` | All public types |
| `Clone` | If cloning is cheap and meaningful |
| `Copy` | If `Clone` is trivial and the value is small |
| `Default` | If a sensible zero-value exists |
| `PartialEq` / `Eq` | For types that support equality |
| `Hash` | Together with `Eq` |
| `Display` | For user-facing string output only |

Use `#[derive(…)]` first; hand-roll only with a documented reason.

### `impl Trait` in argument position

```rust
// ✅ ergonomic for callers
fn render(output: impl Write) { … }

// ✅ explicit when caller needs to name the type
fn store<W: Write + Send + 'static>(writer: W) { … }
```

### No blanket impls on foreign types

Implement foreign traits only for your own types.

---

## 9. Async & Concurrency

### Runtime

`tokio` with `#[tokio::main]` in binaries. Libraries must stay runtime-agnostic where
feasible, or document the runtime requirement clearly.

### Do not hold locks across `.await`

```rust
// ✅ lock released before await
let value = {
    let guard = mutex.lock().await;
    guard.clone()
};
do_async_work(value).await;

// ❌ lock held across await
let guard = mutex.lock().await;
do_async_work(&*guard).await; // deadlock risk
```

Use `parking_lot::Mutex` (or `std::sync::Mutex`) for short critical sections that don't
cross await points. Use `tokio::sync::Mutex` only when you must hold across an `.await`.

### Prefer channels over shared state

`Arc<Mutex<T>>` is a code smell. Consider `tokio::sync::{mpsc, watch, broadcast}` first.

### Cancellation safety

Design futures to be cancel-safe, or document clearly that they are not:
```rust
/// # Cancellation
/// This future is **not** cancel-safe: dropping it mid-flight may leave
/// the underlying connection in an inconsistent state.
pub async fn send_transaction(…) { … }
```

### JoinHandles must be managed

Either `.await` the handle or explicitly detach it with a comment explaining why.

---

## 10. Unsafe

Unsafe code is **forbidden by default**. Each `unsafe` block requires a `// SAFETY:` comment
explaining which invariants are upheld and by whom.

```rust
// ✅
// SAFETY: `ptr` is non-null and properly aligned, guaranteed by
//         the allocator contract in `Allocator::alloc`.
unsafe { ptr.write(value) };

// ❌
unsafe { ptr.write(value) }; // no comment
```

Add `#![deny(unsafe_op_in_unsafe_fn)]` at crate root where practical (it is enforced at the
workspace level via `[workspace.lints.rust]`).

---

## 11. Logging, Tracing, and Observability

### `tracing` is the logging framework

Use `tracing::{error!, warn!, info!, debug!, trace!}`. No `println!` or `eprintln!` in library
code (`rustyclaw-core`, `rustyclaw-tui`). User-facing CLI output in `rustyclaw-cli` is the
exception.

```rust
// ✅ library
tracing::info!(session_id = %id, "Session started");

// ❌ library
println!("Session started: {}", id);
```

### Structured fields over interpolated strings

```rust
// ✅
tracing::warn!(peer = %addr, error = %e, "Connection failed");

// ❌
tracing::warn!("Connection to {} failed: {}", addr, e);
```

### Never log secrets

**Never log** API keys, tokens, passwords, auth headers, or full command lines that may
contain them. Redact at the logging site:

```rust
tracing::debug!(key = "***", "API key configured");
```

### Log levels

| Level | When to use |
|-------|-------------|
| `error` | Actionable failure requiring operator attention |
| `warn` | Recoverable anomaly |
| `info` | Lifecycle events (startup, shutdown, connections) |
| `debug` | Developer-oriented diagnostics |
| `trace` | Firehose — call paths, loop iterations |

---

## 12. Testing

### Unit tests: co-located

```rust
// at the bottom of the file
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_url() { … }
}
```

### Integration tests in `tests/`

```
tests/
  tool_execution.rs
  golden/           # golden files; update with UPDATE_GOLDEN=1
```

### Test hygiene

- Tests must be deterministic. No network, no wall-clock, no real filesystem (use `tempfile`
  for filesystem-touching tests unless explicitly tagged `#[ignore]` as integration tests).
- Use `#[should_panic(expected = "substring")]` — never bare `#[should_panic]`.
- Use `assert!(cond)` / `assert!(!cond)` for boolean checks; never
  `assert_eq!(result, true)`.

### Property tests encouraged

For parsers, serializers, and data-transformation functions, use `proptest` or `quickcheck`:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn round_trips(input in ".*") {
        let encoded = encode(&input);
        prop_assert_eq!(decode(&encoded), input);
    }
}
```

---

## 13. Documentation

### Every public item needs a doc comment

```rust
/// Returns the configured gateway URL, defaulting to `ws://localhost:7878`.
///
/// # Examples
///
/// ```rust
/// let config = Config::default();
/// assert!(config.gateway_url().starts_with("ws://"));
/// ```
pub fn gateway_url(&self) -> &str { … }
```

### Doc comment structure

1. One-sentence summary (used in generated index pages).
2. Extended description (optional).
3. `# Examples` — doctests that are `cargo test`-able.
4. `# Errors` — when the function returns `Result<_, E>`.
5. `# Panics` — when the function can `panic!`.
6. `# Safety` — for `unsafe fn`.

### Crate-level docs

Every crate's `lib.rs` (or `main.rs`) must have a `//!` block:

```rust
//! `rustyclaw-core` — shared configuration, gateway protocol, secrets management,
//! tool dispatch, skills, providers, and types used by all RustyClaw clients.
```

### Unknown behavior

If you can't determine what a public item does from its implementation, add a
`// TODO(docs):` marker rather than inventing prose.

---

## 14. Dependencies

### Minimal, well-maintained crates

Aligned with [`PHILOSOPHY.md`](PHILOSOPHY.md): prefer minimal, well-maintained crates.
Avoid proprietary-only SDKs or crates without a clear maintenance path.

### All versions pinned in `[workspace.dependencies]`

```toml
# ✅ Cargo.toml (workspace root)
[workspace.dependencies]
tokio = { version = "1.35", features = ["full"] }

# ✅ crate Cargo.toml
[dependencies]
tokio.workspace = true
```

### No `*` versions; no git deps in release code

The one current exception: `steel-memory` is tracked via a `git` dependency while awaiting
a compatible crates.io release (tracked issue: see repo). Document any such exception with a
comment.

### Feature flags: minimal by default

Optional heavyweight dependencies are gated behind features (e.g., `browser`, `mcp`,
`steel-memory`). The `default` feature set must compile on a stock system without extra
system libraries.

---

## 15. Security (see also [`SECURITY.md`](docs/SECURITY.md))

### Secrets go in the vault, not env vars

Use `securestore` / the encrypted vault for API keys, tokens, and credentials. Never write
them to log files. Never store them in config files unencrypted.

### Debug impls must redact secrets

Any type that holds a secret must produce `"***"` in its `Debug` output:

```rust
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKey").field("value", &"***").finish()
    }
}
```

### Subprocess execution through the sandbox

All shell/subprocess execution goes through the sandbox layer (`crates/rustyclaw-core/src/sandbox.rs`).
Never call `std::process::Command` directly in tool execution paths.

### Input validation at boundaries

Validate and sanitize config values, network data, and tool arguments at the entry point,
not deep inside logic.

---

## 16. Performance

### Measure before optimizing

Use `cargo bench` with `criterion` for hot paths. Profile before micro-optimizing.

### Iterators over manual indexing

```rust
// ✅
let sum: u32 = values.iter().map(|v| v.cost()).sum();

// ❌
let mut sum = 0u32;
for i in 0..values.len() {
    sum += values[i].cost();
}
```

`collect::<Vec<_>>()` is idiomatic when the intermediate `Vec` clarifies the code.

### Avoid allocations in hot paths

Use `&str` over `String`, `Cow<'_, str>` when mixed ownership is needed. Avoid
`format!("…")` when a string literal suffices.

```rust
// ❌ allocates unnecessarily
let msg = format!("error");

// ✅
let msg = "error";
```

### Release profile

The release profile (`Cargo.toml`) already sets `lto = true`, `codegen-units = 1`,
`strip = true`. Do not remove or weaken these settings.

---

## 17. Clippy Lints — Baseline

The root `Cargo.toml` encodes the baseline in `[workspace.lints.clippy]`:

```toml
[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
# lints intentionally silenced — see rationale below
too_many_arguments = "allow"   # reason: complex async handlers have many params; restructuring deferred
type_complexity    = "allow"   # reason: complex generic types appear in gateway/messenger code
module_inception   = "allow"   # reason: crate uses foo/foo.rs module patterns by convention
```

All crates inherit this via `[lints] workspace = true` in each `Cargo.toml`.

When adding a new `#[allow(clippy::…)]` at the call site:

```rust
#[allow(clippy::too_many_lines)] // reason: this function is a large match dispatch table
pub fn dispatch(…) { … }
```

Prefer adding recurring allows to `[workspace.lints.clippy]` rather than scattering
`#[allow]` annotations throughout the codebase.

---

## 18. Commits, PRs, and Review

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full workflow. Summary:

- **Conventional Commits** — `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`
- **Warning-free builds** before opening a PR (enforced by CI).
- **Focused PRs** — avoid drive-by refactors in feature PRs. If you find an unrelated issue,
  open a separate PR or a tracking issue.
- **Pre-PR checklist** (`CONTRIBUTING.md`):
  ```bash
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo check --workspace --all-targets
  cargo test --workspace
  ```

---

## 19. Further Reading

| Resource | URL |
|----------|-----|
| `mre/idiomatic-rust` (community index) | <https://github.com/mre/idiomatic-rust> |
| Rust API Guidelines | <https://rust-lang.github.io/api-guidelines/> |
| The Rust Book | <https://doc.rust-lang.org/book/> |
| Rust by Example | <https://doc.rust-lang.org/rust-by-example/> |
| Rust Design Patterns | <https://rust-unofficial.github.io/patterns/> |
| Clippy lint index | <https://rust-lang.github.io/rust-clippy/master/> |
| RFC 430 — Naming conventions | <https://github.com/rust-lang/rfcs/blob/master/text/0430-finalizing-naming-conventions.md> |

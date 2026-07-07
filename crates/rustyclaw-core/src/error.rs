//! Error-handling strategy for RustyClaw.
//!
//! RustyClaw uses three error-handling patterns, chosen by layer:
//!
//! 1. **Internal logic** (typed `thiserror` enums): Library modules define
//!    small per-module error enums deriving [`thiserror::Error`] (e.g.
//!    `CronError` in [`crate::cron`], `VaultError` in [`crate::memory_vault`],
//!    `SsrfError` in `crate::security::ssrf`). Use `#[from]` for `std::io` and
//!    `serde_json` sources so callers can match on variants and inspect the
//!    error chain.
//!
//! 2. **Tool/model boundary** ([`crate::tools::ToolResult`]): AI-callable
//!    tool functions return [`crate::tools::ToolError`], whose `Display`
//!    output is the message sent back to the model. Per-module typed errors
//!    propagate into it via `#[from]`/`?`; bespoke messages route through
//!    `ToolError::Msg` (via `From<String>`, so
//!    `.map_err(|e| format!("context: {e}"))?` still reads naturally).
//!    The dispatch layer that packages tool output for the model is the
//!    single place a `ToolError` is flattened to a string — nowhere earlier.
//!
//! 3. **Binaries and application glue** (`anyhow::Result`): Top-level
//!    binaries use `anyhow` for its rich context and easy propagation.
//!    Library code in this crate should prefer typed errors instead.
//!
//! This module currently holds no code; it documents the strategy and is the
//! natural home for any future shared error utilities.

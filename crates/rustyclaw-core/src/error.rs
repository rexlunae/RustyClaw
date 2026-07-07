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
//! 2. **Tool/model boundary** (`Result<String, String>`): AI-callable tool
//!    functions return simple string errors because the message is sent back
//!    to the model, which can then try to recover or report the issue to the
//!    user. Flattening a typed error with `.map_err(|e| e.to_string())` at
//!    this boundary is correct — but only at the boundary.
//!
//! 3. **Binaries and application glue** (`anyhow::Result`): Top-level
//!    binaries use `anyhow` for its rich context and easy propagation.
//!    Library code in this crate should prefer typed errors instead.
//!
//! This module currently holds no code; it documents the strategy and is the
//! natural home for any future shared error utilities.

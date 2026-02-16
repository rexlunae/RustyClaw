//! Security module for RustyClaw
//!
//! Provides security validation layers including:
//! - SSRF (Server-Side Request Forgery) protection
//! - Prompt injection defense
//! - Other security utilities

pub mod prompt_guard;
pub mod ssrf;

pub use prompt_guard::{GuardAction, GuardResult, PromptGuard};
pub use ssrf::SsrfValidator;

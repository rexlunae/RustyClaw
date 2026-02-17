//! Security module for RustyClaw
//!
//! Provides security validation layers including:
//! - **SafetyLayer** - Unified security defense (recommended)
//! - SSRF (Server-Side Request Forgery) protection
//! - Prompt injection defense
//! - Credential leak detection

pub mod prompt_guard;
pub mod safety_layer;
pub mod ssrf;

pub use prompt_guard::{GuardAction, GuardResult, PromptGuard};
pub use safety_layer::{
    DefenseCategory, DefenseResult, LeakDetector, LeakResult, PolicyAction, SafetyConfig,
    SafetyLayer,
};
pub use ssrf::SsrfValidator;

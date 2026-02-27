//! Security module for RustyClaw
//!
//! Provides security validation layers including:
//! - **SafetyLayer** - Unified security defense (recommended)
//! - SSRF (Server-Side Request Forgery) protection
//! - Prompt injection defense
//! - Credential leak detection
//! - Input validation
//!
//! # Components
//!
//! - `SafetyLayer` - High-level API combining all defenses
//! - `PromptGuard` - Detects prompt injection attacks with scoring
//! - `LeakDetector` - Prevents credential exfiltration (Aho-Corasick accelerated)
//! - `InputValidator` - Validates input length, encoding, patterns
//! - `SsrfValidator` - Prevents Server-Side Request Forgery
//!
//! # Attribution
//!
//! HTTP request scanning and Aho-Corasick optimization in `LeakDetector`
//! inspired by [IronClaw](https://github.com/nearai/ironclaw) (Apache-2.0).
//! Input validation patterns also adapted from IronClaw.

pub mod leak_detector;
pub mod prompt_guard;
pub mod safety_layer;
pub mod ssrf;
pub mod validator;

pub use leak_detector::{
    LeakAction, LeakDetectionError, LeakDetector, LeakMatch, LeakPattern, LeakScanResult,
    LeakSeverity,
};
pub use prompt_guard::{GuardAction, GuardResult, PromptGuard};
pub use safety_layer::{
    DefenseCategory, DefenseResult, PolicyAction, SafetyConfig, SafetyLayer,
};
pub use ssrf::SsrfValidator;
pub use validator::{InputValidator, ValidationError, ValidationErrorCode, ValidationResult};

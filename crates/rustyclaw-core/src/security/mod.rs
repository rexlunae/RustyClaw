//! Security module for RustyClaw
//!
//! Provides Server-Side Request Forgery (SSRF) protection for the
//! web_fetch tool and any other HTTP-based functionality.

pub mod ssrf;

pub use ssrf::{SsrfError, SsrfValidator};

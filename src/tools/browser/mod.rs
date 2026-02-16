//! Browser automation module
//!
//! Provides browser control via Chrome DevTools Protocol (CDP) with support for:
//! - Multiple isolated profiles
//! - Tab management
//! - Element interaction
//! - JavaScript evaluation
//! - Screenshots and accessibility snapshots

pub mod profiles;
mod automation;

pub use automation::exec_browser;
pub use profiles::{ProfileInfo, ProfileManager};

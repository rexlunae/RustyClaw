//! Simple markdown to styled text conversion for TUI rendering.
//!
//! Thin wrapper around [`rustyclaw_core::markdown`] that adds iocraft
//! rendering helpers.

#[allow(unused_imports)]
use iocraft::prelude::*;

// Re-export the core markdown types and functions
pub use rustyclaw_core::markdown::{parse_markdown, render_ansi, StyledSegment};

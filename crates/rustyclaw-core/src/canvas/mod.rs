//! Canvas — agent-controlled visual workspace.
//!
//! Canvas provides a way for agents to display HTML, CSS, JS content
//! and A2UI (Agent-to-UI) components in a visual panel.
//!
//! # Features
//!
//! - Serve HTML/CSS/JS content from a session-specific directory
//! - A2UI protocol support for declarative UI updates
//! - Snapshot capture for visual context
//! - JavaScript evaluation
//!
//! # Usage
//!
//! Canvas is typically accessed via node commands:
//! - `canvas.present` — show the canvas panel
//! - `canvas.navigate` — navigate to a path or URL
//! - `canvas.eval` — evaluate JavaScript
//! - `canvas.snapshot` — capture current state as image
//! - `canvas.a2ui_push` — push A2UI updates

mod config;
mod host;
mod a2ui;

pub use config::CanvasConfig;
pub use host::CanvasHost;
pub use a2ui::{A2UIMessage, A2UISurface, A2UIComponent};

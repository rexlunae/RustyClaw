//! Canvas configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Canvas configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasConfig {
    /// Whether canvas is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Port for the canvas host server
    #[serde(default = "default_port")]
    pub port: u16,

    /// Root directory for canvas files
    #[serde(default)]
    pub root: Option<PathBuf>,

    /// Whether to enable A2UI support
    #[serde(default = "default_a2ui")]
    pub a2ui: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_port() -> u16 {
    18789 // Same as OpenClaw
}

fn default_a2ui() -> bool {
    true
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 18789,
            root: None,
            a2ui: true,
        }
    }
}

impl CanvasConfig {
    /// Get the canvas root directory, defaulting to workspace/canvas
    pub fn canvas_root(&self, workspace: &std::path::Path) -> PathBuf {
        self.root
            .clone()
            .unwrap_or_else(|| workspace.join("canvas"))
    }

    /// Get the session canvas directory
    pub fn session_dir(&self, workspace: &std::path::Path, session: &str) -> PathBuf {
        self.canvas_root(workspace).join(session)
    }
}

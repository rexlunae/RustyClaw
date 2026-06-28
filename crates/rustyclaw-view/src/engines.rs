//! View data for the local engines/model management panel.

use serde::{Deserialize, Serialize};

/// Full panel data for the engine manager UI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnginesPanelData {
    /// All known engines with their current status.
    pub engines: Vec<LocalEngineData>,
    /// Currently selected engine id (for model list view).
    pub selected_engine: Option<String>,
    /// Models for the selected engine.
    pub models: Vec<LocalModelData>,
    /// Active pull progress (if any).
    pub pull_progress: Option<PullProgressData>,
    /// Host resource summary.
    pub host_ram_bytes: u64,
    pub host_vram_bytes: u64,
    pub host_gpu_name: Option<String>,
}

impl EnginesPanelData {
    /// Get engine data by id.
    pub fn engine(&self, id: &str) -> Option<&LocalEngineData> {
        self.engines.iter().find(|e| e.id == id)
    }

    /// Whether any engine is currently running.
    pub fn any_running(&self) -> bool {
        self.engines.iter().any(|e| e.running)
    }
}

/// View data for a single local engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalEngineData {
    pub id: String,
    pub display_name: String,
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    pub endpoint: Option<String>,
    pub available_models: u32,
    pub loaded_models: u32,
    pub caps: EngineCapsData,
}

impl LocalEngineData {
    /// Status badge string for display.
    pub fn status_badge(&self) -> &'static str {
        if !self.installed {
            "not installed"
        } else if self.running {
            "running"
        } else {
            "stopped"
        }
    }

    /// CSS class name for the status badge.
    pub fn status_class(&self) -> &'static str {
        if !self.installed {
            "is-light"
        } else if self.running {
            "is-success"
        } else {
            "is-warning"
        }
    }

    /// Whether a given action is supported.
    pub fn can(&self, action: &str) -> bool {
        match action {
            "install" => self.caps.can_install,
            "start" => self.caps.can_start,
            "stop" => self.caps.can_stop,
            "pull" => self.caps.can_pull,
            "remove" => self.caps.can_remove,
            "load" => self.caps.can_load,
            "unload" => self.caps.can_unload,
            _ => false,
        }
    }
}

/// Capability flags for UI enable/disable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineCapsData {
    pub can_install: bool,
    pub can_start: bool,
    pub can_stop: bool,
    pub can_pull: bool,
    pub can_remove: bool,
    pub can_load: bool,
    pub can_unload: bool,
}

/// View data for a single local model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalModelData {
    pub engine: String,
    pub name: String,
    pub size_bytes: u64,
    pub quantization: Option<String>,
    pub context_length: Option<u32>,
    pub loaded: bool,
    pub vram_bytes: Option<u64>,
    pub family: Option<String>,
    pub format: Option<String>,
    /// Whether this model fits the host's available resources.
    pub fits_host: bool,
    /// Specific warning message from host-fit analysis.
    #[serde(default)]
    pub fit_warning_msg: String,
}

impl LocalModelData {
    /// Human-readable size.
    pub fn size_display(&self) -> String {
        if self.size_bytes >= 1_000_000_000 {
            format!("{:.1} GB", self.size_bytes as f64 / 1e9)
        } else if self.size_bytes > 0 {
            format!("{:.0} MB", self.size_bytes as f64 / 1e6)
        } else {
            "unknown".into()
        }
    }

    /// Load status badge.
    pub fn load_badge(&self) -> &'static str {
        if self.loaded { "loaded" } else { "on disk" }
    }

    /// Warning message if model doesn't fit (returns the detailed message
    /// from the host-fit analysis, or None if it fits).
    pub fn fit_warning(&self) -> Option<&str> {
        if !self.fits_host {
            if self.fit_warning_msg.is_empty() {
                Some("may not fit host VRAM/RAM")
            } else {
                Some(&self.fit_warning_msg)
            }
        } else {
            None
        }
    }
}

/// Streaming pull progress.
#[derive(Debug, Clone, PartialEq)]
pub struct PullProgressData {
    pub engine: String,
    pub model: String,
    pub percent: f32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub status: String,
}

impl PullProgressData {
    /// Progress bar percentage (0..100).
    pub fn pct(&self) -> u8 {
        (self.percent.clamp(0.0, 100.0)) as u8
    }

    /// Human-readable progress string.
    pub fn display(&self) -> String {
        let dl = format_bytes(self.downloaded_bytes);
        let total = format_bytes(self.total_bytes);
        format!("{}: {:.0}% ({}/{})", self.model, self.percent, dl, total)
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1e6)
    } else if bytes > 0 {
        format!("{:.0} KB", bytes as f64 / 1e3)
    } else {
        "0".into()
    }
}

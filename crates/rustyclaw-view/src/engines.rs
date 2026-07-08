//! View data for the local engines/model management panel.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Full panel data for the engine manager UI.
///
/// The dialog renders one tab per engine; [`selected_engine`](Self::selected_engine)
/// is the active tab (and also drives which engine's models are shown).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnginesPanelData {
    /// All known engines with their current status.
    pub engines: Vec<LocalEngineData>,
    /// Currently selected engine id — the active tab, and the engine whose
    /// model list is loaded into [`models`](Self::models).
    pub selected_engine: Option<String>,
    /// Models for the selected engine.
    pub models: Vec<LocalModelData>,
    /// Active pull progress (if any).
    pub pull_progress: Option<PullProgressData>,
    /// Live install output per engine id. Kept separate from
    /// [`engines`](Self::engines) so it survives the frequent engine-list
    /// refreshes that rebuild the engine entries.
    pub install_output: HashMap<String, InstallOutputData>,
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

    /// Index of the active-tab engine within [`engines`](Self::engines),
    /// derived from [`selected_engine`](Self::selected_engine). Falls back
    /// to the first engine when nothing is selected.
    pub fn active_index(&self) -> usize {
        self.selected_engine
            .as_deref()
            .and_then(|id| self.engines.iter().position(|e| e.id == id))
            .unwrap_or(0)
    }

    /// The active-tab engine, if any engines exist.
    pub fn active_engine(&self) -> Option<&LocalEngineData> {
        self.engines.get(self.active_index())
    }

    /// Install output for the active engine, if any has streamed.
    pub fn active_install_output(&self) -> Option<&InstallOutputData> {
        let id = &self.active_engine()?.id;
        self.install_output.get(id)
    }

    /// Append a line of install output for an engine, bounding the tail so
    /// the log can't grow without limit.
    ///
    /// If the engine's previous install had already finished, this line
    /// begins a new install run, so the stale log is cleared first rather
    /// than appended to.
    pub fn push_install_line(&mut self, engine: &str, line: impl Into<String>) {
        let entry = self.install_output.entry(engine.to_string()).or_default();
        if entry.done {
            entry.lines.clear();
            entry.ok = false;
        }
        entry.done = false;
        entry.lines.push(line.into());
        let overflow = entry
            .lines
            .len()
            .saturating_sub(InstallOutputData::MAX_LINES);
        if overflow > 0 {
            entry.lines.drain(..overflow);
        }
    }

    /// Mark an engine's install as finished (success or failure).
    ///
    /// The terminal `message` from a streaming install is the full joined
    /// output that was already recorded line by line, so it is only used as
    /// a fallback when nothing streamed — e.g. an "already installed" early
    /// return that never shelled out. Appending it otherwise would show the
    /// whole log a second time as one blob.
    pub fn finish_install(&mut self, engine: &str, ok: bool, message: impl Into<String>) {
        let entry = self.install_output.entry(engine.to_string()).or_default();
        entry.done = true;
        entry.ok = ok;
        let message = message.into();
        if entry.lines.is_empty() && !message.trim().is_empty() {
            entry.lines.push(message);
        }
        let overflow = entry
            .lines
            .len()
            .saturating_sub(InstallOutputData::MAX_LINES);
        if overflow > 0 {
            entry.lines.drain(..overflow);
        }
    }
}

/// Live output of an engine install, accumulated line by line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstallOutputData {
    /// Output lines streamed from the installer (bounded tail).
    pub lines: Vec<String>,
    /// Whether the install has finished.
    pub done: bool,
    /// Whether it finished successfully (only meaningful once `done`).
    pub ok: bool,
}

impl InstallOutputData {
    /// Maximum output lines retained; older lines are dropped.
    pub const MAX_LINES: usize = 200;

    /// A one-line status header for the install panel.
    pub fn status_line(&self) -> &'static str {
        if !self.done {
            "installing…"
        } else if self.ok {
            "install complete"
        } else {
            "install failed"
        }
    }

    /// The last `n` output lines, for a bounded display.
    pub fn tail(&self, n: usize) -> &[String] {
        let start = self.lines.len().saturating_sub(n);
        &self.lines[start..]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(id: &str) -> LocalEngineData {
        LocalEngineData {
            id: id.into(),
            display_name: id.into(),
            installed: false,
            running: false,
            version: None,
            endpoint: None,
            available_models: 0,
            loaded_models: 0,
            caps: EngineCapsData::default(),
        }
    }

    #[test]
    fn active_tab_tracks_selected_engine() {
        let mut panel = EnginesPanelData {
            engines: vec![engine("ollama"), engine("llamacpp"), engine("exo")],
            ..Default::default()
        };
        // No selection → first engine is the active tab.
        assert_eq!(panel.active_index(), 0);
        assert_eq!(panel.active_engine().unwrap().id, "ollama");

        panel.selected_engine = Some("exo".into());
        assert_eq!(panel.active_index(), 2);
        assert_eq!(panel.active_engine().unwrap().id, "exo");

        // A stale selection falls back to the first engine.
        panel.selected_engine = Some("gone".into());
        assert_eq!(panel.active_index(), 0);
    }

    #[test]
    fn install_output_accumulates_and_finishes() {
        let mut panel = EnginesPanelData {
            engines: vec![engine("ollama")],
            selected_engine: Some("ollama".into()),
            ..Default::default()
        };
        panel.push_install_line("ollama", "downloading…");
        panel.push_install_line("ollama", "installing binary");

        let out = panel.active_install_output().expect("has output");
        assert_eq!(out.lines, vec!["downloading…", "installing binary"]);
        assert!(!out.done);
        assert_eq!(out.status_line(), "installing…");

        // The terminal message is the full joined output already streamed,
        // so finishing must NOT re-append it (which would duplicate the log).
        panel.finish_install("ollama", true, "downloading…\ninstalling binary");
        let out = panel.active_install_output().unwrap();
        assert!(out.done && out.ok);
        assert_eq!(out.status_line(), "install complete");
        assert_eq!(out.lines, vec!["downloading…", "installing binary"]);
    }

    #[test]
    fn new_install_run_clears_the_previous_log() {
        let mut panel = EnginesPanelData {
            engines: vec![engine("ollama")],
            selected_engine: Some("ollama".into()),
            ..Default::default()
        };
        panel.push_install_line("ollama", "old line 1");
        panel.finish_install("ollama", false, "old failed");
        assert!(panel.active_install_output().unwrap().done);

        // A fresh install run starts — the stale log is cleared, not appended.
        panel.push_install_line("ollama", "new line 1");
        let out = panel.active_install_output().unwrap();
        assert_eq!(out.lines, vec!["new line 1"]);
        assert!(!out.done && !out.ok);
    }

    #[test]
    fn finished_install_is_not_reopened_by_a_second_finish() {
        // Guards in the client handlers skip finish_install once done, but
        // the flag itself must also survive so the status stays correct.
        let mut panel = EnginesPanelData {
            engines: vec![engine("ollama")],
            selected_engine: Some("ollama".into()),
            ..Default::default()
        };
        panel.push_install_line("ollama", "installing");
        panel.finish_install("ollama", true, "");
        let out = panel.install_output.get("ollama").unwrap();
        assert!(out.done && out.ok);
        // The handler guard is `!o.done`, so a later start/stop result never
        // reaches finish_install for this engine.
        assert!(out.done);
    }

    #[test]
    fn finish_install_uses_message_only_when_nothing_streamed() {
        let mut panel = EnginesPanelData {
            engines: vec![engine("ollama")],
            selected_engine: Some("ollama".into()),
            ..Default::default()
        };
        // No streamed lines (e.g. an "already installed" early return).
        panel.finish_install("ollama", true, "Ollama is already installed.");
        let out = panel.active_install_output().unwrap();
        assert_eq!(out.lines, vec!["Ollama is already installed."]);
        assert!(out.done && out.ok);
    }

    #[test]
    fn install_output_tail_is_bounded() {
        let mut panel = EnginesPanelData::default();
        for i in 0..(InstallOutputData::MAX_LINES + 50) {
            panel.push_install_line("ollama", format!("line {i}"));
        }
        let out = panel.install_output.get("ollama").unwrap();
        assert_eq!(out.lines.len(), InstallOutputData::MAX_LINES);
        // Oldest lines were dropped; the newest is retained.
        assert_eq!(
            out.lines.last().unwrap(),
            &format!("line {}", InstallOutputData::MAX_LINES + 49)
        );
        assert_eq!(out.tail(3).len(), 3);
    }

    #[test]
    fn install_output_survives_engine_list_refresh_pattern() {
        // Mirrors the client handlers: a fresh engine list is merged into
        // the existing panel, which must not wipe streamed install output.
        let mut panel = EnginesPanelData::default();
        panel.push_install_line("ollama", "step 1");
        panel.engines = vec![engine("ollama"), engine("exo")];
        assert_eq!(panel.install_output.get("ollama").unwrap().lines.len(), 1);
    }
}

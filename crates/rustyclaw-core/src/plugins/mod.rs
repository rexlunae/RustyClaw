//! Plugin system — dynamically-loaded UI panels the agent and user can
//! interact with, rendered beside the chat window.
//!
//! ## Structure
//!
//! Each plugin lives in `<workspace>/plugins/<name>/` with:
//!   - `plugin.json` — metadata, state schema, actions, and instructions
//!   - `index.html`  — (optional) HTML template for custom rendering
//!
//! ## Communication
//!
//! ```text
//! Agent ←→ plugin_* tools          (read/write state, invoke actions)
//! Agent ←→ Gateway ←→ plugin events (state push to desktop/TUI clients)
//! User  → Desktop UI → Gateway      (user interactions are forwarded)
//! ```
//!
//! ## Plugin JSON Format
//!
//! ```json
//! {
//!   "name": "chart",
//!   "description": "Render live charts from data",
//!   "version": "1.0.0",
//!   "emoji": "📊",
//!   "instructions": "## Agent Instructions\n\nUse plugin_state_set to push data.",
//!   "schema": {
//!     "type": "object",
//!     "properties": {
//!       "title": { "type": "string" },
//!       "chartType": { "type": "string", "enum": ["bar", "line", "pie"] },
//!       "data": { "type": "array" }
//!     }
//!   },
//!   "initial_state": { "title": "My Chart", "chartType": "bar", "data": [] },
//!   "actions": [
//!     { "name": "refresh", "description": "Re-fetch data" },
//!     { "name": "export", "description": "Export as CSV" }
//!   ]
//! }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod error;
pub use error::PluginError;

// ── Plugin types ──────────────────────────────────────────────────────────

/// A loaded plugin from disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    /// Unique kebab-case identifier.
    pub name: String,
    /// Human-readable one-liner.
    pub description: String,
    /// Semver version.
    pub version: String,
    /// Display emoji for the UI.
    pub emoji: Option<String>,
    /// Markdown instructions for the agent.
    pub instructions: String,
    /// Path to the plugin directory.
    pub path: PathBuf,
    /// JSON Schema for the plugin state.
    #[serde(default)]
    pub state_schema: Value,
    /// Initial state (default values).
    #[serde(default)]
    pub initial_state: Value,
    /// Named actions the agent can invoke.
    #[serde(default)]
    pub actions: Vec<PluginAction>,
    /// Whether this plugin is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Custom HTML template path (relative to plugin dir, or None for default).
    #[serde(default)]
    pub html_template: Option<String>,
}

fn default_true() -> bool {
    true
}

/// A named action the agent can invoke on a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginAction {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub params: Vec<PluginActionParam>,
}

/// A parameter for a plugin action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginActionParam {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

// ── Plugin manager ────────────────────────────────────────────────────────

/// Manages the lifecycle of all plugins.
pub struct PluginManager {
    plugins_dir: PathBuf,
    plugins: Vec<Plugin>,
    /// Runtime state for each plugin (keyed by plugin name).
    states: HashMap<String, Value>,
}

impl PluginManager {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self {
            plugins_dir,
            plugins: Vec::new(),
            states: HashMap::new(),
        }
    }

    /// Load all plugins from the plugins directory.
    pub fn load(&mut self) -> Result<(), PluginError> {
        self.plugins.clear();

        let dir = &self.plugins_dir;
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest = path.join("plugin.json");
            if !manifest.exists() {
                continue;
            }
            match Self::load_plugin(&path, &manifest) {
                Ok(plugin) => {
                    // Restore persisted state if present
                    let state = self.load_state(&plugin);
                    self.states.insert(plugin.name.clone(), state);
                    self.plugins.push(plugin);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to load plugin");
                }
            }
        }

        // Sort by name for determinism
        self.plugins.sort_by(|a, b| a.name.cmp(&b.name));
        tracing::info!(count = self.plugins.len(), "Loaded plugins");
        Ok(())
    }

    fn load_plugin(dir: &Path, manifest: &Path) -> Result<Plugin, PluginError> {
        let raw = std::fs::read_to_string(manifest)
            .map_err(|e| PluginError::Io(e, manifest.to_path_buf()))?;
        let mut plugin: Plugin = serde_json::from_str(&raw)
            .map_err(|e| PluginError::Parse(e, manifest.to_path_buf()))?;
        plugin.path = dir.to_path_buf();

        // Validate
        if plugin.name.is_empty() {
            return Err(PluginError::Invalid("plugin missing 'name'".into()));
        }
        if plugin.name.contains('/') || plugin.name.contains(' ') {
            return Err(PluginError::Invalid(
                "plugin name must be a simple identifier (no slashes or spaces)".into(),
            ));
        }

        Ok(plugin)
    }

    /// Load persisted state from disk.
    fn load_state(&self, plugin: &Plugin) -> Value {
        let state_file = plugin.path.join("state.json");
        match std::fs::read_to_string(&state_file) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| plugin.initial_state.clone()),
            Err(_) => plugin.initial_state.clone(),
        }
    }

    /// Persist a plugin's state to disk.
    pub fn save_state(&self, plugin_name: &str) -> Result<(), PluginError> {
        if let Some(plugin) = self.get(plugin_name) {
            let state = self.states.get(plugin_name).cloned().unwrap_or_default();
            let state_file = plugin.path.join("state.json");
            let raw = serde_json::to_string_pretty(&state)
                .map_err(|e| PluginError::Serialize(e, plugin_name.to_string()))?;
            std::fs::write(&state_file, raw).map_err(|e| PluginError::Io(e, state_file))?;
        }
        Ok(())
    }

    // ── Query ────────────────────────────────────────────────────────────

    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }

    pub fn get(&self, name: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.name == name)
    }

    // ── State CRUD ───────────────────────────────────────────────────────

    pub fn get_state(&self, name: &str) -> Result<Value, PluginError> {
        self.states
            .get(name)
            .cloned()
            .ok_or_else(|| PluginError::NotFound(name.to_string()))
    }

    pub fn set_state(&mut self, name: &str, state: Value) -> Result<(), PluginError> {
        if self.get(name).is_none() {
            return Err(PluginError::NotFound(name.to_string()));
        }
        self.states.insert(name.to_string(), state);
        self.save_state(name)?;
        Ok(())
    }

    /// Merge a partial update into the plugin state (JSON Merge Patch).
    pub fn patch_state(&mut self, name: &str, patch: &Value) -> Result<Value, PluginError> {
        if self.get(name).is_none() {
            return Err(PluginError::NotFound(name.to_string()));
        }
        let mut current = self.states.get(name).cloned().unwrap_or_default();
        json_merge(&mut current, patch);
        self.states.insert(name.to_string(), current.clone());
        self.save_state(name)?;
        Ok(current)
    }

    /// Snapshot all plugin states for sending to a UI client.
    pub fn state_snapshot(&self) -> HashMap<String, Value> {
        self.states.clone()
    }

    // ── Plugin creation ──────────────────────────────────────────────────

    /// Create a new plugin on disk from name, description, and instructions.
    pub fn create(
        &mut self,
        name: &str,
        description: &str,
        emoji: Option<&str>,
        instructions: &str,
        schema: Option<&Value>,
        initial_state: Option<&Value>,
        actions: Option<Vec<PluginAction>>,
        html_template: Option<&str>,
    ) -> Result<&Plugin, PluginError> {
        // Validate
        if name.is_empty() {
            return Err(PluginError::Invalid("name cannot be empty".into()));
        }
        if name.contains('/') || name.contains(' ') {
            return Err(PluginError::Invalid(
                "name must be a simple identifier (no slashes or spaces)".into(),
            ));
        }

        let plugin_dir = self.plugins_dir.join(name);
        if plugin_dir.exists() {
            return Err(PluginError::AlreadyExists(name.to_string()));
        }

        std::fs::create_dir_all(&plugin_dir).map_err(|e| PluginError::Io(e, plugin_dir.clone()))?;

        let plugin = Plugin {
            name: name.to_string(),
            description: description.to_string(),
            version: "0.1.0".into(),
            emoji: emoji.map(|s| s.to_string()),
            instructions: instructions.to_string(),
            path: plugin_dir.clone(),
            state_schema: schema.cloned().unwrap_or_default(),
            initial_state: initial_state.cloned().unwrap_or_default(),
            actions: actions.unwrap_or_default(),
            enabled: true,
            html_template: html_template.map(|s| s.to_string()),
        };

        let manifest = serde_json::to_string_pretty(&plugin)
            .map_err(|e| PluginError::Serialize(e, name.to_string()))?;
        std::fs::write(plugin_dir.join("plugin.json"), &manifest)
            .map_err(|e| PluginError::Io(e, plugin_dir.join("plugin.json")))?;

        // Write initial state
        let state = initial_state.cloned().unwrap_or_default();
        let state_raw = serde_json::to_string_pretty(&state)
            .map_err(|e| PluginError::Serialize(e, name.to_string()))?;
        std::fs::write(plugin_dir.join("state.json"), state_raw)
            .map_err(|e| PluginError::Io(e, plugin_dir.join("state.json")))?;

        // Write optional HTML template
        if let Some(template) = html_template {
            std::fs::write(plugin_dir.join("index.html"), template)
                .map_err(|e| PluginError::Io(e, plugin_dir.join("index.html")))?;
        }

        // Insert into in-memory state
        self.states.insert(name.to_string(), state);

        // Reload to pick up the new plugin
        self.load()?;

        Ok(self.get(name).expect("just loaded"))
    }

    // ── Agent prompt context ─────────────────────────────────────────────

    /// Generate prompt context describing all loaded plugins to the agent.
    pub fn prompt_context(&self) -> String {
        let plugins = &self.plugins;
        if plugins.is_empty() {
            return String::new();
        }

        let mut ctx = String::from("## Plugins (UI Panels)\n\n");
        ctx.push_str(
            "These plugins render interactive panels beside the chat. You control them with:\n",
        );
        ctx.push_str("- `plugin_list` — list all plugins and their current state\n");
        ctx.push_str("- `plugin_state_get(plugin_name)` — read a plugin's state\n");
        ctx.push_str("- `plugin_state_set(plugin_name, state)` — update a plugin's state (full replacement)\n");
        ctx.push_str("- `plugin_state_patch(plugin_name, patch)` — merge partial state update\n");
        ctx.push_str("- `plugin_create(...)` — create a new plugin from scratch\n\n");

        ctx.push_str("<available_plugins>\n");
        for p in plugins {
            ctx.push_str("  <plugin>\n");
            ctx.push_str(&format!("    <name>{}</name>\n", p.name));
            if let Some(ref emoji) = p.emoji {
                ctx.push_str(&format!("    <emoji>{}</emoji>\n", emoji));
            }
            ctx.push_str(&format!(
                "    <description>{}</description>\n",
                p.description
            ));
            if let Some(ref state) = self.states.get(&p.name) {
                let compact = serde_json::to_string(state).unwrap_or_default();
                ctx.push_str(&format!(
                    "    <state>{}</state>\n",
                    truncate_for_prompt(&compact, 256)
                ));
            }
            if !p.actions.is_empty() {
                ctx.push_str("    <actions>\n");
                for a in &p.actions {
                    ctx.push_str(&format!(
                        "      <action name=\"{}\" description=\"{}\"/>\n",
                        a.name, a.description
                    ));
                }
                ctx.push_str("    </actions>\n");
            }
            ctx.push_str("  </plugin>\n");
        }
        ctx.push_str("</available_plugins>\n\n");
        ctx
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Get the default plugins directory for a workspace.
pub fn default_plugins_dir(workspace: &Path) -> PathBuf {
    workspace.join("plugins")
}

/// Truncate a string for prompt context display.
fn truncate_for_prompt(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

/// Simple JSON merge: recursively merge `patch` into `target`.
fn json_merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                match (t.get_mut(k), v) {
                    (Some(Value::Object(_)), Value::Object(_)) => {
                        json_merge(t.get_mut(k).expect("key present: matched Some in guard above"), v);
                    }
                    _ => {
                        t.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (t, p) => {
            *t = p.clone();
        }
    }
}

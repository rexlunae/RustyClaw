//! Plugin system — dynamically-loaded UI panels the agent and user can
//! interact with, rendered beside the chat window.
//!
//! ## Structure
//!
//! Each plugin lives in `<workspace>/plugins/<name>/` with:
//!   - `plugin.toml` — metadata, state schema, actions, and instructions
//!   - `state.toml`  — the plugin's current state, persisted on every change
//!   - `index.html`  — (optional) HTML template for custom rendering
//!
//! Like every other file this project writes, plugin manifests and state are
//! TOML. The one wrinkle is `null`: TOML has no null, so a `null` in state is
//! dropped at the storage boundary — a null object value is omitted (the same
//! shape JSON Merge Patch uses to delete a key) and a null array element is
//! skipped. In memory and on the wire, state is still JSON (`serde_json::Value`):
//! that is the tool-call and gateway protocol, not storage.
//!
//! ## Communication
//!
//! ```text
//! Agent ←→ plugin_* tools          (read/write state, invoke actions)
//! Agent ←→ Gateway ←→ plugin events (state push to desktop/TUI clients)
//! User  → Desktop UI → Gateway      (user interactions are forwarded)
//! ```
//!
//! ## Plugin TOML Format
//!
//! ```toml
//! name = "chart"
//! description = "Render live charts from data"
//! version = "1.0.0"
//! emoji = "📊"
//! instructions = "## Agent Instructions\n\nUse plugin_state_set to push data."
//!
//! [state_schema]
//! type = "object"
//!
//! [state_schema.properties]
//! title = { type = "string" }
//! chartType = { type = "string", enum = ["bar", "line", "pie"] }
//! data = { type = "array" }
//!
//! [initial_state]
//! title = "My Chart"
//! chartType = "bar"
//! data = []
//!
//! [[actions]]
//! name = "refresh"
//! description = "Re-fetch data"
//!
//! [[actions]]
//! name = "export"
//! description = "Export as CSV"
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod error;
pub use error::PluginError;

// ── Plugin types ──────────────────────────────────────────────────────────

/// A loaded plugin from disk.
///
/// This is the in-memory shape; the on-disk manifest (`plugin.toml`) is the
/// [`PluginFile`] mirror, which carries the two free-form fields as TOML
/// values. State stays `serde_json::Value` because it travels over the tool
/// layer and the gateway wire as JSON — only files on disk are TOML.
#[derive(Debug, Clone)]
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
    /// Schema for the plugin state (stored as TOML in `plugin.toml`).
    pub state_schema: Value,
    /// Initial state (default values).
    pub initial_state: Value,
    /// Named actions the agent can invoke.
    pub actions: Vec<PluginAction>,
    /// Whether this plugin is enabled.
    pub enabled: bool,
    /// Custom HTML template path (relative to plugin dir, or None for default).
    pub html_template: Option<String>,
}

/// On-disk manifest shape (`plugin.toml`).
///
/// Mirrors [`Plugin`], except that the two free-form fields
/// (`state_schema`, `initial_state`) are TOML values; they convert to
/// `serde_json::Value` for the in-memory type.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PluginFile {
    name: String,
    description: String,
    version: String,
    emoji: Option<String>,
    instructions: String,
    #[serde(default)]
    state_schema: Option<toml::Value>,
    #[serde(default)]
    initial_state: Option<toml::Value>,
    #[serde(default)]
    actions: Vec<PluginAction>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    html_template: Option<String>,
}

impl From<PluginFile> for Plugin {
    fn from(file: PluginFile) -> Self {
        Self {
            name: file.name,
            description: file.description,
            version: file.version,
            emoji: file.emoji,
            instructions: file.instructions,
            path: PathBuf::new(), // filled in by the loader
            state_schema: file
                .state_schema
                .as_ref()
                .map(toml_to_json)
                .unwrap_or_default(),
            initial_state: file
                .initial_state
                .as_ref()
                .map(toml_to_json)
                .unwrap_or_default(),
            actions: file.actions,
            enabled: file.enabled,
            html_template: file.html_template,
        }
    }
}

impl From<&Plugin> for PluginFile {
    fn from(plugin: &Plugin) -> Self {
        Self {
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            version: plugin.version.clone(),
            emoji: plugin.emoji.clone(),
            instructions: plugin.instructions.clone(),
            state_schema: json_to_toml(&plugin.state_schema),
            initial_state: json_to_toml(&plugin.initial_state),
            actions: plugin.actions.clone(),
            enabled: plugin.enabled,
            html_template: plugin.html_template.clone(),
        }
    }
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
            let manifest = path.join("plugin.toml");
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
        let file: PluginFile =
            toml::from_str(&raw).map_err(|e| PluginError::Parse(e, manifest.to_path_buf()))?;
        let mut plugin = Plugin::from(file);
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
        let state_file = plugin.path.join("state.toml");
        match std::fs::read_to_string(&state_file) {
            Ok(raw) => match toml::from_str::<toml::Value>(&raw) {
                Ok(state) => toml_to_json(&state),
                Err(e) => {
                    // Falling back to initial_state used to discard the
                    // saved state silently — and the next save_state wrote
                    // the default over it. Keep the bytes aside.
                    crate::persist::quarantine(&state_file, &e.to_string());
                    plugin.initial_state.clone()
                }
            },
            Err(_) => plugin.initial_state.clone(),
        }
    }

    /// Persist a plugin's state to disk. Atomic, and re-creates the plugin
    /// directory if something removed it since install.
    pub fn save_state(&self, plugin_name: &str) -> Result<(), PluginError> {
        if let Some(plugin) = self.get(plugin_name) {
            let state = self.states.get(plugin_name).cloned().unwrap_or_default();
            let state_file = plugin.path.join("state.toml");
            let raw = json_to_toml(&state)
                .map(|v| toml::to_string_pretty(&v))
                .transpose()
                .map_err(|e| PluginError::Serialize(e, plugin_name.to_string()))?
                .unwrap_or_default();
            crate::persist::write_atomically(&state_file, raw.as_bytes())
                .map_err(|e| PluginError::Io(e, state_file))?;
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
        if name.contains('/')
            || name.contains(' ')
            || name.contains('\\')
            || name == "."
            || name == ".."
            || name.starts_with("./")
            || name.starts_with("../")
            || name.contains("/../")
            || name.contains("/./")
        {
            return Err(PluginError::Invalid(
                "name must be a simple identifier (no slashes, spaces, or path segments)".into(),
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

        let manifest = toml::to_string_pretty(&PluginFile::from(&plugin))
            .map_err(|e| PluginError::Serialize(e, name.to_string()))?;
        std::fs::write(plugin_dir.join("plugin.toml"), &manifest)
            .map_err(|e| PluginError::Io(e, plugin_dir.join("plugin.toml")))?;

        // Write initial state
        let state = initial_state.cloned().unwrap_or_default();
        let state_raw = json_to_toml(&state)
            .map(|v| toml::to_string_pretty(&v))
            .transpose()
            .map_err(|e| PluginError::Serialize(e, name.to_string()))?
            .unwrap_or_default();
        std::fs::write(plugin_dir.join("state.toml"), state_raw)
            .map_err(|e| PluginError::Io(e, plugin_dir.join("state.toml")))?;

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

/// Truncate a string for prompt context display, respecting UTF-8 code point
/// boundaries. `max` is a byte budget, since this feeds a prompt-size limit.
fn truncate_for_prompt(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", crate::text::truncate_bytes(s, max))
    }
}

/// Simple JSON merge: recursively merge `patch` into `target`.
fn json_merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                match (t.get_mut(k), v) {
                    (Some(Value::Object(_)), Value::Object(_)) => {
                        json_merge(
                            t.get_mut(k)
                                .expect("key present: matched Some in guard above"),
                            v,
                        );
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

/// Convert a JSON value into the TOML value stored on disk.
///
/// TOML has no `null`, so null is dropped at the storage boundary: a null
/// object value is omitted (the same shape JSON Merge Patch uses to delete a
/// key), a null array element is skipped, and a wholly null value has no TOML
/// representation at all (`None`). A `u64` beyond `i64::MAX` — TOML integers
/// are 64-bit signed — is stored as a float rather than dropped.
fn json_to_toml(value: &Value) -> Option<toml::Value> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(toml::Value::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(toml::Value::Integer(i))
            } else if let Some(u) = n.as_u64() {
                i64::try_from(u)
                    .map(toml::Value::Integer)
                    .ok()
                    .or(Some(toml::Value::Float(u as f64)))
            } else {
                n.as_f64().map(toml::Value::Float)
            }
        }
        Value::String(s) => Some(toml::Value::String(s.clone())),
        Value::Array(items) => Some(toml::Value::Array(
            items.iter().filter_map(json_to_toml).collect(),
        )),
        Value::Object(map) => {
            let mut table = toml::Table::new();
            for (k, v) in map {
                if let Some(tv) = json_to_toml(v) {
                    table.insert(k.clone(), tv);
                }
            }
            Some(toml::Value::Table(table))
        }
    }
}

/// Convert a TOML value back into the JSON value used in memory and on the
/// wire. Datetimes have no JSON representation and become their RFC 3339
/// string form.
fn toml_to_json(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::from(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(items) => Value::Array(items.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_prompt_ascii() {
        assert_eq!(truncate_for_prompt("hello", 5), "hello");
        assert_eq!(truncate_for_prompt("hello world", 5), "hello…");
    }

    #[test]
    fn truncate_prompt_emoji_safe() {
        // 😀 is 4 bytes — truncating at byte 3 falls inside the emoji
        // and must walk back to boundary 0, yielding just the ellipsis.
        assert_eq!(truncate_for_prompt("😀🎉", 4), "😀…");
        assert_eq!(truncate_for_prompt("😀🎉", 3), "…");
    }

    #[test]
    fn truncate_prompt_multibyte_boundary() {
        // é is 2 bytes — "café" = 5 bytes, boundaries at 0,1,2,5
        assert_eq!(truncate_for_prompt("café", 3), "caf…");
        assert_eq!(truncate_for_prompt("café", 2), "ca…");
    }

    #[test]
    fn truncate_prompt_no_truncation() {
        assert_eq!(truncate_for_prompt("abc", 10), "abc");
    }

    #[test]
    fn json_toml_roundtrip_preserves_typed_values() {
        let json = serde_json::json!({
            "title": "Sales",
            "count": 3,
            "ratio": 0.5,
            "enabled": true,
            "tags": ["a", "b"],
            "nested": { "deep": [1, 2, 3] },
        });
        let toml = json_to_toml(&json).expect("no nulls to drop");
        assert_eq!(toml_to_json(&toml), json);
    }

    #[test]
    fn null_values_are_dropped_at_the_storage_boundary() {
        let json = serde_json::json!({
            "keep": 1,
            "delete": null,
            "list": [null, 2, null],
        });
        let toml = json_to_toml(&json).expect("the object itself is storable");
        let raw = toml::to_string_pretty(&toml).unwrap();
        assert!(
            !raw.contains("delete"),
            "null object values must not be stored"
        );
        assert!(raw.contains("keep = 1"), "non-null values must survive");
        // A null array element is skipped, not stored as anything.
        let back = toml_to_json(&toml);
        assert_eq!(back["list"], serde_json::json!([2]));
    }

    #[test]
    fn wholly_null_value_has_no_toml_representation() {
        assert!(json_to_toml(&Value::Null).is_none());
    }

    #[test]
    fn manifest_roundtrips_through_toml() {
        let plugin = Plugin {
            name: "chart".into(),
            description: "Render live charts".into(),
            version: "1.0.0".into(),
            emoji: Some("📊".into()),
            instructions: "## Agent\n\nUse plugin_state_set.".into(),
            path: PathBuf::from("/tmp/chart"),
            state_schema: serde_json::json!({
                "type": "object",
                "properties": { "title": { "type": "string" } }
            }),
            initial_state: serde_json::json!({ "title": "My Chart" }),
            actions: vec![PluginAction {
                name: "refresh".into(),
                description: "Re-fetch data".into(),
                params: vec![],
            }],
            enabled: true,
            html_template: None,
        };

        let raw = toml::to_string_pretty(&PluginFile::from(&plugin)).unwrap();
        let back = Plugin::from(toml::from_str::<PluginFile>(&raw).unwrap());

        assert_eq!(back.name, plugin.name);
        assert_eq!(back.description, plugin.description);
        assert_eq!(back.state_schema, plugin.state_schema);
        assert_eq!(back.initial_state, plugin.initial_state);
        assert_eq!(back.actions.len(), plugin.actions.len());
        assert_eq!(back.actions[0].name, "refresh");
    }

    #[test]
    fn create_then_reload_roundtrips_plugin_and_state() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        mgr.create(
            "demo",
            "A demo plugin",
            Some("📊"),
            "Use plugin_state_set.",
            Some(&serde_json::json!({ "type": "object" })),
            Some(&serde_json::json!({ "tick": 1 })),
            None,
            None,
        )
        .unwrap();

        // A fresh manager reads the same files back from disk.
        let mut reloaded = PluginManager::new(tmp.path().to_path_buf());
        reloaded.load().unwrap();
        assert_eq!(reloaded.plugins().len(), 1);
        assert_eq!(
            reloaded.get_state("demo").unwrap(),
            serde_json::json!({ "tick": 1 })
        );

        // The files on disk are TOML, not JSON.
        let dir = tmp.path().join("demo");
        assert!(dir.join("plugin.toml").exists());
        assert!(dir.join("state.toml").exists());
        assert!(!dir.join("plugin.json").exists());
        assert!(!dir.join("state.json").exists());
    }

    #[test]
    fn state_persists_through_save_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        mgr.create(
            "counter",
            "Counts things",
            None,
            "",
            None,
            Some(&serde_json::json!({ "n": 0 })),
            None,
            None,
        )
        .unwrap();

        // Mutate and persist.
        mgr.set_state("counter", serde_json::json!({ "n": 42 }))
            .unwrap();
        drop(mgr);

        // A fresh manager sees the persisted state, not the initial state.
        let mut reloaded = PluginManager::new(tmp.path().to_path_buf());
        reloaded.load().unwrap();
        assert_eq!(
            reloaded.get_state("counter").unwrap(),
            serde_json::json!({ "n": 42 })
        );
    }
}

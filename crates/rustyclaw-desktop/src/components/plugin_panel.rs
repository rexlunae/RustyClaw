//! Plugin panel for the plugin dock (the workspace's right-hand column).
//!
//! Renders plugin states as interactive panels. Each plugin can:
//! - Show a title with emoji
//! - Display JSON state as auto-rendered key-value pairs
//! - Have action buttons the user can click

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Snapshot of one plugin's state, received from the gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginSnapshot {
    pub name: String,
    pub description: String,
    pub emoji: Option<String>,
    pub version: String,
    pub enabled: bool,
    pub state: Value,
    pub actions: Vec<PluginActionInfo>,
    pub html_template: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginActionInfo {
    pub name: String,
    pub description: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct PluginPanelProps {
    pub plugins: Vec<PluginSnapshot>,
    pub active_plugin: Option<String>,
    pub on_select_plugin: EventHandler<String>,
    pub on_action: EventHandler<PluginActionEvent>,
    pub on_refresh: EventHandler<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginActionEvent {
    pub plugin_name: String,
    pub action_name: String,
}

#[component]
pub fn PluginPanel(props: PluginPanelProps) -> Element {
    let plugins = props.plugins;
    let active_name = props.active_plugin;
    let on_select_plugin = props.on_select_plugin;
    let on_action = props.on_action;
    let on_refresh = props.on_refresh;

    // Find active plugin data (if any) before entering rsx.
    let active_plugin_data: Option<(
        String,                // emoji
        String,                // name
        String,                // description
        Value,                 // state
        Vec<PluginActionInfo>, // actions
    )> = plugins
        .iter()
        .find(|p| Some(&p.name) == active_name.as_ref())
        .map(|p| {
            (
                p.emoji.clone().unwrap_or_else(|| "📦".into()),
                p.name.clone(),
                p.description.clone(),
                p.state.clone(),
                p.actions.clone(),
            )
        });

    let empty = plugins.is_empty();

    rsx! {
        div { class: "plugin-panel",
            if empty {
                div { class: "plugin-panel-empty",
                    div { class: "plugin-panel-empty-icon", "🔌" }
                    div { class: "plugin-panel-empty-text", "No plugins loaded" }
                    div { class: "plugin-panel-empty-hint",
                        "Ask the agent to create a plugin with "
                        code { "plugin_create" }
                    }
                }
            } else {
                div { class: "plugin-panel-tabs",
                    for plugin in plugins.iter() {
                        {
                            let is_active = active_name.as_ref().is_some_and(|n| n == &plugin.name);
                            let cls = if is_active { "plugin-tab active" } else { "plugin-tab" };
                            let plugin_name = plugin.name.clone();
                            let emoji = plugin.emoji.clone().unwrap_or_else(|| "📦".into());
                            let on_select = on_select_plugin;
                            rsx! {
                                div {
                                    key: "{plugin.name}",
                                    class: "{cls}",
                                    onclick: move |_| on_select.call(plugin_name.clone()),
                                    span { class: "plugin-tab-emoji", "{emoji}" }
                                    span { class: "plugin-tab-name", "{plugin.name}" }
                                }
                            }
                        }
                    }
                }
                if let Some((emoji, name, desc, state, actions)) = active_plugin_data {
                    ActivePluginContent {
                        active_emoji: emoji,
                        active_name: name,
                        active_desc: desc,
                        state,
                        actions,
                        on_action,
                        on_refresh,
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct ActivePluginContentProps {
    active_emoji: String,
    active_name: String,
    active_desc: String,
    state: Value,
    actions: Vec<PluginActionInfo>,
    on_action: EventHandler<PluginActionEvent>,
    on_refresh: EventHandler<String>,
}

#[component]
fn ActivePluginContent(props: ActivePluginContentProps) -> Element {
    let refresh_name = props.active_name.clone();
    let plugin_name = props.active_name.clone();
    let actions = props.actions.clone();
    let state = props.state.clone();
    let on_action = props.on_action;
    let on_refresh = props.on_refresh;

    rsx! {
        div { class: "plugin-panel-content",
            div { class: "plugin-panel-header",
                span { class: "plugin-panel-emoji", "{props.active_emoji}" }
                span { class: "plugin-panel-title", "{props.active_name}" }
                button {
                    class: "plugin-refresh-btn",
                    onclick: move |_| on_refresh.call(refresh_name.clone()),
                    title: "Refresh plugin state",
                    "↻"
                }
            }

            p { class: "plugin-panel-description", "{props.active_desc}" }

            if !actions.is_empty() {
                div { class: "plugin-panel-actions",
                    for action in actions.iter() {
                        {
                            let plugin_name = plugin_name.clone();
                            let action_name = action.name.clone();
                            let desc = action.description.clone();
                            rsx! {
                                button {
                                    key: "{action.name}",
                                    class: "plugin-action-btn",
                                    onclick: move |_| {
                                        on_action.call(PluginActionEvent {
                                            plugin_name: plugin_name.clone(),
                                            action_name: action_name.clone(),
                                        });
                                    },
                                    "{desc}"
                                }
                            }
                        }
                    }
                }
            }

            div { class: "plugin-panel-state",
                PluginStateDisplay { state }
            }
        }
    }
}

/// Renders a JSON value as a pretty tree of key-value pairs.
#[component]
fn PluginStateDisplay(state: Value) -> Element {
    match &state {
        Value::Object(map) if map.is_empty() => {
            rsx! {
                div { class: "plugin-state-empty", "No data" }
            }
        }
        Value::Object(map) => {
            let entries: Vec<(String, String)> = map
                .iter()
                .map(|(k, v)| (k.clone(), render_value(v)))
                .collect();
            rsx! {
                div { class: "plugin-state-fields",
                    for (key, val) in entries.iter() {
                        div { class: "plugin-state-field",
                            span { class: "plugin-state-key", "{key}" }
                            span { class: "plugin-state-value", "{val}" }
                        }
                    }
                }
            }
        }
        Value::Array(arr) => {
            let items: Vec<(usize, String)> = arr
                .iter()
                .enumerate()
                .map(|(i, v)| (i, render_value(v)))
                .collect();
            rsx! {
                div { class: "plugin-state-list",
                    for (i, val) in items.iter() {
                        div { class: "plugin-state-list-item",
                            span { class: "plugin-state-index", "[{i}]" }
                            span { class: "plugin-state-value", "{val}" }
                        }
                    }
                }
            }
        }
        other => {
            let s = render_value(other);
            rsx! { span { class: "plugin-state-scalar", "{s}" } }
        }
    }
}

fn render_value(val: &Value) -> String {
    match val {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string_pretty(val).unwrap_or_else(|_| format!("{val}"))
        }
    }
}

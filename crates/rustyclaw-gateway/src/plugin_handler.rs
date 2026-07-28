//! Plugin client-frame handlers (list / refresh).
//!
//! Plugins are loaded and mutated gateway-side — the agent drives them through
//! the `plugin_*` tools. Until this module existed they had no wire
//! representation at all, so a client's plugin panel had nothing to render and
//! always showed an empty state. These handlers push the manager's view of
//! every plugin, and its live state, to the connected client.

use anyhow::Result;
use tracing::debug;

use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, protocol, transport};

/// Build the wire snapshot of every loaded plugin.
///
/// State is carried as a JSON string: the wire format is bincode, which cannot
/// encode a `serde_json::Value` directly, and re-modelling arbitrary plugin
/// state as a typed enum would defeat the point of a schema-driven plugin.
fn snapshots() -> Vec<protocol::PluginInfoDto> {
    rustyclaw_core::tools::plugin_snapshots()
        .into_iter()
        .map(|(plugin, state)| protocol::PluginInfoDto {
            name: plugin.name,
            description: plugin.description,
            emoji: plugin.emoji,
            version: plugin.version,
            enabled: plugin.enabled,
            // A state value that will not serialize is a broken plugin, not a
            // reason to drop the whole list — send an empty object and say so.
            state_json: serde_json::to_string(&state).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Plugin state did not serialize; sending empty state");
                "{}".to_string()
            }),
            actions: plugin
                .actions
                .into_iter()
                .map(|a| protocol::PluginActionDto {
                    name: a.name,
                    description: a.description,
                })
                .collect(),
            html_template: plugin.html_template,
        })
        .collect()
}

/// Send the current plugin list and state to the client.
pub(crate) async fn send_plugins_update(writer: &mut dyn transport::TransportWriter) -> Result<()> {
    let plugins = snapshots();
    debug!(count = plugins.len(), "Sending PluginsUpdate");
    let frame = ServerFrame {
        frame_type: ServerFrameType::PluginsUpdate,
        payload: ServerPayload::PluginsUpdate { plugins },
    };
    send_frame(writer, &frame).await
}

/// Handle a `PluginList`: push the current plugin list.
pub(crate) async fn handle_plugin_list(writer: &mut dyn transport::TransportWriter) -> Result<()> {
    send_plugins_update(writer).await
}

/// Handle a `PluginRefresh`: re-read plugin state from disk, then push.
///
/// State writes go straight to disk (`PluginManager::set_state` persists
/// immediately), so re-reading is authoritative and cannot lose an unsaved
/// in-memory edit.
pub(crate) async fn handle_plugin_refresh(
    writer: &mut dyn transport::TransportWriter,
    plugin_name: String,
) -> Result<()> {
    debug!(plugin = %plugin_name, "Plugin refresh request");
    if let Err(e) = rustyclaw_core::tools::reload_plugins() {
        // Reporting beats a refresh button that silently does nothing.
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: format!("Could not reload plugins: {e}"),
            },
        };
        return send_frame(writer, &frame).await;
    }
    send_plugins_update(writer).await
}

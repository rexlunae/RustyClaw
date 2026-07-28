//! Tools for Freenet and the apps built on it.
//!
//! Three tools live here:
//!
//! * `freenet` — the generic layer: node status, and raw contract get/put/update.
//! * `river` — River group chat: read a room, list its members, post a message.
//! * `atlas` — Atlas discovery: search, browse, and look up an index.
//!
//! All three talk to a **local** Freenet node over its WebSocket command API;
//! the node does the network work. See [`client`] for how the node is located.
//!
//! # When the `freenet` feature is off
//!
//! The tools stay registered and each returns an error naming the feature. The
//! alternative — hiding them — would leave a caller unable to tell "this build
//! cannot do that" from "that is not a thing", and a silent absence is exactly
//! the failure mode worth avoiding here. The dependencies (a WebSocket client,
//! ed25519, River's contract types) are too heavy for the default build.

use std::path::Path;

use serde_json::Value;

use crate::tools::ToolParam;
use crate::tools::error::ToolResult;

#[cfg(feature = "freenet")]
pub mod atlas;
#[cfg(feature = "freenet")]
pub mod client;
#[cfg(feature = "freenet")]
pub mod river;

/// Default number of items returned by a listing action.
///
/// This and the three helpers below are reached only from the feature-on
/// executors (and the tests), so a build without `freenet` sees them as dead.
#[cfg_attr(not(feature = "freenet"), allow(dead_code))]
const DEFAULT_LIMIT: usize = 20;
/// Ceiling on `limit`, so a tool result cannot blow out the context window.
#[cfg_attr(not(feature = "freenet"), allow(dead_code))]
const MAX_LIMIT: usize = 200;

/// Read a required string argument.
fn required<'a>(args: &'a Value, name: &str) -> ToolResult<&'a str> {
    args.get(name)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Missing required parameter: {name}").into())
}

/// Read an optional string argument, treating blank as absent.
#[cfg_attr(not(feature = "freenet"), allow(dead_code))]
fn optional<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
}

/// Read the `limit` argument, clamped to something a caller can actually read.
#[cfg_attr(not(feature = "freenet"), allow(dead_code))]
fn limit(args: &Value) -> usize {
    args.get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(1, MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT)
}

/// The error returned by every action when the crate was built without the
/// `freenet` feature.
#[cfg(not(feature = "freenet"))]
fn feature_disabled(tool: &str) -> crate::tools::error::ToolError {
    format!(
        "The `{tool}` tool needs the `freenet` feature, which this build does not \
         have. Rebuild rustyclaw-core with `--features freenet` (it is included in \
         `--features full`). It is off by default because it pulls in a WebSocket \
         client, ed25519, and River's contract types."
    )
    .into()
}

// ── freenet ─────────────────────────────────────────────────────────────────

/// Parameters for the `freenet` tool.
pub fn freenet_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "One of: status (node and peer summary), get (fetch a \
                          contract's state), put (publish a contract), update \
                          (apply a delta to a contract)."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "contract".into(),
            description: "Contract instance id (43-44 base58 chars). A \
                          `freenet:<id><path>` locator is also accepted. Required \
                          for get and update."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "output_file".into(),
            description: "For get: write the raw state bytes to this path instead \
                          of previewing them. Contract state is arbitrary binary."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "code_file".into(),
            description: "For put: path to the contract's compiled .wasm.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "state_file".into(),
            description: "For put: path to the contract's initial state bytes.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "params_file".into(),
            description: "For put: path to the contract's parameter bytes. \
                          Omit for a contract that takes no parameters."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "delta_file".into(),
            description: "For update: path to the delta bytes to apply. The \
                          encoding is defined by the contract."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "node_url".into(),
            description: "Override the node's WebSocket URL. Defaults to \
                          $FREENET_NODE_URL, then a local node on port 7509."
                .into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

/// Placeholder executor: the `freenet` tool is async-only.
pub fn exec_freenet_stub(_args: &Value, _workspace_dir: &Path) -> ToolResult {
    Err("freenet requires async execution".into())
}

/// Execute the `freenet` tool.
#[cfg(feature = "freenet")]
pub async fn exec_freenet_async(args: &Value, workspace_dir: &Path) -> ToolResult {
    use serde_json::json;

    let action = required(args, "action")?;
    let url = client::node_url(optional(args, "node_url"));

    match action {
        "status" => {
            let mut node = client::NodeClient::connect(&url).await?;
            let peers = node.connected_peers().await?;
            Ok(json!({
                "node_url": url,
                "connected": true,
                "peer_count": peers.len(),
                "peers": peers
                    .iter()
                    .map(|(id, addr)| json!({ "id": id, "address": addr }))
                    .collect::<Vec<_>>(),
            })
            .to_string())
        }
        "get" => {
            let id = client::parse_instance_id(required(args, "contract")?)?;
            let mut node = client::NodeClient::connect(&url).await?;
            let fetched = node.get(id, false).await?;
            match optional(args, "output_file") {
                Some(path) => {
                    let written = write_bytes(workspace_dir, path, &fetched.state)?;
                    Ok(json!({
                        "contract": id.to_string(),
                        "bytes": fetched.state.len(),
                        "written_to": written,
                    })
                    .to_string())
                }
                None => Ok(json!({
                    "contract": id.to_string(),
                    "bytes": fetched.state.len(),
                    "preview": preview(&fetched.state),
                })
                .to_string()),
            }
        }
        "put" => {
            let code = read_bytes(workspace_dir, required(args, "code_file")?)?;
            let state = read_bytes(workspace_dir, required(args, "state_file")?)?;
            let params = match optional(args, "params_file") {
                Some(path) => read_bytes(workspace_dir, path)?,
                None => Vec::new(),
            };
            let mut node = client::NodeClient::connect(&url).await?;
            // Subscribe on publish so the node hosts the contract; without it a
            // remote GET can dead-end even though the PUT succeeded locally.
            let key = node.put(code, params, state, true).await?;
            Ok(json!({ "contract": key.id().to_string(), "published": true }).to_string())
        }
        "update" => {
            let id = client::parse_instance_id(required(args, "contract")?)?;
            let delta = read_bytes(workspace_dir, required(args, "delta_file")?)?;
            let mut node = client::NodeClient::connect(&url).await?;
            // An update is addressed by instance id *and* code hash, and only
            // the node knows the latter — hence the lookup before the write.
            let key = node.key_for(id).await?;
            node.update_delta(key, delta).await?;
            Ok(json!({ "contract": id.to_string(), "updated": true }).to_string())
        }
        other => Err(format!("Unknown action: {other}. Valid: status, get, put, update").into()),
    }
}

/// Execute the `freenet` tool when the feature is off.
#[cfg(not(feature = "freenet"))]
pub async fn exec_freenet_async(args: &Value, _workspace_dir: &Path) -> ToolResult {
    // Validate the action first, so an outright typo is still reported as one.
    required(args, "action")?;
    Err(feature_disabled("freenet"))
}

// ── river ───────────────────────────────────────────────────────────────────

/// Parameters for the `river` tool.
pub fn river_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "One of: read (recent messages), members (who is in the \
                          room), send (post a public text message)."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "room".into(),
            description: "The room's contract instance id, as found in a River \
                          invite link or shown by River's UI."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "message".into(),
            description: "For send: the text to post. The author must already be \
                          a member of the room."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "signing_key".into(),
            description: "For send: a `river:v1:sk:…` signing key. Prefer setting \
                          $RIVER_SIGNING_KEY instead — a key passed here appears \
                          in tool arguments and transcripts."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "limit".into(),
            description: "For read: how many of the most recent messages to \
                          return. Default 20, max 200."
                .into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "node_url".into(),
            description: "Override the node's WebSocket URL.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

/// Placeholder executor: the `river` tool is async-only.
pub fn exec_river_stub(_args: &Value, _workspace_dir: &Path) -> ToolResult {
    Err("river requires async execution".into())
}

/// Execute the `river` tool.
#[cfg(feature = "freenet")]
pub async fn exec_river_async(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let action = required(args, "action")?;
    let room = required(args, "room")?;
    let url = optional(args, "node_url");

    match action {
        "read" => river::read(room, url, limit(args)).await,
        "members" => river::members(room, url).await,
        "send" => {
            let text = required(args, "message")?;
            river::send(room, text, optional(args, "signing_key"), url).await
        }
        other => Err(format!("Unknown action: {other}. Valid: read, members, send").into()),
    }
}

/// Execute the `river` tool when the feature is off.
#[cfg(not(feature = "freenet"))]
pub async fn exec_river_async(args: &Value, _workspace_dir: &Path) -> ToolResult {
    required(args, "action")?;
    Err(feature_disabled("river"))
}

// ── atlas ───────────────────────────────────────────────────────────────────

/// Parameters for the `atlas` tool.
pub fn atlas_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "One of: search (rank entries against a query), list \
                          (browse newest first), get (look up one subject)."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "query".into(),
            description: "For search: whitespace-separated terms. Every term must \
                          match; title hits rank above tags, tags above snippets."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "subject".into(),
            description: "For get: the subject id of the entry to fetch.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "index".into(),
            description: "The index contract's instance id. Atlas is pluralistic \
                          and has no canonical index, so this or \
                          $ATLAS_INDEX_CONTRACT is required."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "limit".into(),
            description: "How many entries to return. Default 20, max 200.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "node_url".into(),
            description: "Override the node's WebSocket URL.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

/// Placeholder executor: the `atlas` tool is async-only.
pub fn exec_atlas_stub(_args: &Value, _workspace_dir: &Path) -> ToolResult {
    Err("atlas requires async execution".into())
}

/// Execute the `atlas` tool.
#[cfg(feature = "freenet")]
pub async fn exec_atlas_async(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let action = required(args, "action")?;
    let index = optional(args, "index");
    let url = optional(args, "node_url");

    match action {
        "search" => atlas::search(required(args, "query")?, index, url, limit(args)).await,
        "list" => atlas::list(index, url, limit(args)).await,
        "get" => atlas::get(required(args, "subject")?, index, url).await,
        other => Err(format!("Unknown action: {other}. Valid: search, list, get").into()),
    }
}

/// Execute the `atlas` tool when the feature is off.
#[cfg(not(feature = "freenet"))]
pub async fn exec_atlas_async(args: &Value, _workspace_dir: &Path) -> ToolResult {
    required(args, "action")?;
    Err(feature_disabled("atlas"))
}

// ── file and preview helpers ────────────────────────────────────────────────

/// Resolve a tool-supplied path, refusing the vault the way the file tools do.
///
/// Without this a contract PUT would be a way to read the credential store
/// through a tool that does not look like a file tool.
#[cfg(feature = "freenet")]
fn resolve_unprotected(workspace_dir: &Path, path: &str) -> ToolResult<std::path::PathBuf> {
    let resolved = crate::tools::helpers::resolve_path(workspace_dir, path);
    if crate::tools::helpers::is_protected_path(&resolved) {
        tracing::warn!(path = %resolved.display(), "Attempted access to protected path");
        return Err(crate::tools::helpers::VAULT_ACCESS_DENIED.into());
    }
    Ok(resolved)
}

/// Read a file through the sandbox-safe open path.
#[cfg(feature = "freenet")]
fn read_bytes(workspace_dir: &Path, path: &str) -> ToolResult<Vec<u8>> {
    use std::io::Read;

    let resolved = resolve_unprotected(workspace_dir, path)?;
    let (mut file, canonical) = crate::tools::helpers::open_file_read_safe(&resolved)
        .map_err(|e| format!("Could not open {}: {e}", resolved.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("Could not read {}: {e}", canonical.display()))?;
    Ok(bytes)
}

/// Write bytes through the sandbox-safe open path, returning where they landed.
#[cfg(feature = "freenet")]
fn write_bytes(workspace_dir: &Path, path: &str, bytes: &[u8]) -> ToolResult<String> {
    use std::io::Write;

    let resolved = resolve_unprotected(workspace_dir, path)?;
    let (mut file, canonical) = crate::tools::helpers::open_file_write_safe(&resolved)
        .map_err(|e| format!("Could not create {}: {e}", resolved.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("Could not write {}: {e}", canonical.display()))?;
    Ok(canonical.display().to_string())
}

/// A readable summary of arbitrary contract state.
///
/// Contract state is opaque bytes. Rendering it as lossy UTF-8 would invent
/// replacement characters and read as corrupt text, so binary is described
/// rather than mangled.
#[cfg(feature = "freenet")]
fn preview(bytes: &[u8]) -> String {
    const MAX_PREVIEW: usize = 2000;
    match std::str::from_utf8(bytes) {
        Ok(text) if text.len() <= MAX_PREVIEW => text.to_string(),
        Ok(text) => format!(
            "{}…\n[truncated; {} bytes total]",
            crate::text::truncate_bytes(text, MAX_PREVIEW),
            bytes.len()
        ),
        Err(_) => {
            let head: String = bytes
                .iter()
                .take(32)
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "[binary, {} bytes; first bytes: {head}] \
                 Use output_file to save it, or a contract-aware tool \
                 (`river`, `atlas`) to decode it.",
                bytes.len()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_blank_argument_counts_as_missing() {
        let args = json!({ "action": "   " });
        assert!(required(&args, "action").is_err());
        assert!(optional(&args, "action").is_none());
    }

    #[test]
    fn limit_is_clamped_to_a_readable_range() {
        assert_eq!(limit(&json!({})), DEFAULT_LIMIT);
        assert_eq!(limit(&json!({ "limit": 0 })), 1);
        assert_eq!(limit(&json!({ "limit": 5 })), 5);
        assert_eq!(limit(&json!({ "limit": 100_000 })), MAX_LIMIT);
    }

    #[test]
    fn the_sync_executors_say_they_need_the_async_path() {
        let args = json!({ "action": "status" });
        let dir = Path::new("/tmp");
        for result in [
            exec_freenet_stub(&args, dir),
            exec_river_stub(&args, dir),
            exec_atlas_stub(&args, dir),
        ] {
            assert!(result.unwrap_err().to_string().contains("async"));
        }
    }

    #[tokio::test]
    async fn an_unknown_action_is_rejected_before_any_connection() {
        // No node is running in a test environment; reaching the network would
        // make this hang rather than fail, so an unknown action must be caught
        // by argument validation first.
        let dir = Path::new("/tmp");
        let err = exec_river_async(&json!({ "action": "nope", "room": "x" }), dir)
            .await
            .expect_err("unknown action should be rejected");
        let text = err.to_string();
        assert!(
            text.contains("nope") || text.contains("freenet` feature"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_missing_action_is_reported_by_name() {
        let dir = Path::new("/tmp");
        let err = exec_atlas_async(&json!({}), dir)
            .await
            .expect_err("a missing action should be rejected");
        assert!(err.to_string().contains("action"), "{err}");
    }

    #[cfg(feature = "freenet")]
    #[test]
    fn a_utf8_state_previews_as_text() {
        assert_eq!(preview(b"hello"), "hello");
    }

    #[cfg(feature = "freenet")]
    #[test]
    fn a_binary_state_is_described_rather_than_mangled() {
        let text = preview(&[0xff, 0xfe, 0x00]);
        assert!(text.contains("binary, 3 bytes"), "{text}");
        assert!(
            !text.contains('\u{fffd}'),
            "must not invent replacement chars"
        );
    }
}

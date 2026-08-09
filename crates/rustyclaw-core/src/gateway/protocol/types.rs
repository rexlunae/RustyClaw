//! Protocol types shared between gateway server and TUI client.
//!
//! This module contains pure data types that represent the communication
//! protocol between client and server. These types have no dependencies
//! on gateway-specific components like Config, SecretsManager, etc.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Reference to a media attachment in a message.
///
/// Media is not stored inline in conversation history. Instead, we store
/// a reference with metadata. The actual data can be:
/// - Downloaded from the original URL (may expire)
/// - Retrieved from the local cache path
/// - Requested via the gateway's media endpoint
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaRef {
    /// Unique ID for referencing this media (e.g., "img_001")
    pub id: String,
    /// MIME type (e.g., "image/jpeg")
    pub mime_type: String,
    /// Original filename if known
    #[serde(default)]
    pub filename: Option<String>,
    /// File size in bytes
    #[serde(default)]
    pub size: Option<usize>,
    /// Original URL (may be temporary/expiring)
    #[serde(default)]
    pub url: Option<String>,
    /// Local cached path (filled after download)
    #[serde(default)]
    pub local_path: Option<String>,
}

impl MediaRef {
    /// Generate a new media ID.
    pub fn new_id() -> String {
        format!("media_{:04}", COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Create a new MediaRef with auto-generated ID.
    pub fn new(mime_type: String) -> Self {
        Self {
            id: Self::new_id(),
            mime_type,
            filename: None,
            size: None,
            url: None,
            local_path: None,
        }
    }

    /// Display placeholder for TUI/text rendering.
    pub fn placeholder(&self) -> String {
        let size_str = self
            .size
            .map(format_size)
            .unwrap_or_else(|| "?".to_string());

        let name = self.filename.as_deref().unwrap_or(&self.id);

        format!("📎 [{}] ({}) - /download {}", name, size_str, self.id)
    }
}

/// Format a byte size for display.
fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// A tool call recorded in a transcript.
///
/// Deliberately native rather than a `serde_json::Value`. Frames are encoded
/// with bincode, which is not self-describing, and `Value` decodes through
/// `deserialize_any` — so a transcript carrying one encodes on the gateway and
/// then fails to decode on the client, with the thread simply never arriving.
/// Threads that have run a tool are exactly the ones worth opening, so the
/// failure lands on the transcripts that matter and spares the trivial ones.
///
/// Mirrors the live [`ServerPayload::ToolCall`] frame, which has always used
/// these three fields: a transcript and the stream that produced it now
/// describe a call the same way.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Provider-assigned id, used to pair a call with its result.
    pub id: String,
    /// Tool name as the model asked for it.
    pub name: String,
    /// Arguments as JSON text, opaque to the protocol — the shape is the
    /// tool's business, and carrying it as text keeps it decodable.
    pub arguments: String,
}

impl ToolCallRecord {
    /// Decode the stored `[{id, name, arguments}, …]` JSON into native records.
    ///
    /// Threads persist as JSON, which is self-describing and can hold a bare
    /// `Value` quite happily; the wire cannot. This is that boundary. Anything
    /// unrecognised degrades to empty strings rather than dropping the call:
    /// a tool call rendered with a missing name still tells the user their
    /// turn ran a tool, where a vanished one does not.
    pub fn from_stored_json(value: &serde_json::Value) -> Vec<Self> {
        let Some(items) = value.as_array() else {
            return Vec::new();
        };
        items
            .iter()
            .map(|tc| Self {
                id: tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                name: tc
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                // Already-text arguments stay as they are; anything else is
                // re-rendered. Going through `to_string` blindly would wrap a
                // string in a second set of quotes every trip.
                arguments: match tc.get("arguments") {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => String::new(),
                },
            })
            .collect()
    }
}

/// A single message in a chat conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    /// Stable message id, minted by the gateway and echoed in history so
    /// clients can refer to a specific message (delete-by-id, …). Clients
    /// may also send one (user messages) so the bubble they show and the
    /// persisted record agree.
    ///
    /// NOTE: no `skip_serializing_if` here on purpose — frames are bincode,
    /// which reads struct fields positionally, so a skipped field would
    /// shift every byte after it.
    #[serde(default)]
    pub id: Option<String>,
    /// Tool calls requested by the assistant.
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallRecord>>,
    /// Tool call ID this message is responding to.
    #[serde(default)]
    pub tool_call_id: Option<String>,
    /// Media attachments (images, files, etc.)
    #[serde(default)]
    pub media: Option<Vec<MediaRef>>,
}

impl ChatMessage {
    /// Create a simple text message.
    pub fn text(role: &str, content: &str) -> Self {
        Self {
            role: role.to_string(),
            content: content.to_string(),
            id: None,
            tool_calls: None,
            tool_call_id: None,
            media: None,
        }
    }

    /// Set the message's stable id.
    pub fn with_id(mut self, id: Option<String>) -> Self {
        self.id = id;
        self
    }

    /// Create a user message with media.
    pub fn user_with_media(content: &str, media: Vec<MediaRef>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.to_string(),
            id: None,
            tool_calls: None,
            tool_call_id: None,
            media: if media.is_empty() { None } else { Some(media) },
        }
    }

    /// Convert wire role text into the shared UI/core role enum.
    pub fn to_core_message_role(&self) -> crate::types::MessageRole {
        match self.role.as_str() {
            "user" => crate::types::MessageRole::User,
            "assistant" => crate::types::MessageRole::Assistant,
            "tool" => crate::types::MessageRole::ToolResult,
            "system" => crate::types::MessageRole::System,
            _ => crate::types::MessageRole::System,
        }
    }

    /// Get display text including media placeholders.
    pub fn display_content(&self) -> String {
        let mut parts = Vec::new();

        if !self.content.is_empty() {
            parts.push(self.content.clone());
        }

        if let Some(media) = &self.media {
            for m in media {
                parts.push(m.placeholder());
            }
        }

        parts.join("\n")
    }
}

/// A parsed tool call from the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// The result of executing a tool locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub id: String,
    pub name: String,
    pub output: String,
    pub is_error: bool,
}

/// A complete model response: optional text + optional tool calls.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ModelResponse {
    pub text: String,
    pub tool_calls: Vec<ParsedToolCall>,
    /// The finish reason from the model (e.g., "stop", "tool_calls", "length").
    pub finish_reason: Option<String>,
    /// Token counts reported by the provider (when available).
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

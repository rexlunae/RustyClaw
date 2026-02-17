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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A single message in a chat conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    /// Tool calls requested by the assistant.
    #[serde(default)]
    pub tool_calls: Option<serde_json::Value>,
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
            tool_calls: None,
            tool_call_id: None,
            media: None,
        }
    }

    /// Create a user message with media.
    pub fn user_with_media(content: &str, media: Vec<MediaRef>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
            media: if media.is_empty() { None } else { Some(media) },
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

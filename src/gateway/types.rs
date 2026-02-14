use crate::config::Config;
use crate::providers;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct GatewayOptions {
    pub listen: String,
}

// ── Chat protocol types ─────────────────────────────────────────────────────

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
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
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
        let size_str = self.size
            .map(|s| format_size(s))
            .unwrap_or_else(|| "?".to_string());
        
        let name = self.filename.as_deref()
            .unwrap_or(&self.id);
        
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

/// An incoming chat request from the TUI.
///
/// All fields except `messages` and `type` are optional — the gateway fills
/// missing values from its own [`ModelContext`] (resolved at startup).
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    /// Must be `"chat"`.
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Conversation messages (system, user, assistant).
    pub messages: Vec<ChatMessage>,
    /// Model name (e.g. `"claude-sonnet-4-20250514"`).
    #[serde(default)]
    pub model: Option<String>,
    /// Provider id (e.g. `"anthropic"`, `"openai"`).
    #[serde(default)]
    pub provider: Option<String>,
    /// API base URL.
    #[serde(default)]
    pub base_url: Option<String>,
    /// API key / bearer token (optional for providers like Ollama).
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Fully-resolved request ready for dispatch to a model provider.
///
/// Created by merging an incoming [`ChatRequest`] with the gateway's
/// [`ModelContext`] defaults.
pub struct ProviderRequest {
    pub messages: Vec<ChatMessage>,
    pub model: String,
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

// ── Model context (resolved once at startup) ────────────────────────────────

/// Pre-resolved model configuration created at gateway startup.
///
/// The gateway reads the configured provider + model from `Config`, fetches
/// the API key from the secrets vault, and holds everything in this struct
/// so per-connection handlers can call the provider without the client
/// needing to send credentials.
#[derive(Debug, Clone)]
pub struct ModelContext {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl ModelContext {
    /// Resolve the model context from the app configuration and secrets vault.
    ///
    /// Returns an error if no `[model]` section is present in the config.
    /// A missing API key is treated as a warning (the provider may not need
    /// one — e.g. Ollama), not a hard error.
    pub fn resolve(config: &Config, secrets: &mut crate::secrets::SecretsManager) -> Result<Self> {
        let mp = config
            .model
            .as_ref()
            .context("No [model] section in config — run `rustyclaw onboard` or add one to config.toml")?;

        let provider = mp.provider.clone();
        let model = mp.model.clone().unwrap_or_default();
        let base_url = mp.base_url.clone().unwrap_or_else(|| {
            providers::base_url_for_provider(&provider)
                .unwrap_or("")
                .to_string()
        });

        let api_key = providers::secret_key_for_provider(&provider).and_then(|key_name| {
            secrets.get_secret(key_name, true).ok().flatten()
        });

        if api_key.is_none() && providers::secret_key_for_provider(&provider).is_some() {
            eprintln!(
                "⚠ No API key found for provider '{}' — model calls will likely fail",
                provider,
            );
        }

        Ok(Self {
            provider,
            model,
            base_url,
            api_key,
        })
    }

    /// Build a model context from configuration and a pre-resolved API key.
    ///
    /// Use this when the caller has already extracted the key (e.g. the CLI
    /// passes just the provider key to the daemon via an environment
    /// variable, so the gateway never needs vault access).
    pub fn from_config(config: &Config, api_key: Option<String>) -> Result<Self> {
        let mp = config
            .model
            .as_ref()
            .context("No [model] section in config — run `rustyclaw onboard` or add one to config.toml")?;

        let provider = mp.provider.clone();
        let model = mp.model.clone().unwrap_or_default();
        let base_url = mp.base_url.clone().unwrap_or_else(|| {
            providers::base_url_for_provider(&provider)
                .unwrap_or("")
                .to_string()
        });

        if api_key.is_none() && providers::secret_key_for_provider(&provider).is_some() {
            eprintln!(
                "⚠ No API key provided for provider '{}' — model calls will likely fail",
                provider,
            );
        }

        Ok(Self {
            provider,
            model,
            base_url,
            api_key,
        })
    }
}

// ── Copilot session token cache ──────────────────────────────────────────────

/// Manages a short-lived Copilot session token, auto-refreshing on expiry.
///
/// GitHub Copilot's chat API requires a session token obtained by
/// exchanging the long-lived OAuth device-flow token.  Session tokens
/// expire after ~30 minutes.  This struct caches the active session and
/// transparently refreshes it when needed.
///
/// Can also be initialized with an imported session token (no OAuth token).
/// In that case, it will use the session token until it expires, then fail.
pub struct CopilotSession {
    /// OAuth token for refreshing (None if using imported session only)
    oauth_token: Option<String>,
    inner: tokio::sync::Mutex<Option<CopilotSessionEntry>>,
}

struct CopilotSessionEntry {
    token: String,
    expires_at: i64,
}

impl CopilotSession {
    /// Create a new session manager wrapping the given OAuth token.
    pub fn new(oauth_token: String) -> Self {
        Self {
            oauth_token: Some(oauth_token),
            inner: tokio::sync::Mutex::new(None),
        }
    }

    /// Create a session manager with an imported session token (no refresh capability).
    pub fn from_session_token(session_token: String, expires_at: i64) -> Self {
        Self {
            oauth_token: None,
            inner: tokio::sync::Mutex::new(Some(CopilotSessionEntry {
                token: session_token,
                expires_at,
            })),
        }
    }

    /// Return a valid session token, exchanging or refreshing as needed.
    ///
    /// Caches the token and only calls the exchange endpoint when the
    /// cached token is missing or within 60 seconds of expiry.
    pub async fn get_token(&self, http: &reqwest::Client) -> Result<String> {
        let mut guard = self.inner.lock().await;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Return cached token if still valid (with 60 s safety margin).
        if let Some(ref entry) = *guard {
            if now < entry.expires_at - 60 {
                return Ok(entry.token.clone());
            }
        }

        // Need to refresh - check if we have an OAuth token
        let oauth_token = match &self.oauth_token {
            Some(t) => t,
            None => {
                anyhow::bail!(
                    "Copilot session token has expired. Please re-authenticate with: rustyclaw onboard"
                );
            }
        };

        // Exchange the OAuth token for a fresh session token.
        let session = providers::exchange_copilot_session(http, oauth_token)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let token = session.token.clone();
        *guard = Some(CopilotSessionEntry {
            token: session.token,
            expires_at: session.expires_at,
        });
        Ok(token)
    }
}

// ── Model response types (shared across providers) ──────────────────────────

/// A parsed tool call from the model.
#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// The result of executing a tool locally.
#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub id: String,
    pub name: String,
    pub output: String,
    pub is_error: bool,
}

/// A complete model response: optional text + optional tool calls.
#[derive(Debug, Default)]
pub struct ModelResponse {
    pub text: String,
    pub tool_calls: Vec<ParsedToolCall>,
    /// Token counts reported by the provider (when available).
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

/// Result of a model connection probe.
pub enum ProbeResult {
    /// Provider responded successfully — everything works.
    Ready,
    /// Authenticated and reachable, but the specific model or request format
    /// wasn't accepted (e.g. 400 "model not supported").  Chat may still
    /// work with the real request format.
    Connected { warning: String },
    /// Hard failure — authentication rejected (401/403).
    AuthError { detail: String },
    /// Hard failure — network error or unexpected server error.
    Unreachable { detail: String },
}

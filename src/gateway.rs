use crate::config::Config;
use crate::providers;
use crate::secrets::SecretsManager;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Debug, Clone)]
pub struct GatewayOptions {
    pub listen: String,
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
    pub fn resolve(config: &Config, secrets: &mut SecretsManager) -> Result<Self> {
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
}

// ── Chat protocol types ─────────────────────────────────────────────────────

/// A single message in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// An incoming chat request from the TUI.
///
/// All fields except `messages` and `type` are optional — the gateway fills
/// missing values from its own [`ModelContext`] (resolved at startup).
#[derive(Debug, Deserialize)]
struct ChatRequest {
    /// Must be `"chat"`.
    #[serde(rename = "type")]
    msg_type: String,
    /// Conversation messages (system, user, assistant).
    messages: Vec<ChatMessage>,
    /// Model name (e.g. `"claude-sonnet-4-20250514"`).
    #[serde(default)]
    model: Option<String>,
    /// Provider id (e.g. `"anthropic"`, `"openai"`).
    #[serde(default)]
    provider: Option<String>,
    /// API base URL.
    #[serde(default)]
    base_url: Option<String>,
    /// API key / bearer token (optional for providers like Ollama).
    #[serde(default)]
    api_key: Option<String>,
}

/// Fully-resolved request ready for dispatch to a model provider.
///
/// Created by merging an incoming [`ChatRequest`] with the gateway's
/// [`ModelContext`] defaults.
struct ProviderRequest {
    messages: Vec<ChatMessage>,
    model: String,
    provider: String,
    base_url: String,
    api_key: Option<String>,
}

/// Merge an incoming chat request with the gateway's model context.
///
/// Fields present in the request take priority; missing fields fall back
/// to the gateway defaults.  Returns an error message string if a required
/// field cannot be resolved from either source.
fn resolve_request(
    req: ChatRequest,
    ctx: Option<&ModelContext>,
) -> std::result::Result<ProviderRequest, String> {
    let provider = req
        .provider
        .or_else(|| ctx.map(|c| c.provider.clone()))
        .ok_or_else(|| "No provider specified and gateway has no model configured".to_string())?;
    let model = req
        .model
        .or_else(|| ctx.map(|c| c.model.clone()))
        .ok_or_else(|| "No model specified and gateway has no model configured".to_string())?;
    let base_url = req
        .base_url
        .or_else(|| ctx.map(|c| c.base_url.clone()))
        .ok_or_else(|| "No base_url specified and gateway has no model configured".to_string())?;
    let api_key = req
        .api_key
        .or_else(|| ctx.and_then(|c| c.api_key.clone()));

    Ok(ProviderRequest {
        messages: req.messages,
        model,
        provider,
        base_url,
        api_key,
    })
}

/// Run the gateway WebSocket server.
///
/// Accepts connections in a loop until the `cancel` token is triggered,
/// at which point the server shuts down gracefully.
///
/// When `model_ctx` is provided the gateway owns the provider credentials
/// and every chat request is resolved against that context.  If `None`,
/// clients must send full `ChatRequest` payloads including provider info.
pub async fn run_gateway(
    config: Config,
    options: GatewayOptions,
    model_ctx: Option<ModelContext>,
    cancel: CancellationToken,
) -> Result<()> {
    let addr = resolve_listen_addr(&options.listen)?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind gateway to {}", addr))?;

    let model_ctx = model_ctx.map(Arc::new);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let config_clone = config.clone();
                let ctx_clone = model_ctx.clone();
                let child_cancel = cancel.child_token();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(stream, peer, config_clone, ctx_clone, child_cancel).await {
                        eprintln!("Gateway connection error from {}: {}", peer, err);
                    }
                });
            }
        }
    }

    Ok(())
}

fn resolve_listen_addr(listen: &str) -> Result<SocketAddr> {
    let trimmed = listen.trim();
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        let url = Url::parse(trimmed).context("Invalid WebSocket URL")?;
        let host = url.host_str().context("WebSocket URL missing host")?;
        let port = url
            .port_or_known_default()
            .context("WebSocket URL missing port")?;
        let addr = format!("{}:{}", host, port);
        return addr
            .parse()
            .with_context(|| format!("Invalid listen address {}", addr));
    }

    trimmed
        .parse()
        .with_context(|| format!("Invalid listen address {}", trimmed))
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    _peer: SocketAddr,
    config: Config,
    model_ctx: Option<Arc<ModelContext>>,
    cancel: CancellationToken,
) -> Result<()> {
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .context("WebSocket handshake failed")?;
    let (mut writer, mut reader) = ws_stream.split();

    let mut hello = json!({
        "type": "hello",
        "agent": "rustyclaw",
        "settings_dir": config.settings_dir,
    });
    if let Some(ref ctx) = model_ctx {
        hello["provider"] = serde_json::Value::String(ctx.provider.clone());
        hello["model"] = serde_json::Value::String(ctx.model.clone());
    }
    writer
        .send(Message::Text(hello.to_string().into()))
        .await
        .context("Failed to send hello message")?;

    let http = reqwest::Client::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = writer.send(Message::Close(None)).await;
                break;
            }
            msg = reader.next() => {
                let message = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                };
                match message {
                    Message::Text(text) => {
                        let response = handle_text_message(&http, text.as_str(), model_ctx.as_deref()).await;
                        writer
                            .send(Message::Text(response.into()))
                            .await
                            .context("Failed to send response")?;
                    }
                    Message::Binary(_) => {
                        let response = json!({
                            "type": "error",
                            "ok": false,
                            "message": "Binary frames are not supported",
                        });
                        writer
                            .send(Message::Text(response.to_string().into()))
                            .await
                            .context("Failed to send error response")?;
                    }
                    Message::Close(_) => {
                        break;
                    }
                    Message::Ping(payload) => {
                        writer.send(Message::Pong(payload)).await?;
                    }
                    Message::Pong(_) => {}
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Route an incoming text frame to the appropriate handler.
async fn handle_text_message(
    http: &reqwest::Client,
    text: &str,
    model_ctx: Option<&ModelContext>,
) -> String {
    // Try to parse as a structured JSON request.
    if let Ok(req) = serde_json::from_str::<ChatRequest>(text) {
        if req.msg_type == "chat" {
            return handle_chat_request(http, req, model_ctx).await;
        }
    }

    // Fall back to echo for unrecognised messages.
    json!({
        "type": "response",
        "ok": true,
        "received": text,
    })
    .to_string()
}

/// Call the model provider and return the assistant's reply.
async fn handle_chat_request(
    http: &reqwest::Client,
    req: ChatRequest,
    model_ctx: Option<&ModelContext>,
) -> String {
    let resolved = match resolve_request(req, model_ctx) {
        Ok(r) => r,
        Err(msg) => {
            return json!({
                "type": "error",
                "ok": false,
                "message": msg,
            })
            .to_string()
        }
    };

    let result = if resolved.provider == "anthropic" {
        call_anthropic(http, &resolved).await
    } else if resolved.provider == "google" {
        call_google(http, &resolved).await
    } else {
        // OpenAI-compatible (openai, xai, openrouter, ollama, github-copilot,
        // copilot-proxy, custom, …)
        call_openai_compatible(http, &resolved).await
    };

    match result {
        Ok(reply) => json!({
            "type": "response",
            "ok": true,
            "received": reply,
        })
        .to_string(),
        Err(err) => json!({
            "type": "error",
            "ok": false,
            "message": err.to_string(),
        })
        .to_string(),
    }
}

// ── Provider-specific callers ───────────────────────────────────────────────

/// Call an OpenAI-compatible `/chat/completions` endpoint.
async fn call_openai_compatible(http: &reqwest::Client, req: &ProviderRequest) -> Result<String> {
    let url = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));
    let body = json!({
        "model": req.model,
        "messages": req.messages,
    });

    let mut builder = http.post(&url).json(&body);
    if let Some(ref key) = req.api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder
        .send()
        .await
        .context("HTTP request to model provider failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Provider returned {} — {}", status, text);
    }

    let data: serde_json::Value = resp.json().await.context("Invalid JSON from provider")?;

    data["choices"][0]["message"]["content"]
        .as_str()
        .map(String::from)
        .context("No content in provider response")
}

/// Call the Anthropic Messages API (`/v1/messages`).
async fn call_anthropic(http: &reqwest::Client, req: &ProviderRequest) -> Result<String> {
    let url = format!("{}/v1/messages", req.base_url.trim_end_matches('/'));

    // Anthropic separates the system prompt from user/assistant messages.
    let system = req
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();

    let mut body = json!({
        "model": req.model,
        "max_tokens": 4096,
        "messages": messages,
    });
    if !system.is_empty() {
        body["system"] = serde_json::Value::String(system);
    }

    let api_key = req.api_key.as_deref().unwrap_or("");
    let resp = http
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .context("HTTP request to Anthropic failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic returned {} — {}", status, text);
    }

    let data: serde_json::Value = resp.json().await.context("Invalid JSON from Anthropic")?;

    // Anthropic response: {"content":[{"type":"text","text":"..."}], ...}
    data["content"][0]["text"]
        .as_str()
        .map(String::from)
        .context("No text content in Anthropic response")
}

/// Call the Google Gemini `generateContent` endpoint.
async fn call_google(http: &reqwest::Client, req: &ProviderRequest) -> Result<String> {
    let api_key = req.api_key.as_deref().unwrap_or("");
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        req.base_url.trim_end_matches('/'),
        req.model,
        api_key,
    );

    // Gemini uses a different message format: system_instruction + contents.
    let system = req
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let role = if m.role == "assistant" { "model" } else { "user" };
            json!({"role": role, "parts": [{"text": m.content}]})
        })
        .collect();

    let mut body = json!({ "contents": contents });
    if !system.is_empty() {
        body["system_instruction"] = json!({"parts": [{"text": system}]});
    }

    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("HTTP request to Google failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Google returned {} — {}", status, text);
    }

    let data: serde_json::Value = resp.json().await.context("Invalid JSON from Google")?;

    data["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .map(String::from)
        .context("No text content in Google response")
}

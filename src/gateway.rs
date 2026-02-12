use crate::config::Config;
use crate::providers;
use crate::secrets::SecretsManager;
use crate::tools;
use anyhow::{Context, Result};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Type alias for the server-side WebSocket write half.
type WsWriter = SplitSink<WebSocketStream<tokio::net::TcpStream>, Message>;

#[derive(Debug, Clone)]
pub struct GatewayOptions {
    pub listen: String,
}

// ── TOTP rate limiter ───────────────────────────────────────────────────────

/// Maximum consecutive TOTP failures before lockout.
const MAX_TOTP_FAILURES: u32 = 3;
/// Duration of the lockout after exceeding the failure limit.
const TOTP_LOCKOUT_SECS: u64 = 30;
/// Window within which failures are counted (resets after this).
const TOTP_FAILURE_WINDOW_SECS: u64 = 60;

/// Per-IP TOTP failure tracking.
#[derive(Debug, Clone)]
struct TotpAttempt {
    failures: u32,
    first_failure: Instant,
    lockout_until: Option<Instant>,
}

/// Thread-safe rate limiter shared across all connections.
type RateLimiter = Arc<Mutex<HashMap<IpAddr, TotpAttempt>>>;

fn new_rate_limiter() -> RateLimiter {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Check whether an IP is currently locked out. Returns the number of
/// seconds remaining if locked, or `None` if the IP may attempt auth.
async fn check_rate_limit(limiter: &RateLimiter, ip: IpAddr) -> Option<u64> {
    let mut map = limiter.lock().await;
    if let Some(attempt) = map.get_mut(&ip) {
        // Expire old failure windows.
        if attempt.first_failure.elapsed().as_secs() > TOTP_FAILURE_WINDOW_SECS {
            *attempt = TotpAttempt {
                failures: 0,
                first_failure: Instant::now(),
                lockout_until: None,
            };
            return None;
        }
        // Check active lockout.
        if let Some(until) = attempt.lockout_until {
            if Instant::now() < until {
                let remaining = (until - Instant::now()).as_secs() + 1;
                return Some(remaining);
            }
            // Lockout expired — reset.
            *attempt = TotpAttempt {
                failures: 0,
                first_failure: Instant::now(),
                lockout_until: None,
            };
        }
    }
    None
}

/// Record a failed TOTP attempt. Returns `true` if the IP is now locked out.
async fn record_totp_failure(limiter: &RateLimiter, ip: IpAddr) -> bool {
    let mut map = limiter.lock().await;
    let attempt = map.entry(ip).or_insert_with(|| TotpAttempt {
        failures: 0,
        first_failure: Instant::now(),
        lockout_until: None,
    });

    // Reset if the window has expired.
    if attempt.first_failure.elapsed().as_secs() > TOTP_FAILURE_WINDOW_SECS {
        attempt.failures = 0;
        attempt.first_failure = Instant::now();
        attempt.lockout_until = None;
    }

    attempt.failures += 1;
    if attempt.failures >= MAX_TOTP_FAILURES {
        attempt.lockout_until = Some(Instant::now() + std::time::Duration::from_secs(TOTP_LOCKOUT_SECS));
        true
    } else {
        false
    }
}

/// Clear failure tracking for an IP after a successful auth.
async fn clear_rate_limit(limiter: &RateLimiter, ip: IpAddr) {
    let mut map = limiter.lock().await;
    map.remove(&ip);
}

// ── Vault state ─────────────────────────────────────────────────────────────

/// Gateway-owned secrets vault, shared across connections.
///
/// The vault may start in a locked state (no password provided yet) and
/// be unlocked later via a control message from an authenticated client.
pub type SharedVault = Arc<Mutex<SecretsManager>>;

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
pub struct CopilotSession {
    oauth_token: String,
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
            oauth_token,
            inner: tokio::sync::Mutex::new(None),
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

        // Exchange the OAuth token for a fresh session token.
        let session = providers::exchange_copilot_session(http, &self.oauth_token)
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

/// Resolve the effective bearer token for an API call.
///
/// For Copilot providers the raw API key is an OAuth token that must be
/// exchanged for a short-lived session token.  For all other providers
/// the raw key is returned as-is.
async fn resolve_bearer_token(
    http: &reqwest::Client,
    provider: &str,
    raw_key: Option<&str>,
    session: Option<&CopilotSession>,
) -> Result<Option<String>> {
    if providers::needs_copilot_session(provider) {
        if let Some(session) = session {
            return Ok(Some(session.get_token(http).await?));
        }
    }
    Ok(raw_key.map(String::from))
}

// ── Status reporting ─────────────────────────────────────────────────────────

/// Build a JSON status frame to push to connected clients.
///
/// Status frames use `{ "type": "status", "status": "…", "detail": "…" }`.
/// The TUI uses these to update the gateway badge and display progress.
fn status_frame(status: &str, detail: &str) -> String {
    json!({
        "type": "status",
        "status": status,
        "detail": detail,
    })
    .to_string()
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

/// Validate the model connection by probing the provider.
///
/// The probe strategy differs by provider:
/// - **OpenAI-compatible**: `GET /models` — an auth-only check that does
///   not send a chat request, avoiding model-format mismatches.
/// - **Anthropic**: `POST /v1/messages` with `max_tokens: 1`.
/// - **Google Gemini**: `GET /models/{model}` metadata endpoint.
///
/// For Copilot providers the optional [`CopilotSession`] is used to
/// exchange the OAuth token for a session token before probing.
///
/// Returns a [`ProbeResult`] that lets the caller distinguish between
/// "fully ready", "connected with a warning", and "hard failure".
pub async fn validate_model_connection(
    http: &reqwest::Client,
    ctx: &ModelContext,
    copilot_session: Option<&CopilotSession>,
) -> ProbeResult {
    // Resolve the bearer token (session token for Copilot, raw key otherwise).
    let effective_key = match resolve_bearer_token(
        http,
        &ctx.provider,
        ctx.api_key.as_deref(),
        copilot_session,
    )
    .await
    {
        Ok(k) => k,
        Err(err) => {
            return ProbeResult::AuthError {
                detail: format!("Token exchange failed: {}", err),
            };
        }
    };

    let result: Result<reqwest::Response> = if ctx.provider == "anthropic" {
        // Anthropic has no /models list endpoint — use a minimal chat.
        let url = format!("{}/v1/messages", ctx.base_url.trim_end_matches('/'));
        let body = json!({
            "model": ctx.model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "Hi"}],
        });
        http.post(&url)
            .header("x-api-key", ctx.api_key.as_deref().unwrap_or(""))
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .context("Probe request to Anthropic failed")
    } else if ctx.provider == "google" {
        // Google: check the model metadata endpoint (no chat needed).
        let key = ctx.api_key.as_deref().unwrap_or("");
        let url = format!(
            "{}/models/{}?key={}",
            ctx.base_url.trim_end_matches('/'),
            ctx.model,
            key,
        );
        http.get(&url)
            .send()
            .await
            .context("Probe request to Google failed")
    } else {
        // OpenAI-compatible: GET /models — lightweight auth check.
        let url = format!("{}/models", ctx.base_url.trim_end_matches('/'));
        let mut builder = http.get(&url);
        if let Some(ref key) = effective_key {
            builder = builder.bearer_auth(key);
        }
        builder = apply_copilot_headers(builder, &ctx.provider);
        builder
            .send()
            .await
            .context("Probe request to provider failed")
    };

    match result {
        Ok(resp) if resp.status().is_success() => ProbeResult::Ready,
        Ok(resp) => {
            let status = resp.status();
            let code = status.as_u16();
            let body = resp.text().await.unwrap_or_default();

            // Try to extract a human-readable error message from JSON.
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message").or(Some(e)))
                        .and_then(|m| m.as_str().map(String::from))
                })
                .unwrap_or(body);

            match code {
                401 | 403 => ProbeResult::AuthError {
                    detail: format!("{} — {}", status, detail),
                },
                // 400, 404, 422 etc — the server answered, auth is fine,
                // but something about the request/model wasn't accepted.
                // Chat may still work with the full request format.
                400..=499 => ProbeResult::Connected {
                    warning: format!("{} — {}", status, detail),
                },
                _ => ProbeResult::Unreachable {
                    detail: format!("{} — {}", status, detail),
                },
            }
        }
        Err(err) => ProbeResult::Unreachable {
            detail: err.to_string(),
        },
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
/// The gateway owns the secrets vault (`vault`) — it uses the vault to
/// verify TOTP codes during the WebSocket authentication handshake and
/// to resolve model credentials.  The vault may be in a locked state
/// (password not yet provided); authenticated clients can unlock it via
/// a control message.
///
/// When `model_ctx` is provided the gateway owns the provider credentials
/// and every chat request is resolved against that context.  If `None`,
/// clients must send full `ChatRequest` payloads including provider info.
pub async fn run_gateway(
    config: Config,
    options: GatewayOptions,
    model_ctx: Option<ModelContext>,
    vault: SharedVault,
    cancel: CancellationToken,
) -> Result<()> {
    let addr = resolve_listen_addr(&options.listen)?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind gateway to {}", addr))?;

    // If the provider uses Copilot session tokens, wrap the OAuth token in
    // a CopilotSession so all connections share the same cached session.
    let copilot_session: Option<Arc<CopilotSession>> = model_ctx
        .as_ref()
        .filter(|ctx| providers::needs_copilot_session(&ctx.provider))
        .and_then(|ctx| ctx.api_key.clone())
        .map(|oauth| Arc::new(CopilotSession::new(oauth)));

    let model_ctx = model_ctx.map(Arc::new);
    let rate_limiter = new_rate_limiter();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let config_clone = config.clone();
                let ctx_clone = model_ctx.clone();
                let session_clone = copilot_session.clone();
                let vault_clone = vault.clone();
                let limiter_clone = rate_limiter.clone();
                let child_cancel = cancel.child_token();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(
                        stream, peer, config_clone, ctx_clone,
                        session_clone, vault_clone, limiter_clone, child_cancel,
                    ).await {
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
    peer: SocketAddr,
    config: Config,
    model_ctx: Option<Arc<ModelContext>>,
    copilot_session: Option<Arc<CopilotSession>>,
    vault: SharedVault,
    rate_limiter: RateLimiter,
    cancel: CancellationToken,
) -> Result<()> {
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .context("WebSocket handshake failed")?;
    let (mut writer, mut reader) = ws_stream.split();
    let peer_ip = peer.ip();

    // ── TOTP authentication challenge ───────────────────────────────
    //
    // If TOTP 2FA is enabled, we require the client to prove identity
    // before granting access to the gateway's capabilities.
    if config.totp_enabled {
        // Check rate limit first.
        if let Some(remaining) = check_rate_limit(&rate_limiter, peer_ip).await {
            let frame = json!({
                "type": "auth_locked",
                "message": format!("Too many failed attempts. Try again in {}s.", remaining),
                "retry_after": remaining,
            });
            writer.send(Message::Text(frame.to_string().into())).await?;
            writer.send(Message::Close(None)).await?;
            return Ok(());
        }

        // Send challenge.
        let challenge = json!({ "type": "auth_challenge", "method": "totp" });
        writer.send(Message::Text(challenge.to_string().into())).await
            .context("Failed to send auth_challenge")?;

        // Wait for auth_response (with a timeout).
        let auth_result = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            wait_for_auth_response(&mut reader),
        )
        .await;

        match auth_result {
            Ok(Ok(code)) => {
                let valid = {
                    let mut v = vault.lock().await;
                    v.verify_totp(code.trim()).unwrap_or(false)
                };
                if valid {
                    clear_rate_limit(&rate_limiter, peer_ip).await;
                    let ok = json!({ "type": "auth_result", "ok": true });
                    writer.send(Message::Text(ok.to_string().into())).await?;
                } else {
                    let locked_out = record_totp_failure(&rate_limiter, peer_ip).await;
                    let msg = if locked_out {
                        format!(
                            "Invalid code. Too many failures — locked out for {}s.",
                            TOTP_LOCKOUT_SECS,
                        )
                    } else {
                        "Invalid 2FA code.".to_string()
                    };
                    let fail = json!({ "type": "auth_result", "ok": false, "message": msg });
                    writer.send(Message::Text(fail.to_string().into())).await?;
                    writer.send(Message::Close(None)).await?;
                    return Ok(());
                }
            }
            Ok(Err(e)) => {
                eprintln!("Auth error from {}: {}", peer, e);
                return Ok(());
            }
            Err(_) => {
                let timeout = json!({
                    "type": "auth_result",
                    "ok": false,
                    "message": "Authentication timed out.",
                });
                let _ = writer.send(Message::Text(timeout.to_string().into())).await;
                let _ = writer.send(Message::Close(None)).await;
                return Ok(());
            }
        }
    }

    // ── Check vault status ──────────────────────────────────────────
    let vault_is_locked = {
        let v = vault.lock().await;
        v.is_locked()
    };

    // ── Send hello ──────────────────────────────────────────────────
    let mut hello = json!({
        "type": "hello",
        "agent": "rustyclaw",
        "settings_dir": config.settings_dir,
        "vault_locked": vault_is_locked,
    });
    if let Some(ref ctx) = model_ctx {
        hello["provider"] = serde_json::Value::String(ctx.provider.clone());
        hello["model"] = serde_json::Value::String(ctx.model.clone());
    }
    writer
        .send(Message::Text(hello.to_string().into()))
        .await
        .context("Failed to send hello message")?;

    if vault_is_locked {
        writer
            .send(Message::Text(
                status_frame("vault_locked", "Secrets vault is locked — provide password to unlock")
                    .into(),
            ))
            .await
            .context("Failed to send vault_locked status")?;
    }

    // ── Report model status to the freshly-connected client ────────
    let http = reqwest::Client::new();

    match model_ctx {
        Some(ref ctx) => {
            let display = providers::display_name_for_provider(&ctx.provider);

            // 1. Model configured
            let detail = format!("{} / {}", display, ctx.model);
            writer
                .send(Message::Text(
                    status_frame("model_configured", &detail).into(),
                ))
                .await
                .context("Failed to send model_configured status")?;

            // 2. Credentials
            if ctx.api_key.is_some() {
                writer
                    .send(Message::Text(
                        status_frame("credentials_loaded", &format!("{} API key loaded", display))
                            .into(),
                    ))
                    .await
                    .context("Failed to send credentials_loaded status")?;
            } else if providers::secret_key_for_provider(&ctx.provider).is_some() {
                writer
                    .send(Message::Text(
                        status_frame(
                            "credentials_missing",
                            &format!("No API key for {} — model calls will fail", display),
                        )
                        .into(),
                    ))
                    .await
                    .context("Failed to send credentials_missing status")?;
            }

            // 3. Validate the connection with a lightweight probe
            //
            // For Copilot providers, exchange the OAuth token for a session
            // token first — the probe must use the session token too.
            writer
                .send(Message::Text(
                    status_frame("model_connecting", &format!("Probing {} …", ctx.base_url))
                        .into(),
                ))
                .await
                .context("Failed to send model_connecting status")?;

            match validate_model_connection(&http, ctx, copilot_session.as_deref()).await {
                ProbeResult::Ready => {
                    writer
                        .send(Message::Text(
                            status_frame(
                                "model_ready",
                                &format!("{} / {} ready", display, ctx.model),
                            )
                            .into(),
                        ))
                        .await
                        .context("Failed to send model_ready status")?;
                }
                ProbeResult::Connected { warning } => {
                    // Auth is fine, provider is reachable — the specific
                    // probe request wasn't accepted, but chat will likely
                    // work with the real request format.
                    writer
                        .send(Message::Text(
                            status_frame(
                                "model_ready",
                                &format!("{} / {} connected (probe: {})", display, ctx.model, warning),
                            )
                            .into(),
                        ))
                        .await
                        .context("Failed to send model_ready status")?;
                }
                ProbeResult::AuthError { detail } => {
                    writer
                        .send(Message::Text(
                            status_frame(
                                "model_error",
                                &format!("{} auth failed: {}", display, detail),
                            )
                            .into(),
                        ))
                        .await
                        .context("Failed to send model_error status")?;
                }
                ProbeResult::Unreachable { detail } => {
                    writer
                        .send(Message::Text(
                            status_frame(
                                "model_error",
                                &format!("{} probe failed: {}", display, detail),
                            )
                            .into(),
                        ))
                        .await
                        .context("Failed to send model_error status")?;
                }
            }
        }
        None => {
            writer
                .send(Message::Text(
                    status_frame(
                        "no_model",
                        "No model configured — clients must send full credentials",
                    )
                    .into(),
                ))
                .await
                .context("Failed to send no_model status")?;
        }
    }

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
                        // ── Handle unlock_vault control message ─────
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                            if val.get("type").and_then(|t| t.as_str()) == Some("unlock_vault") {
                                if let Some(pw) = val.get("password").and_then(|p| p.as_str()) {
                                    let mut v = vault.lock().await;
                                    v.set_password(pw.to_string());
                                    // Try to access the vault to verify the password works.
                                    // get_secret returns Err if the vault cannot be decrypted.
                                    match v.get_secret("__vault_check__", true) {
                                        Ok(_) => {
                                            let ok = json!({
                                                "type": "vault_unlocked",
                                                "ok": true,
                                            });
                                            let _ = writer.send(Message::Text(ok.to_string().into())).await;
                                        }
                                        Err(e) => {
                                            // Revert to locked state.
                                            v.clear_password();
                                            let fail = json!({
                                                "type": "vault_unlocked",
                                                "ok": false,
                                                "message": format!("Failed to unlock vault: {}", e),
                                            });
                                            let _ = writer.send(Message::Text(fail.to_string().into())).await;
                                        }
                                    }
                                }
                                continue;
                            }
                        }

                        if let Err(err) = dispatch_text_message(
                            &http,
                            text.as_str(),
                            model_ctx.as_deref(),
                            copilot_session.as_deref(),
                            &mut writer,
                        )
                        .await
                        {
                            let frame = json!({
                                "type": "error",
                                "ok": false,
                                "message": err.to_string(),
                            });
                            let _ = writer
                                .send(Message::Text(frame.to_string().into()))
                                .await;
                        }
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

/// Wait for an `auth_response` frame from the client.
///
/// Reads WebSocket messages until we get a JSON frame with
/// `{"type": "auth_response", "code": "..."}` or the connection drops.
async fn wait_for_auth_response(
    reader: &mut futures_util::stream::SplitStream<WebSocketStream<tokio::net::TcpStream>>,
) -> Result<String> {
    while let Some(msg) = reader.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                    if val.get("type").and_then(|t| t.as_str()) == Some("auth_response") {
                        if let Some(code) = val.get("code").and_then(|c| c.as_str()) {
                            return Ok(code.to_string());
                        }
                        anyhow::bail!("auth_response missing 'code' field");
                    }
                }
                // Ignore non-auth frames during the handshake.
            }
            Ok(Message::Close(_)) => {
                anyhow::bail!("Client disconnected during authentication");
            }
            Err(e) => {
                anyhow::bail!("WebSocket error during authentication: {}", e);
            }
            _ => {} // Ignore ping/pong/binary during auth
        }
    }
    anyhow::bail!("Connection closed before authentication completed")
}

/// Route an incoming text frame to the appropriate handler.
///
/// Implements an agentic tool loop: the model is called, and if it
/// requests tool calls, the gateway executes them locally and feeds
/// the results back into the conversation, repeating until the model
/// produces a final text response (or a safety limit is hit).
async fn dispatch_text_message(
    http: &reqwest::Client,
    text: &str,
    model_ctx: Option<&ModelContext>,
    copilot_session: Option<&CopilotSession>,
    writer: &mut WsWriter,
) -> Result<()> {
    // Try to parse as a structured JSON request.
    let req = match serde_json::from_str::<ChatRequest>(text) {
        Ok(r) if r.msg_type == "chat" => r,
        Ok(r) => {
            let frame = json!({
                "type": "error",
                "ok": false,
                "message": format!("Unknown message type: {:?}", r.msg_type),
            });
            writer
                .send(Message::Text(frame.to_string().into()))
                .await
                .context("Failed to send error frame")?;
            return Ok(());
        }
        Err(err) => {
            let frame = json!({
                "type": "error",
                "ok": false,
                "message": format!("Invalid JSON: {}", err),
            });
            writer
                .send(Message::Text(frame.to_string().into()))
                .await
                .context("Failed to send error frame")?;
            return Ok(());
        }
    };

    let mut resolved = match resolve_request(req, model_ctx) {
        Ok(r) => r,
        Err(msg) => {
            let frame = json!({ "type": "error", "ok": false, "message": msg });
            writer
                .send(Message::Text(frame.to_string().into()))
                .await
                .context("Failed to send error frame")?;
            return Ok(());
        }
    };

    // For Copilot providers, swap the raw OAuth token for a session token.
    match resolve_bearer_token(
        http,
        &resolved.provider,
        resolved.api_key.as_deref(),
        copilot_session,
    )
    .await
    {
        Ok(token) => resolved.api_key = token,
        Err(err) => {
            let frame = json!({
                "type": "error",
                "ok": false,
                "message": format!("Token exchange failed: {}", err),
            });
            writer
                .send(Message::Text(frame.to_string().into()))
                .await
                .context("Failed to send error frame")?;
            return Ok(());
        }
    }

    // ── Agentic tool loop ───────────────────────────────────────────
    const MAX_TOOL_ROUNDS: usize = 25;

    for _round in 0..MAX_TOOL_ROUNDS {
        let result = if resolved.provider == "anthropic" {
            call_anthropic_with_tools(http, &resolved).await
        } else if resolved.provider == "google" {
            call_google_with_tools(http, &resolved).await
        } else {
            call_openai_with_tools(http, &resolved).await
        };

        let model_resp = match result {
            Ok(r) => r,
            Err(err) => {
                let frame = json!({
                    "type": "error",
                    "ok": false,
                    "message": err.to_string(),
                });
                writer
                    .send(Message::Text(frame.to_string().into()))
                    .await
                    .context("Failed to send error frame")?;
                return Ok(());
            }
        };

        // Stream any text content to the client.
        if !model_resp.text.is_empty() {
            send_chunk(writer, &model_resp.text).await?;
        }

        if model_resp.tool_calls.is_empty() {
            // No tool calls — the model is done.
            send_response_done(writer).await?;
            return Ok(());
        }

        // ── Execute each requested tool ─────────────────────────────
        let mut tool_results: Vec<ToolCallResult> = Vec::new();

        for tc in &model_resp.tool_calls {
            // Notify the client about the tool call.
            let call_frame = json!({
                "type": "tool_call",
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            });
            writer
                .send(Message::Text(call_frame.to_string().into()))
                .await
                .context("Failed to send tool_call frame")?;

            // Execute the tool.
            let (output, is_error) = match tools::execute_tool(&tc.name, &tc.arguments) {
                Ok(text) => (text, false),
                Err(err) => (err, true),
            };

            // Notify the client about the result.
            let result_frame = json!({
                "type": "tool_result",
                "id": tc.id,
                "name": tc.name,
                "result": output,
                "is_error": is_error,
            });
            writer
                .send(Message::Text(result_frame.to_string().into()))
                .await
                .context("Failed to send tool_result frame")?;

            tool_results.push(ToolCallResult {
                id: tc.id.clone(),
                name: tc.name.clone(),
                output,
                is_error,
            });
        }

        // ── Append assistant + tool-result messages to conversation ──
        // The model's response (possibly with text + tool calls) becomes
        // an assistant message, and each tool result becomes a tool message.
        append_tool_round(
            &resolved.provider,
            &mut resolved.messages,
            &model_resp,
            &tool_results,
        );
    }

    // If we exhausted all rounds, send what we have and stop.
    let frame = json!({
        "type": "error",
        "ok": false,
        "message": "Tool loop limit reached — stopping.",
    });
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send error frame")?;
    send_response_done(writer).await?;
    Ok(())
}

// ── Streaming helpers ───────────────────────────────────────────────────────

/// Send a single `{"type": "chunk", "delta": "..."}` frame.
async fn send_chunk(writer: &mut WsWriter, delta: &str) -> Result<()> {
    let frame = json!({ "type": "chunk", "delta": delta });
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send chunk frame")
}

/// Send the `{"type": "response_done"}` sentinel frame.
async fn send_response_done(writer: &mut WsWriter) -> Result<()> {
    let frame = json!({ "type": "response_done", "ok": true });
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send response_done frame")
}

// ── Model response types (shared across providers) ──────────────────────────

/// A parsed tool call from the model.
#[derive(Debug, Clone)]
struct ParsedToolCall {
    id: String,
    name: String,
    arguments: serde_json::Value,
}

/// The result of executing a tool locally.
#[derive(Debug, Clone)]
struct ToolCallResult {
    id: String,
    name: String,
    output: String,
    is_error: bool,
}

/// A complete model response: optional text + optional tool calls.
#[derive(Debug, Default)]
struct ModelResponse {
    text: String,
    tool_calls: Vec<ParsedToolCall>,
}

/// Append the model's assistant turn and tool results to the conversation
/// so the next round has full context.
fn append_tool_round(
    provider: &str,
    messages: &mut Vec<ChatMessage>,
    model_resp: &ModelResponse,
    results: &[ToolCallResult],
) {
    if provider == "anthropic" {
        // Anthropic: assistant message has content blocks (text + tool_use),
        // then one "user" message with tool_result blocks.
        let mut content_blocks = Vec::new();
        if !model_resp.text.is_empty() {
            content_blocks.push(json!({ "type": "text", "text": model_resp.text }));
        }
        for tc in &model_resp.tool_calls {
            content_blocks.push(json!({
                "type": "tool_use",
                "id": tc.id,
                "name": tc.name,
                "input": tc.arguments,
            }));
        }
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: serde_json::to_string(&content_blocks).unwrap_or_default(),
        });

        let mut result_blocks = Vec::new();
        for r in results {
            result_blocks.push(json!({
                "type": "tool_result",
                "tool_use_id": r.id,
                "content": r.output,
                "is_error": r.is_error,
            }));
        }
        messages.push(ChatMessage {
            role: "user".into(),
            content: serde_json::to_string(&result_blocks).unwrap_or_default(),
        });
    } else if provider == "google" {
        // Google: model turn with function calls, then user turn with function responses.
        let mut parts = Vec::new();
        if !model_resp.text.is_empty() {
            parts.push(json!({ "text": model_resp.text }));
        }
        for tc in &model_resp.tool_calls {
            parts.push(json!({
                "functionCall": { "name": tc.name, "args": tc.arguments }
            }));
        }
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: serde_json::to_string(&parts).unwrap_or_default(),
        });

        let mut resp_parts = Vec::new();
        for r in results {
            resp_parts.push(json!({
                "functionResponse": {
                    "name": r.name,
                    "response": { "content": r.output, "is_error": r.is_error }
                }
            }));
        }
        messages.push(ChatMessage {
            role: "user".into(),
            content: serde_json::to_string(&resp_parts).unwrap_or_default(),
        });
    } else {
        // OpenAI-compatible: assistant message with tool_calls array,
        // then one "tool" message per result.
        let tc_array: Vec<serde_json::Value> = model_resp
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    }
                })
            })
            .collect();

        // The assistant message carries both text and tool_calls.
        let assistant_json = json!({
            "role": "assistant",
            "content": if model_resp.text.is_empty() { serde_json::Value::Null } else { json!(model_resp.text) },
            "tool_calls": tc_array,
        });
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: serde_json::to_string(&assistant_json).unwrap_or_default(),
        });

        for r in results {
            messages.push(ChatMessage {
                role: "tool".into(),
                content: json!({
                    "role": "tool",
                    "tool_call_id": r.id,
                    "content": r.output,
                })
                .to_string(),
            });
        }
    }
}

// ── Provider-specific callers ───────────────────────────────────────────────

/// Attach GitHub-Copilot-required IDE headers to a request builder.
fn apply_copilot_headers(
    builder: reqwest::RequestBuilder,
    provider: &str,
) -> reqwest::RequestBuilder {
    if !providers::needs_copilot_session(provider) {
        return builder;
    }
    let version = env!("CARGO_PKG_VERSION");
    builder
        .header("Editor-Version", format!("RustyClaw/{}", version))
        .header("Editor-Plugin-Version", format!("rustyclaw/{}", version))
        .header("Copilot-Integration-Id", "rustyclaw")
        .header("openai-intent", "conversation-panel")
}

// ── OpenAI-compatible ───────────────────────────────────────────────────────

/// Call an OpenAI-compatible `/chat/completions` endpoint (non-streaming)
/// with tool definitions.  Returns structured text + tool calls.
async fn call_openai_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
) -> Result<ModelResponse> {
    let url = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));

    // Build the messages array.  Most messages are simple role+content,
    // but tool-loop continuation messages have structured JSON content
    // that must be sent as raw objects rather than string-escaped.
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            // Try to parse content as JSON first (for assistant messages
            // with tool_calls and tool-result messages).
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_object() && parsed.get("role").is_some() {
                    return parsed;
                }
            }
            json!({ "role": m.role, "content": m.content })
        })
        .collect();

    let tool_defs = tools::tools_openai();

    let mut body = json!({
        "model": req.model,
        "messages": messages,
    });
    if !tool_defs.is_empty() {
        body["tools"] = json!(tool_defs);
    }

    let mut builder = http.post(&url).json(&body);
    if let Some(ref key) = req.api_key {
        builder = builder.bearer_auth(key);
    }
    builder = apply_copilot_headers(builder, &req.provider);

    let resp = builder
        .send()
        .await
        .context("HTTP request to model provider failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Provider returned {} — {}", status, text);
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .context("Invalid JSON from provider")?;

    let choice = &data["choices"][0];
    let message = &choice["message"];

    let mut result = ModelResponse::default();

    // Extract text content.
    if let Some(text) = message["content"].as_str() {
        result.text = text.to_string();
    }

    // Extract tool calls.
    if let Some(tc_array) = message["tool_calls"].as_array() {
        for tc in tc_array {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let arguments = serde_json::from_str(args_str).unwrap_or(json!({}));
            result.tool_calls.push(ParsedToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    Ok(result)
}

// ── Anthropic ───────────────────────────────────────────────────────────────

/// Call the Anthropic Messages API with tool definitions (non-streaming).
async fn call_anthropic_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
) -> Result<ModelResponse> {
    let url = format!("{}/v1/messages", req.base_url.trim_end_matches('/'));

    let system = req
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    // Build messages.  Tool-loop continuation messages have structured
    // JSON content (content blocks) that must be sent as arrays.
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            // Try to parse content as a JSON array (content blocks).
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_array() {
                    return json!({ "role": m.role, "content": parsed });
                }
            }
            json!({ "role": m.role, "content": m.content })
        })
        .collect();

    let tool_defs = tools::tools_anthropic();

    let mut body = json!({
        "model": req.model,
        "max_tokens": 4096,
        "messages": messages,
    });
    if !system.is_empty() {
        body["system"] = serde_json::Value::String(system);
    }
    if !tool_defs.is_empty() {
        body["tools"] = json!(tool_defs);
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

    let mut result = ModelResponse::default();

    if let Some(content) = data["content"].as_array() {
        for block in content {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        if !result.text.is_empty() {
                            result.text.push('\n');
                        }
                        result.text.push_str(text);
                    }
                }
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or("").to_string();
                    let name = block["name"].as_str().unwrap_or("").to_string();
                    let arguments = block["input"].clone();
                    result.tool_calls.push(ParsedToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                _ => {}
            }
        }
    }

    Ok(result)
}

// ── Google Gemini ───────────────────────────────────────────────────────────

/// Call Google Gemini with function declarations (non-streaming).
async fn call_google_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
) -> Result<ModelResponse> {
    let api_key = req.api_key.as_deref().unwrap_or("");
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        req.base_url.trim_end_matches('/'),
        req.model,
        api_key,
    );

    let system = req
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    // Build contents.  Tool-loop continuation messages may have
    // structured JSON parts that need to be sent as arrays.
    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let role = if m.role == "assistant" { "model" } else { "user" };
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_array() {
                    return json!({ "role": role, "parts": parsed });
                }
            }
            json!({ "role": role, "parts": [{ "text": m.content }] })
        })
        .collect();

    let tool_defs = tools::tools_google();

    let mut body = json!({ "contents": contents });
    if !system.is_empty() {
        body["system_instruction"] = json!({ "parts": [{ "text": system }] });
    }
    if !tool_defs.is_empty() {
        body["tools"] = json!([{ "function_declarations": tool_defs }]);
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

    let mut result = ModelResponse::default();

    if let Some(parts) = data["candidates"][0]["content"]["parts"].as_array() {
        for (i, part) in parts.iter().enumerate() {
            if let Some(text) = part["text"].as_str() {
                if !result.text.is_empty() {
                    result.text.push('\n');
                }
                result.text.push_str(text);
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc["name"].as_str().unwrap_or("").to_string();
                let arguments = fc["args"].clone();
                result.tool_calls.push(ParsedToolCall {
                    id: format!("google_call_{}", i),
                    name,
                    arguments,
                });
            }
        }
    }

    Ok(result)
}

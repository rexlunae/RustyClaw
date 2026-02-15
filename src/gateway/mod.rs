//! Gateway module — WebSocket server for agent communication.
//!
//! This module provides the gateway server that handles WebSocket connections
//! from TUI clients, manages authentication, and dispatches chat requests to
//! model providers. It also polls configured messengers (Telegram, Discord, etc.)
//! for incoming messages and routes them through the model.

mod auth;
mod helpers;
mod messenger_handler;
mod providers;
mod secrets_handler;
mod skills_handler;
mod types;

// Re-export public types
pub use types::{
    ChatMessage, ChatRequest, CopilotSession, GatewayOptions, MediaRef, ModelContext,
    ModelResponse, ParsedToolCall, ProbeResult, ProviderRequest, ToolCallResult,
};

// Re-export messenger handler types
pub use messenger_handler::{
    create_messenger_manager, run_messenger_loop, SharedMessengerManager,
};

use crate::config::Config;
use crate::providers as crate_providers;
use crate::secrets::SecretsManager;
use crate::skills::SkillManager;
use crate::tools;
use anyhow::{Context, Result};
use dirs;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::sync::CancellationToken;

/// Shared flag for cancelling the tool loop from another task.
pub type ToolCancelFlag = Arc<AtomicBool>;

/// Type alias for the server-side WebSocket write half.
type WsWriter = SplitSink<WebSocketStream<tokio::net::TcpStream>, Message>;

/// Gateway-owned secrets vault, shared across connections.
///
/// The vault may start in a locked state (no password provided yet) and
/// be unlocked later via a control message from an authenticated client.
pub type SharedVault = Arc<Mutex<SecretsManager>>;

/// Gateway-owned skill manager, shared across connections.
pub type SharedSkillManager = Arc<Mutex<SkillManager>>;

// Re-export validate_model_connection for external use
pub use providers::validate_model_connection;

// ── Constants ───────────────────────────────────────────────────────────────

/// Duration of the lockout after exceeding the failure limit.
const TOTP_LOCKOUT_SECS: u64 = 30;

/// Compaction fires when estimated usage exceeds this fraction of the context window.
const COMPACTION_THRESHOLD: f64 = 0.75;

/// Try to import a fresh GitHub Copilot token from OpenClaw's credential store.
///
/// This allows RustyClaw to automatically refresh its session token when the
/// vault copy expires, as long as OpenClaw is running and has a valid token.
fn try_import_openclaw_token(vault: &mut SecretsManager) -> Option<CopilotSession> {
    let openclaw_dir = dirs::home_dir()?.join(".openclaw");
    let token_file = openclaw_dir.join("credentials/github-copilot.token.json");

    if !token_file.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&token_file).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let token = json.get("token")?.as_str()?;
    let expires_at_ms = json.get("expiresAt")?.as_i64()?;
    let expires_at = expires_at_ms / 1000; // Convert ms to seconds

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let remaining = expires_at - now;
    if remaining <= 60 {
        eprintln!("  ⊘ OpenClaw token also expired");
        return None;
    }

    eprintln!("  ✓ Auto-imported fresh token from OpenClaw (~{}h remaining)", remaining / 3600);

    // Store in our vault for next time
    let session_data = serde_json::json!({
        "session_token": token,
        "expires_at": expires_at,
    });
    if let Err(e) = vault.store_secret("GITHUB_COPILOT_SESSION", &session_data.to_string()) {
        eprintln!("    ⚠ Failed to cache in vault: {}", e);
    }

    Some(CopilotSession::from_session_token(token.to_string(), expires_at))
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
    skill_mgr: SharedSkillManager,
    cancel: CancellationToken,
) -> Result<()> {
    // Register the credentials directory so file-access tools can enforce
    // the vault boundary (blocks read_file, execute_command, etc.).
    tools::set_credentials_dir(config.credentials_dir());

    // Register the vault so web_fetch can access the cookie jar.
    tools::set_vault(vault.clone());

    // Initialize sandbox for command execution
    let sandbox_mode = config.sandbox.mode.parse().unwrap_or_default();
    tools::init_sandbox(
        sandbox_mode,
        config.workspace_dir(),
        config.credentials_dir(),
        config.sandbox.deny_paths.clone(),
    );

    let addr = helpers::resolve_listen_addr(&options.listen)?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind gateway to {}", addr))?;

    // If the provider uses Copilot session tokens, check for:
    // 1. Imported session token (GITHUB_COPILOT_SESSION) - use until expiry
    // 2. Fresh token from OpenClaw (~/.openclaw/credentials/github-copilot.token.json)
    // 3. OAuth token (GITHUB_COPILOT_TOKEN) - can refresh sessions
    let copilot_session: Option<Arc<CopilotSession>> = if model_ctx
        .as_ref()
        .map(|ctx| crate_providers::needs_copilot_session(&ctx.provider))
        .unwrap_or(false)
    {
        // First check for imported session token in our vault
        let mut vault_guard = vault.lock().await;
        let session_result = vault_guard.get_secret("GITHUB_COPILOT_SESSION", true);
        
        let mut session_from_import = match &session_result {
            Ok(Some(json_str)) => {
                eprintln!("  ✓ Found GITHUB_COPILOT_SESSION in vault");
                serde_json::from_str::<serde_json::Value>(json_str)
                    .ok()
                    .and_then(|json| {
                        let token = json.get("session_token")?.as_str()?.to_string();
                        let expires_at = json.get("expires_at")?.as_i64()?;
                        
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        
                        let remaining = expires_at - now;
                        eprintln!("    Session expires in {}s", remaining);
                        
                        if remaining > 60 {
                            Some(CopilotSession::from_session_token(token, expires_at))
                        } else {
                            eprintln!("    Session expired or expiring soon");
                            None
                        }
                    })
            }
            Ok(None) => {
                eprintln!("  ⊘ GITHUB_COPILOT_SESSION not found in vault");
                None
            }
            Err(e) => {
                eprintln!("  ✗ Failed to read GITHUB_COPILOT_SESSION: {}", e);
                None
            }
        };

        // If vault session is expired/missing, try to auto-import from OpenClaw
        if session_from_import.is_none() {
            if let Some(session) = try_import_openclaw_token(&mut vault_guard) {
                session_from_import = Some(session);
            }
        }
        drop(vault_guard);

        if let Some(session) = session_from_import {
            eprintln!("  ✓ Using imported session token");
            Some(Arc::new(session))
        } else {
            // Fall back to OAuth token
            if let Some(oauth) = model_ctx.as_ref().and_then(|ctx| ctx.api_key.clone()) {
                eprintln!("  → Falling back to OAuth token");
                Some(Arc::new(CopilotSession::new(oauth)))
            } else {
                eprintln!("  ✗ No OAuth token available either");
                None
            }
        }
    } else {
        None
    };

    let model_ctx = model_ctx.map(Arc::new);
    let rate_limiter = auth::new_rate_limiter();

    // ── Initialize and start messenger loop ─────────────────────────
    //
    // If messengers are configured, we poll them for incoming messages
    // and route them through the model.
    let messenger_mgr = if !config.messengers.is_empty() {
        match messenger_handler::create_messenger_manager(&config).await {
            Ok(mgr) => {
                let shared_mgr: SharedMessengerManager = Arc::new(Mutex::new(mgr));
                
                // Spawn messenger loop
                let messenger_config = config.clone();
                let messenger_ctx = model_ctx.clone();
                let messenger_vault = vault.clone();
                let messenger_skills = skill_mgr.clone();
                let messenger_cancel = cancel.child_token();
                let mgr_clone = shared_mgr.clone();
                
                tokio::spawn(async move {
                    if let Err(e) = messenger_handler::run_messenger_loop(
                        messenger_config,
                        mgr_clone,
                        messenger_ctx,
                        messenger_vault,
                        messenger_skills,
                        messenger_cancel,
                    ).await {
                        eprintln!("[gateway] Messenger loop error: {}", e);
                    }
                });
                
                Some(shared_mgr)
            }
            Err(e) => {
                eprintln!("[gateway] Failed to initialize messengers: {}", e);
                None
            }
        }
    } else {
        None
    };

    eprintln!("[gateway] Listening on {}", addr);
    if messenger_mgr.is_some() {
        eprintln!("[gateway] Messenger polling enabled");
    }

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
                let skill_clone = skill_mgr.clone();
                let limiter_clone = rate_limiter.clone();
                let child_cancel = cancel.child_token();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(
                        stream, peer, config_clone, ctx_clone,
                        session_clone, vault_clone, skill_clone,
                        limiter_clone, child_cancel,
                    ).await {
                        eprintln!("Gateway connection error from {}: {}", peer, err);
                    }
                });
            }
        }
    }

    Ok(())
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
    config: Config,
    model_ctx: Option<Arc<ModelContext>>,
    copilot_session: Option<Arc<CopilotSession>>,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    rate_limiter: auth::RateLimiter,
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
        if let Some(remaining) = auth::check_rate_limit(&rate_limiter, peer_ip).await {
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

        // Allow up to 3 attempts before closing the connection.
        const MAX_TOTP_ATTEMPTS: u8 = 3;
        let mut attempts = 0u8;

        loop {
            // Wait for auth_response (with a timeout).
            let auth_result = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                auth::wait_for_auth_response(&mut reader),
            )
            .await;

            match auth_result {
                Ok(Ok(code)) => {
                    let valid = {
                        let mut v = vault.lock().await;
                        v.verify_totp(code.trim()).unwrap_or(false)
                    };
                    if valid {
                        auth::clear_rate_limit(&rate_limiter, peer_ip).await;
                        let ok = json!({ "type": "auth_result", "ok": true });
                        writer.send(Message::Text(ok.to_string().into())).await?;
                        break; // Authentication successful, continue to main loop
                    } else {
                        attempts += 1;
                        let locked_out = auth::record_totp_failure(&rate_limiter, peer_ip).await;

                        if locked_out {
                            let msg = format!(
                                "Invalid code. Too many failures — locked out for {}s.",
                                TOTP_LOCKOUT_SECS,
                            );
                            let fail = json!({ "type": "auth_result", "ok": false, "message": msg });
                            writer.send(Message::Text(fail.to_string().into())).await?;
                            writer.send(Message::Close(None)).await?;
                            return Ok(());
                        } else if attempts >= MAX_TOTP_ATTEMPTS {
                            let msg = "Invalid code. Maximum attempts exceeded.";
                            let fail = json!({ "type": "auth_result", "ok": false, "message": msg });
                            writer.send(Message::Text(fail.to_string().into())).await?;
                            writer.send(Message::Close(None)).await?;
                            return Ok(());
                        } else {
                            let remaining = MAX_TOTP_ATTEMPTS - attempts;
                            let msg = format!(
                                "Invalid 2FA code. {} attempt{} remaining.",
                                remaining,
                                if remaining == 1 { "" } else { "s" }
                            );
                            let retry = json!({
                                "type": "auth_result",
                                "ok": false,
                                "retry": true,
                                "message": msg,
                            });
                            writer.send(Message::Text(retry.to_string().into())).await?;
                            // Continue loop to allow retry
                        }
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
                helpers::status_frame("vault_locked", "Secrets vault is locked — provide password to unlock")
                    .into(),
            ))
            .await
            .context("Failed to send vault_locked status")?;
    }

    // ── Report model status to the freshly-connected client ────────
    let http = reqwest::Client::new();

    match model_ctx {
        Some(ref ctx) => {
            let display = crate_providers::display_name_for_provider(&ctx.provider);

            // 1. Model configured
            let detail = format!("{} / {}", display, ctx.model);
            writer
                .send(Message::Text(
                    helpers::status_frame("model_configured", &detail).into(),
                ))
                .await
                .context("Failed to send model_configured status")?;

            // 2. Credentials
            if ctx.api_key.is_some() {
                writer
                    .send(Message::Text(
                        helpers::status_frame("credentials_loaded", &format!("{} API key loaded", display))
                            .into(),
                    ))
                    .await
                    .context("Failed to send credentials_loaded status")?;
            } else if crate_providers::secret_key_for_provider(&ctx.provider).is_some() {
                writer
                    .send(Message::Text(
                        helpers::status_frame(
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
                    helpers::status_frame("model_connecting", &format!("Probing {} …", ctx.base_url))
                        .into(),
                ))
                .await
                .context("Failed to send model_connecting status")?;

            match providers::validate_model_connection(&http, ctx, copilot_session.as_deref()).await {
                ProbeResult::Ready => {
                    writer
                        .send(Message::Text(
                            helpers::status_frame(
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
                            helpers::status_frame(
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
                            helpers::status_frame(
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
                            helpers::status_frame(
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
                    helpers::status_frame(
                        "no_model",
                        "No model configured — clients must send full credentials",
                    )
                    .into(),
                ))
                .await
                .context("Failed to send no_model status")?;
        }
    }

    // ── Spawn reader task with cancel flag ─────────────────────────
    //
    // The reader runs in a separate task so it can receive cancel messages
    // even while dispatch_text_message is running. Messages are forwarded
    // through a channel; cancel requests set a shared flag.
    let tool_cancel: ToolCancelFlag = Arc::new(AtomicBool::new(false));
    let (msg_tx, mut msg_rx) = tokio::sync::mpsc::channel::<Message>(32);

    let reader_cancel = cancel.clone();
    let reader_tool_cancel = tool_cancel.clone();
    let reader_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = reader_cancel.cancelled() => break,
                msg = reader.next() => {
                    match msg {
                        Some(Ok(Message::Text(ref text))) => {
                            // Check for cancel message
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                                if val.get("type").and_then(|t| t.as_str()) == Some("cancel") {
                                    reader_tool_cancel.store(true, Ordering::Relaxed);
                                    // Don't forward cancel messages, just set the flag
                                    continue;
                                }
                            }
                            // Forward other messages
                            if msg_tx.send(Message::Text(text.clone())).await.is_err() {
                                break; // Channel closed
                            }
                        }
                        Some(Ok(msg)) => {
                            if msg_tx.send(msg).await.is_err() {
                                break;
                            }
                        }
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
            }
        }
    });

    // Main message handling loop — receives from channel
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = writer.send(Message::Close(None)).await;
                break;
            }
            msg = msg_rx.recv() => {
                let message = match msg {
                    Some(m) => m,
                    None => break, // Channel closed (reader exited)
                };
                match message {
                    Message::Text(text) => {
                        // Reset cancel flag for new request
                        tool_cancel.store(false, Ordering::Relaxed);

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

                            // ── Handle secrets control messages ──────
                            let msg_type = val.get("type").and_then(|t| t.as_str());

                            match msg_type {
                                Some("secrets_list") => {
                                    let mut v = vault.lock().await;
                                    let entries = v.list_all_entries();
                                    let json_entries: Vec<serde_json::Value> = entries.iter().map(|(name, entry)| {
                                        let mut obj = serde_json::to_value(entry).unwrap_or_default();
                                        if let Some(map) = obj.as_object_mut() {
                                            map.insert("name".to_string(), json!(name));
                                        }
                                        obj
                                    }).collect();
                                    let resp = json!({
                                        "type": "secrets_list_result",
                                        "ok": true,
                                        "entries": json_entries,
                                    });
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_store") => {
                                    let key = val.get("key").and_then(|k| k.as_str()).unwrap_or("");
                                    let value = val.get("value").and_then(|v| v.as_str()).unwrap_or("");
                                    let mut v = vault.lock().await;
                                    let resp = match v.store_secret(key, value) {
                                        Ok(()) => json!({
                                            "type": "secrets_store_result",
                                            "ok": true,
                                            "message": format!("Secret '{}' stored.", key),
                                        }),
                                        Err(e) => json!({
                                            "type": "secrets_store_result",
                                            "ok": false,
                                            "message": format!("Failed to store secret: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_get") => {
                                    let key = val.get("key").and_then(|k| k.as_str()).unwrap_or("");
                                    let mut v = vault.lock().await;
                                    let resp = match v.get_secret(key, true) {
                                        Ok(Some(value)) => json!({
                                            "type": "secrets_get_result",
                                            "ok": true,
                                            "key": key,
                                            "value": value,
                                        }),
                                        Ok(None) => json!({
                                            "type": "secrets_get_result",
                                            "ok": false,
                                            "key": key,
                                            "message": format!("Secret '{}' not found.", key),
                                        }),
                                        Err(e) => json!({
                                            "type": "secrets_get_result",
                                            "ok": false,
                                            "key": key,
                                            "message": format!("Failed to get secret: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_delete") => {
                                    let key = val.get("key").and_then(|k| k.as_str()).unwrap_or("");
                                    let mut v = vault.lock().await;
                                    let resp = match v.delete_secret(key) {
                                        Ok(()) => json!({
                                            "type": "secrets_delete_result",
                                            "ok": true,
                                        }),
                                        Err(e) => json!({
                                            "type": "secrets_delete_result",
                                            "ok": false,
                                            "message": format!("Failed to delete: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_peek") => {
                                    let name = val.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                    let mut v = vault.lock().await;
                                    let resp = match v.peek_credential_display(name) {
                                        Ok(fields) => {
                                            let field_arrays: Vec<serde_json::Value> = fields.iter()
                                                .map(|(label, value)| json!([label, value]))
                                                .collect();
                                            json!({
                                                "type": "secrets_peek_result",
                                                "ok": true,
                                                "fields": field_arrays,
                                            })
                                        }
                                        Err(e) => json!({
                                            "type": "secrets_peek_result",
                                            "ok": false,
                                            "message": format!("Failed to peek: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_set_policy") => {
                                    let name = val.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                    let policy_str = val.get("policy").and_then(|p| p.as_str()).unwrap_or("");
                                    let skills_list = val.get("skills").and_then(|s| s.as_array())
                                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>())
                                        .unwrap_or_default();
                                    let policy = match policy_str {
                                        "always" => Some(crate::secrets::AccessPolicy::Always),
                                        "ask" => Some(crate::secrets::AccessPolicy::WithApproval),
                                        "auth" => Some(crate::secrets::AccessPolicy::WithAuth),
                                        "skill_only" => Some(crate::secrets::AccessPolicy::SkillOnly(skills_list)),
                                        _ => None,
                                    };
                                    let mut v = vault.lock().await;
                                    let resp = if let Some(policy) = policy {
                                        match v.set_credential_policy(name, policy) {
                                            Ok(()) => json!({
                                                "type": "secrets_set_policy_result",
                                                "ok": true,
                                            }),
                                            Err(e) => json!({
                                                "type": "secrets_set_policy_result",
                                                "ok": false,
                                                "message": format!("Failed to set policy: {}", e),
                                            }),
                                        }
                                    } else {
                                        json!({
                                            "type": "secrets_set_policy_result",
                                            "ok": false,
                                            "message": format!("Unknown policy: {}", policy_str),
                                        })
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_set_disabled") => {
                                    let name = val.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                    let disabled = val.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false);
                                    let mut v = vault.lock().await;
                                    let resp = match v.set_credential_disabled(name, disabled) {
                                        Ok(()) => json!({
                                            "type": "secrets_set_disabled_result",
                                            "ok": true,
                                        }),
                                        Err(e) => json!({
                                            "type": "secrets_set_disabled_result",
                                            "ok": false,
                                            "message": format!("Failed: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_delete_credential") => {
                                    let name = val.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                    let mut v = vault.lock().await;
                                    // Also delete legacy bare key if present
                                    let meta_key = format!("cred:{}", name);
                                    let is_legacy = v.get_secret(&meta_key, true).ok().flatten().is_none();
                                    if is_legacy {
                                        let _ = v.delete_secret(name);
                                    }
                                    let resp = match v.delete_credential(name) {
                                        Ok(()) => json!({
                                            "type": "secrets_delete_credential_result",
                                            "ok": true,
                                        }),
                                        Err(e) => json!({
                                            "type": "secrets_delete_credential_result",
                                            "ok": false,
                                            "message": format!("Failed: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_has_totp") => {
                                    let mut v = vault.lock().await;
                                    let has_totp = v.has_totp();
                                    let resp = json!({
                                        "type": "secrets_has_totp_result",
                                        "has_totp": has_totp,
                                    });
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_setup_totp") => {
                                    let mut v = vault.lock().await;
                                    let resp = match v.setup_totp("rustyclaw") {
                                        Ok(uri) => json!({
                                            "type": "secrets_setup_totp_result",
                                            "ok": true,
                                            "uri": uri,
                                        }),
                                        Err(e) => json!({
                                            "type": "secrets_setup_totp_result",
                                            "ok": false,
                                            "message": format!("Failed: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_verify_totp") => {
                                    let code = val.get("code").and_then(|c| c.as_str()).unwrap_or("");
                                    let mut v = vault.lock().await;
                                    let resp = match v.verify_totp(code) {
                                        Ok(valid) => json!({
                                            "type": "secrets_verify_totp_result",
                                            "ok": valid,
                                        }),
                                        Err(e) => json!({
                                            "type": "secrets_verify_totp_result",
                                            "ok": false,
                                            "message": format!("Error: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                Some("secrets_remove_totp") => {
                                    let mut v = vault.lock().await;
                                    let resp = match v.remove_totp() {
                                        Ok(()) => json!({
                                            "type": "secrets_remove_totp_result",
                                            "ok": true,
                                        }),
                                        Err(e) => json!({
                                            "type": "secrets_remove_totp_result",
                                            "ok": false,
                                            "message": format!("Failed: {}", e),
                                        }),
                                    };
                                    let _ = writer.send(Message::Text(resp.to_string().into())).await;
                                    continue;
                                }
                                _ => {
                                    // Not a secrets control message — fall through to dispatch
                                }
                            }
                        }

                        let workspace_dir = config.workspace_dir();
                        if let Err(err) = dispatch_text_message(
                            &http,
                            text.as_str(),
                            model_ctx.as_deref(),
                            copilot_session.as_deref(),
                            &mut writer,
                            &workspace_dir,
                            &vault,
                            &skill_mgr,
                            &tool_cancel,
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

    // Clean up reader task
    reader_handle.abort();

    Ok(())
}

/// Route an incoming text frame to the appropriate handler.
///
/// Implements an agentic tool loop: the model is called, and if it
/// requests tool calls, the gateway executes them locally and feeds
/// the results back into the conversation, repeating until the model
/// produces a final text response (or a safety limit is hit).
///
/// The `tool_cancel` flag can be set by another task to interrupt the
/// tool loop gracefully.
async fn dispatch_text_message(
    http: &reqwest::Client,
    text: &str,
    model_ctx: Option<&ModelContext>,
    copilot_session: Option<&CopilotSession>,
    writer: &mut WsWriter,
    workspace_dir: &std::path::Path,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    tool_cancel: &ToolCancelFlag,
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

    let mut resolved = match providers::resolve_request(req, model_ctx) {
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

    // Store the original API key for non-Copilot providers.
    // For Copilot, we'll refresh the session token on each loop iteration.
    let original_api_key = resolved.api_key.clone();

    // ── Agentic tool loop ───────────────────────────────────────────
    // No hard limit — the model will stop when it's done. The user can
    // cancel by sending a {"type": "cancel"} message (e.g., pressing Esc).
    // We use a very high limit as a safety net against infinite loops.
    const MAX_TOOL_ROUNDS: usize = 500;
    /// Maximum consecutive auto-continuations before giving up.
    /// Prevents infinite loops when the model keeps narrating intent
    /// but never actually makes tool calls.
    const MAX_AUTO_CONTINUES: usize = 2;

    let context_limit = helpers::context_window_for_model(&resolved.model);
    let mut consecutive_continues: usize = 0;

    for _round in 0..MAX_TOOL_ROUNDS {
        // ── Check for cancellation ──────────────────────────────────
        if tool_cancel.load(Ordering::Relaxed) {
            let frame = json!({
                "type": "info",
                "message": "Tool loop cancelled by user.",
            });
            writer
                .send(Message::Text(frame.to_string().into()))
                .await
                .context("Failed to send cancel info")?;
            providers::send_response_done(writer).await?;
            return Ok(());
        }

        // Refresh the bearer token before each model call.
        // For Copilot providers, this ensures the session token is still valid.
        match auth::resolve_bearer_token(
            http,
            &resolved.provider,
            original_api_key.as_deref(),
            copilot_session,
        )
        .await
        {
            Ok(token) => resolved.api_key = token,
            Err(err) => {
                let frame = json!({
                    "type": "error",
                    "ok": false,
                    "message": format!("Token refresh failed: {}", err),
                });
                writer
                    .send(Message::Text(frame.to_string().into()))
                    .await
                    .context("Failed to send error frame")?;
                return Ok(());
            }
        }

        // ── Auto-compact if context is getting large ────────────────
        let estimated = helpers::estimate_tokens(&resolved.messages);
        let threshold = (context_limit as f64 * COMPACTION_THRESHOLD) as usize;
        if estimated > threshold {
            match providers::compact_conversation(
                http,
                &mut resolved,
                context_limit,
                writer,
            )
            .await
            {
                Ok(()) => {} // compacted in-place
                Err(err) => {
                    // Non-fatal — log a warning and keep going with the
                    // full context; the provider may still accept it.
                    let warn_frame = json!({
                        "type": "info",
                        "message": format!("Context compaction failed: {}", err),
                    });
                    let _ = writer
                        .send(Message::Text(warn_frame.to_string().into()))
                        .await;
                }
            }
        }

        let result = if resolved.provider == "anthropic" {
            // Anthropic: use streaming mode with writer for real-time chunks
            providers::call_anthropic_with_tools(http, &resolved, Some(writer)).await
        } else if resolved.provider == "google" {
            providers::call_google_with_tools(http, &resolved).await
        } else {
            providers::call_openai_with_tools(http, &resolved).await
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
        // For Anthropic, text is already streamed via the writer, so skip if empty.
        // For other providers, send the accumulated text.
        eprintln!(
            "[Gateway] provider='{}', text_len={}, tool_calls={}",
            resolved.provider,
            model_resp.text.len(),
            model_resp.tool_calls.len()
        );
        if !model_resp.text.is_empty() && resolved.provider != "anthropic" {
            eprintln!("[Gateway] Sending chunk to TUI: {} chars", model_resp.text.len());
            providers::send_chunk(writer, &model_resp.text).await?;
        }

        // Check if the model is truly done or if something went wrong
        let finish_reason = model_resp.finish_reason.as_deref().unwrap_or("stop");

        if model_resp.tool_calls.is_empty() {
            // No tool calls requested
            if finish_reason == "stop" || finish_reason == "end_turn" {
                // ── Auto-continuation for incomplete intent ─────────────
                // Sometimes the model narrates what it plans to do ("Let me check...")
                // but returns finish_reason=stop without making a tool call.
                // This is common with large contexts or certain API proxies.
                // Detect this and prompt the model to continue.
                const INTENT_PATTERNS: &[&str] = &[
                    "Let me ",
                    "I'll ",
                    "I will ",
                    "Now let me ",
                    "Let's ",
                    "Now I'll ",
                    "I need to ",
                    "First, let me ",
                    "First let me ",
                ];
                let text_suggests_action = INTENT_PATTERNS
                    .iter()
                    .any(|p| model_resp.text.contains(p));

                // Also trigger continuation if response ends with colon (about to list/show something)
                let ends_with_continuation = model_resp.text.trim_end().ends_with(':');

                if (text_suggests_action || ends_with_continuation)
                    && consecutive_continues < MAX_AUTO_CONTINUES
                {
                    consecutive_continues += 1;
                    // Model said it would act but didn't — prompt continuation
                    eprintln!(
                        "[Gateway] Detected incomplete intent ({} chars text, no tool calls, ends_colon={}, attempt {}/{}), prompting continuation",
                        model_resp.text.len(),
                        ends_with_continuation,
                        consecutive_continues,
                        MAX_AUTO_CONTINUES
                    );

                    // Send the partial text to the client so they see it (with newline for visual separation)
                    if !model_resp.text.is_empty() && resolved.provider != "anthropic" {
                        providers::send_chunk(writer, &format!("{}\n", model_resp.text)).await?;
                    }

                    // Append assistant message and continuation prompt
                    resolved.messages.push(ChatMessage::text("assistant", &model_resp.text));
                    resolved.messages.push(ChatMessage::text(
                        "user",
                        "Continue. Execute the action you described.",
                    ));

                    // Don't send response_done — continue the tool loop
                    continue;
                }

                // Model explicitly finished — we're done
                providers::send_response_done(writer).await?;
                return Ok(());
            } else if finish_reason == "length" {
                // Hit token limit — warn and stop
                let frame = json!({
                    "type": "info",
                    "message": "Response truncated due to token limit.",
                });
                writer
                    .send(Message::Text(frame.to_string().into()))
                    .await
                    .context("Failed to send length warning")?;
                providers::send_response_done(writer).await?;
                return Ok(());
            } else {
                // Unexpected finish_reason with no tool calls
                // Log it and treat as done (better than looping forever)
                let frame = json!({
                    "type": "info",
                    "message": format!("Model finished with reason '{}' but no tool calls.", finish_reason),
                });
                writer
                    .send(Message::Text(frame.to_string().into()))
                    .await
                    .context("Failed to send finish info")?;
                providers::send_response_done(writer).await?;
                return Ok(());
            }
        }

        // Reset continuation counter — model made an actual tool call
        consecutive_continues = 0;

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
            let (output, is_error) = if tools::is_secrets_tool(&tc.name) {
                // Secrets tools are handled here — they need vault access.
                match secrets_handler::execute_secrets_tool(&tc.name, &tc.arguments, vault).await {
                    Ok(text) => (text, false),
                    Err(err) => (err, true),
                }
            } else if tools::is_skill_tool(&tc.name) {
                // Skill tools are handled here — they need SkillManager access.
                match skills_handler::execute_skill_tool(&tc.name, &tc.arguments, skill_mgr).await {
                    Ok(text) => (text, false),
                    Err(err) => (err, true),
                }
            } else {
                match tools::execute_tool(&tc.name, &tc.arguments, workspace_dir) {
                    Ok(text) => (text, false),
                    Err(err) => (err, true),
                }
            };

            // Sanitize the output (truncate large outputs, warn about garbage).
            let output = tools::sanitize_tool_output(output);

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
        providers::append_tool_round(
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
        "message": format!("Safety limit reached ({} tool rounds) — stopping to prevent infinite loop.", MAX_TOOL_ROUNDS),
    });
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send error frame")?;
    providers::send_response_done(writer).await?;
    Ok(())
}

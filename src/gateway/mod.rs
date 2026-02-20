//! Gateway module — WebSocket server for agent communication.
//!
//! This module provides the gateway server that handles WebSocket connections
//! from TUI clients, manages authentication, and dispatches chat requests to
//! model providers. It also polls configured messengers (Telegram, Discord, etc.)
//! for incoming messages and routes them through the model.

mod auth;
pub mod csrf;
pub mod health;
mod helpers;
mod messenger_handler;
mod providers;
pub mod protocol;
mod secrets_handler;
mod skills_handler;
mod types;

// Re-export protocol types
pub use protocol::{
    ClientFrameType, ServerFrameType, StatusType, ServerFrame, ClientFrame,
    serialize_frame, deserialize_frame, ServerPayload, ClientPayload, SecretEntryDto,
    FrameAction, server_frame_to_action,
};

// Re-export public types (includes protocol types via types module)
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
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::sync::CancellationToken;

/// Shared flag for cancelling the tool loop from another task.
pub type ToolCancelFlag = Arc<AtomicBool>;

/// Trait alias for an async stream that can be either a plain TCP or TLS stream.
trait AsyncStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncStream for T {}

/// A boxed stream that is either a plain TCP stream or a TLS-wrapped one.
type MaybeTlsStream = Box<dyn AsyncStream>;

/// Type alias for the server-side WebSocket write half.
type WsWriter = SplitSink<WebSocketStream<MaybeTlsStream>, Message>;

/// Gateway-owned secrets vault, shared across connections.
///
/// The vault may start in a locked state (no password provided yet) and
/// be unlocked later via a control message from an authenticated client.
pub type SharedVault = Arc<Mutex<SecretsManager>>;

/// Gateway-owned skill manager, shared across connections.
pub type SharedSkillManager = Arc<Mutex<SkillManager>>;

/// Shared config, updated on reload.
pub type SharedConfig = Arc<RwLock<Config>>;

/// Shared model context, updated on reload.
pub type SharedModelCtx = Arc<RwLock<Option<Arc<ModelContext>>>>;

// Re-export protocol helpers for external use
pub use protocol::server::{
    parse_client_frame, send_frame,
    send_vault_unlocked, send_secrets_list_result,
    send_secrets_store_result, send_secrets_get_result, send_secrets_delete_result,
    send_secrets_peek_result, send_secrets_set_policy_result, send_secrets_set_disabled_result,
    send_secrets_delete_credential_result, send_secrets_has_totp_result, send_secrets_setup_totp_result,
    send_secrets_verify_totp_result, send_secrets_remove_totp_result, send_reload_result,
};

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

    // ── Build TLS acceptor if cert/key are configured ───────────────
    let tls_acceptor: Option<tokio_rustls::TlsAcceptor> =
        match (&options.tls_cert, &options.tls_key) {
            (Some(cert_path), Some(key_path)) => {
                let cert_pem = std::fs::read(cert_path)
                    .with_context(|| format!("Failed to read TLS cert: {}", cert_path.display()))?;
                let key_pem = std::fs::read(key_path)
                    .with_context(|| format!("Failed to read TLS key: {}", key_path.display()))?;

                let certs: Vec<_> = rustls_pemfile::certs(&mut &cert_pem[..])
                    .collect::<Result<Vec<_>, _>>()
                    .context("Failed to parse TLS certificate PEM")?;
                let key = rustls_pemfile::private_key(&mut &key_pem[..])
                    .context("Failed to parse TLS private key PEM")?
                    .context("No private key found in PEM file")?;

                let tls_config = tokio_rustls::rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(certs, key)
                    .context("Invalid TLS certificate/key")?;

                eprintln!("[gateway] TLS enabled (WSS)");
                Some(tokio_rustls::TlsAcceptor::from(Arc::new(tls_config)))
            }
            (Some(_), None) | (None, Some(_)) => {
                anyhow::bail!("Both tls_cert and tls_key must be specified for WSS support");
            }
            (None, None) => None,
        };

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
    let shared_config: SharedConfig = Arc::new(RwLock::new(config.clone()));
    let shared_model_ctx: SharedModelCtx = Arc::new(RwLock::new(model_ctx.clone()));
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
                let shared_cfg = shared_config.clone();
                let shared_ctx = shared_model_ctx.clone();
                let session_clone = copilot_session.clone();
                let vault_clone = vault.clone();
                let skill_clone = skill_mgr.clone();
                let limiter_clone = rate_limiter.clone();
                let child_cancel = cancel.child_token();
                let tls = tls_acceptor.clone();
                tokio::spawn(async move {
                    // Wrap in TLS if configured, otherwise use plain TCP.
                    let boxed_stream: MaybeTlsStream = if let Some(acceptor) = tls {
                        match acceptor.accept(stream).await {
                            Ok(tls_stream) => Box::new(tls_stream),
                            Err(err) => {
                                eprintln!("TLS handshake failed from {}: {}", peer, err);
                                return;
                            }
                        }
                    } else {
                        Box::new(stream)
                    };

                    if let Err(err) = handle_connection(
                        boxed_stream, peer, shared_cfg, shared_ctx,
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
    stream: MaybeTlsStream,
    peer: SocketAddr,
    shared_config: SharedConfig,
    shared_model_ctx: SharedModelCtx,
    copilot_session: Option<Arc<CopilotSession>>,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    rate_limiter: auth::RateLimiter,
    cancel: CancellationToken,
) -> Result<()> {
    let ws_stream: WebSocketStream<MaybeTlsStream> = tokio_tungstenite::accept_async(stream)
        .await
        .context("WebSocket handshake failed")?;
    let (mut writer, mut reader) = ws_stream.split();
    let peer_ip = peer.ip();

    // Snapshot config and model context for this connection.
    // Reload updates the shared state; new connections pick up changes.
    let config = shared_config.read().await.clone();
    let model_ctx = shared_model_ctx.read().await.clone();

    // ── TOTP authentication challenge ───────────────────────────────
    //
    // If TOTP 2FA is enabled, we require the client to prove identity
    // before granting access to the gateway's capabilities.
    if config.totp_enabled {
        // Check rate limit first.
        if let Some(remaining) = auth::check_rate_limit(&rate_limiter, peer_ip).await {
            send_frame(
                &mut writer,
                &ServerFrame {
                    frame_type: ServerFrameType::AuthLocked,
                    payload: ServerPayload::AuthLocked {
                        message: format!("Too many failed attempts. Try again in {}s.", remaining),
                        retry_after: Some(remaining),
                    },
                },
            )
            .await?;
            writer.send(Message::Close(None)).await?;
            return Ok(());
        }

        // Send challenge.
        protocol::server::send_auth_challenge(&mut writer, "totp")
            .await
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
                        protocol::server::send_auth_result(&mut writer, true, None, None)
                            .await?;
                        break; // Authentication successful, continue to main loop
                    } else {
                        attempts += 1;
                        let locked_out = auth::record_totp_failure(&rate_limiter, peer_ip).await;

                        if locked_out {
                            let msg = format!(
                                "Invalid code. Too many failures — locked out for {}s.",
                                TOTP_LOCKOUT_SECS,
                            );
                            protocol::server::send_auth_result(&mut writer, false, Some(&msg), None)
                                .await?;
                            writer.send(Message::Close(None)).await?;
                            return Ok(());
                        } else if attempts >= MAX_TOTP_ATTEMPTS {
                            let msg = "Invalid code. Maximum attempts exceeded.";
                            protocol::server::send_auth_result(&mut writer, false, Some(msg), None)
                                .await?;
                            writer.send(Message::Close(None)).await?;
                            return Ok(());
                        } else {
                            let remaining = MAX_TOTP_ATTEMPTS - attempts;
                            let msg = format!(
                                "Invalid 2FA code. {} attempt{} remaining.",
                                remaining,
                                if remaining == 1 { "" } else { "s" }
                            );
                            protocol::server::send_auth_result(&mut writer, false, Some(&msg), Some(true))
                                .await?;
                            // Continue loop to allow retry
                        }
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("Auth error from {}: {}", peer, e);
                    return Ok(());
                }
                Err(_) => {
                    protocol::server::send_auth_result(
                        &mut writer,
                        false,
                        Some("Authentication timed out."),
                        None,
                    )
                    .await?;
                    writer.send(Message::Close(None)).await?;
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
    protocol::server::send_hello(
        &mut writer,
        "rustyclaw",
        &config.settings_dir.to_string_lossy(),
        vault_is_locked,
        model_ctx.as_ref().map(|c| c.provider.as_str()),
        model_ctx.as_ref().map(|c| c.model.as_str()),
    )
    .await
    .context("Failed to send hello message")?;

    if vault_is_locked {
        protocol::server::send_status(
            &mut writer,
            StatusType::VaultLocked,
            "Secrets vault is locked — provide password to unlock",
        )
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
            protocol::server::send_status(&mut writer, StatusType::ModelConfigured, &detail)
                .await
                .context("Failed to send model_configured status")?;

            // 2. Credentials
            if ctx.api_key.is_some() {
                protocol::server::send_status(
                    &mut writer,
                    StatusType::CredentialsLoaded,
                    &format!("{} API key loaded", display),
                )
                .await
                .context("Failed to send credentials_loaded status")?;
            } else if crate_providers::secret_key_for_provider(&ctx.provider).is_some() {
                protocol::server::send_status(
                    &mut writer,
                    StatusType::CredentialsMissing,
                    &format!("No API key for {} — model calls will fail", display),
                )
                .await
                .context("Failed to send credentials_missing status")?;
            }

            // 3. Validate the connection with a lightweight probe
            //
            // For Copilot providers, exchange the OAuth token for a session
            // token first — the probe must use the session token too.
            //
            // If the cached model context has no API key, try fetching it
            // from the vault (it may have been stored since startup).
            let probe_ctx = if ctx.api_key.is_none() {
                if let Some(key_name) = crate_providers::secret_key_for_provider(&ctx.provider) {
                    let mut v = vault.lock().await;
                    if let Ok(Some(key)) = v.get_secret(key_name, true) {
                        let mut updated = (**ctx).clone();
                        updated.api_key = Some(key);
                        std::sync::Arc::new(updated)
                    } else {
                        ctx.clone()
                    }
                } else {
                    ctx.clone()
                }
            } else {
                ctx.clone()
            };

            protocol::server::send_status(
                &mut writer,
                StatusType::ModelConnecting,
                &format!("Probing {} …", ctx.base_url),
            )
            .await
            .context("Failed to send model_connecting status")?;

            match providers::validate_model_connection(&http, &probe_ctx, copilot_session.as_deref()).await {
                ProbeResult::Ready => {
                    protocol::server::send_status(
                        &mut writer,
                        StatusType::ModelReady,
                        &format!("{} / {} ready", display, ctx.model),
                    )
                    .await
                    .context("Failed to send model_ready status")?;
                }
                ProbeResult::Connected { warning } => {
                    // Auth is fine, provider is reachable — the specific
                    // probe request wasn't accepted, but chat will likely
                    // work with the real request format.
                    protocol::server::send_status(
                        &mut writer,
                        StatusType::ModelReady,
                        &format!("{} / {} connected (probe: {})", display, ctx.model, warning),
                    )
                    .await
                    .context("Failed to send model_ready status")?;
                }
                ProbeResult::AuthError { detail } => {
                    protocol::server::send_status(
                        &mut writer,
                        StatusType::ModelError,
                        &format!("{} auth failed: {}", display, detail),
                    )
                    .await
                    .context("Failed to send model_error status")?;
                }
                ProbeResult::Unreachable { detail } => {
                    protocol::server::send_status(
                        &mut writer,
                        StatusType::ModelError,
                        &format!("{} probe failed: {}", display, detail),
                    )
                    .await
                    .context("Failed to send model_error status")?;
                }
            }
        }
        None => {
            protocol::server::send_status(
                &mut writer,
                StatusType::NoModel,
                "No model configured — clients must send full credentials",
            )
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

    // Channel for tool-approval responses (used by the Ask permission flow).
    let (approval_tx, approval_rx) = tokio::sync::mpsc::channel::<(String, bool)>(4);
    let approval_rx = Arc::new(Mutex::new(approval_rx));

    // Channel for user-prompt responses (used by the ask_user tool).
    let (user_prompt_tx, user_prompt_rx) =
        tokio::sync::mpsc::channel::<(String, bool, serde_json::Value)>(4);
    let user_prompt_rx = Arc::new(Mutex::new(user_prompt_rx));

    let reader_cancel = cancel.clone();
    let reader_tool_cancel = tool_cancel.clone();
    let reader_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = reader_cancel.cancelled() => break,
                msg = reader.next() => {
                    match msg {
                        Some(Ok(Message::Text(_))) => {
                            // Text frames are not supported - skip them
                            continue;
                        }
                        Some(Ok(Message::Binary(ref data))) => {
                            eprintln!("[Gateway][DEBUG] Received Binary frame from client: {} bytes", data.len());
                            // Check for cancel message in binary
                            match parse_client_frame(data) {
                                Ok(frame) => {
                                    eprintln!("[Gateway][DEBUG] Parsed client frame: {:?}", frame.frame_type);
                                    if frame.frame_type == ClientFrameType::Cancel {
                                        reader_tool_cancel.store(true, Ordering::Relaxed);
                                        continue;
                                    }
                                    if frame.frame_type == ClientFrameType::ToolApprovalResponse {
                                        if let ClientPayload::ToolApprovalResponse { id, approved } = frame.payload {
                                            let _ = approval_tx.send((id, approved)).await;
                                            continue;
                                        }
                                    }
                                    if frame.frame_type == ClientFrameType::UserPromptResponse {
                                        if let ClientPayload::UserPromptResponse { id, dismissed, value } = frame.payload {
                                            let _ = user_prompt_tx.send((id, dismissed, value)).await;
                                            continue;
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[Gateway][DEBUG] Failed to parse client frame: {}", e);
                                }
                            }
                            // Forward binary messages
                            if msg_tx.send(Message::Binary(data.clone())).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(msg)) => {
                            eprintln!("[Gateway][DEBUG] Received non-binary message from client: {:?}", msg);
                            if msg_tx.send(msg).await.is_err() {
                                break;
                            }
                        }
                        Some(Err(e)) => {
                            eprintln!("[Gateway][DEBUG] Error reading message from client: {}", e);
                            break;
                        }
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
                    Message::Binary(data) => {
                        eprintln!("[Gateway][DEBUG] Handling Binary message from client: {} bytes", data.len());
                        // Reset cancel flag for new request
                        tool_cancel.store(false, Ordering::Relaxed);

                        // Parse the binary frame
                        let frame = match deserialize_frame::<ClientFrame>(&data) {
                            Ok(f) => {
                                eprintln!("[Gateway][DEBUG] Successfully deserialized ClientFrame: {:?}", f.frame_type);
                                f
                            },
                            Err(e) => {
                                eprintln!("[Gateway][DEBUG] Failed to deserialize ClientFrame: {}", e);
                                // Send error response
                                let error_frame = ServerFrame {
                                    frame_type: ServerFrameType::Error,
                                    payload: ServerPayload::Error {
                                        ok: false,
                                        message: format!("Failed to parse client frame: {}", e),
                                    },
                                };
                                send_frame(&mut writer, &error_frame).await?;
                                continue;
                            }
                        };

                        // Handle the frame based on type
                        match frame.payload {
                            ClientPayload::UnlockVault { password } => {
                                let mut v = vault.lock().await;
                                v.set_password(password);
                                match v.get_secret("__vault_check__", true) {
                                    Ok(_) => {
                                        send_vault_unlocked(&mut writer, true, None).await?;
                                    }
                                    Err(e) => {
                                        v.clear_password();
                                        send_vault_unlocked(
                                            &mut writer,
                                            false,
                                            Some(&format!("Failed to unlock vault: {}", e)),
                                        ).await?;
                                    }
                                }
                            }
                            ClientPayload::SecretsList => {
                                let mut v = vault.lock().await;
                                let entries = v.list_all_entries();
                                let dto_entries: Vec<SecretEntryDto> = entries
                                    .iter()
                                    .map(|(name, entry)| SecretEntryDto {
                                        name: name.clone(),
                                        label: entry.label.clone(),
                                        kind: format!("{:?}", entry.kind),
                                        policy: format!("{:?}", entry.policy),
                                        disabled: entry.disabled,
                                    })
                                    .collect();
                                send_secrets_list_result(&mut writer, true, dto_entries).await?;
                            }
                            ClientPayload::SecretsStore { key, value } => {
                                let mut v = vault.lock().await;
                                let result = v.store_secret(&key, &value);
                                match result {
                                    Ok(()) => send_secrets_store_result(
                                        &mut writer,
                                        true,
                                        &format!("Secret '{}' stored.", key),
                                    ).await?,
                                    Err(e) => send_secrets_store_result(
                                        &mut writer,
                                        false,
                                        &format!("Failed to store secret: {}", e),
                                    ).await?,
                                };
                            }
                            ClientPayload::SecretsGet { key } => {
                                let mut v = vault.lock().await;
                                let result = v.get_secret(&key, true);
                                match result {
                                    Ok(Some(value)) => {
                                        send_secrets_get_result(&mut writer, true, &key, Some(&value), None).await?
                                    }
                                    Ok(None) => {
                                        send_secrets_get_result(&mut writer, false, &key, None, Some(&format!("Secret '{}' not found.", key))).await?
                                    }
                                    Err(e) => {
                                        send_secrets_get_result(&mut writer, false, &key, None, Some(&format!("Failed to get secret: {}", e))).await?
                                    }
                                };
                            }
                            ClientPayload::SecretsDelete { key } => {
                                let mut v = vault.lock().await;
                                let result = v.delete_secret(&key);
                                match result {
                                    Ok(()) => send_secrets_delete_result(&mut writer, true, None).await?,
                                    Err(e) => send_secrets_delete_result(
                                        &mut writer,
                                        false,
                                        Some(&format!("Failed to delete: {}", e)),
                                    ).await?,
                                };
                            }
                            ClientPayload::SecretsPeek { name } => {
                                let mut v = vault.lock().await;
                                let result = v.peek_credential_display(&name);
                                match result {
                                    Ok(fields) => {
                                        let field_tuples: Vec<(String, String)> = fields
                                            .iter()
                                            .map(|(label, value)| (label.clone(), value.clone()))
                                            .collect();
                                        send_secrets_peek_result(&mut writer, true, field_tuples, None)
                                            .await?
                                    }
                                    Err(e) => send_secrets_peek_result(
                                        &mut writer,
                                        false,
                                        vec![],
                                        Some(&format!("Failed to peek: {}", e)),
                                    ).await?,
                                };
                            }
                            ClientPayload::SecretsSetPolicy { name, policy, skills } => {
                                let mut v = vault.lock().await;
                                let policy_str = policy.clone();
                                let policy = match policy.as_str() {
                                    "always" => Some(crate::secrets::AccessPolicy::Always),
                                    "ask" => Some(crate::secrets::AccessPolicy::WithApproval),
                                    "auth" => Some(crate::secrets::AccessPolicy::WithAuth),
                                    "skill_only" => Some(crate::secrets::AccessPolicy::SkillOnly(skills)),
                                    _ => None,
                                };
                                if let Some(policy) = policy {
                                    let result = v.set_credential_policy(&name, policy);
                                    match result {
                                        Ok(()) => send_secrets_set_policy_result(&mut writer, true, None).await?,
                                        Err(e) => send_secrets_set_policy_result(
                                            &mut writer,
                                            false,
                                            Some(&format!("Failed to set policy: {}", e)),
                                        ).await?,
                                    }
                                } else {
                                    send_secrets_set_policy_result(
                                        &mut writer,
                                        false,
                                        Some(&format!("Unknown policy: {}", policy_str)),
                                    ).await?;
                                }
                            }
                            ClientPayload::SecretsSetDisabled { name, disabled } => {
                                let mut v = vault.lock().await;
                                let result = v.set_credential_disabled(&name, disabled);
                                match result {
                                    Ok(()) => send_secrets_set_disabled_result(&mut writer, true, None).await?,
                                    Err(e) => send_secrets_set_disabled_result(&mut writer, false, Some(&format!("Failed: {}", e))).await?,
                                };
                            }
                            ClientPayload::SecretsDeleteCredential { name } => {
                                let mut v = vault.lock().await;
                                let meta_key = format!("cred:{}", name);
                                let is_legacy = v.get_secret(&meta_key, true).ok().flatten().is_none();
                                if is_legacy {
                                    let _ = v.delete_secret(&name);
                                }
                                let result = v.delete_credential(&name);
                                match result {
                                    Ok(()) => send_secrets_delete_credential_result(&mut writer, true, None).await?,
                                    Err(e) => send_secrets_delete_credential_result(&mut writer, false, Some(&format!("Failed: {}", e))).await?,
                                };
                            }
                            ClientPayload::SecretsHasTotp => {
                                let mut v = vault.lock().await;
                                let has_totp = v.has_totp();
                                send_secrets_has_totp_result(&mut writer, has_totp).await?;
                            }
                            ClientPayload::SecretsSetupTotp => {
                                let mut v = vault.lock().await;
                                let result = v.setup_totp("rustyclaw");
                                match result {
                                    Ok(uri) => send_secrets_setup_totp_result(&mut writer, true, Some(&uri), None).await?,
                                    Err(e) => send_secrets_setup_totp_result(&mut writer, false, None, Some(&format!("Failed: {}", e))).await?,
                                };
                            }
                            ClientPayload::SecretsVerifyTotp { code } => {
                                let mut v = vault.lock().await;
                                let result = v.verify_totp(&code);
                                match result {
                                    Ok(valid) => send_secrets_verify_totp_result(&mut writer, valid, None).await?,
                                    Err(e) => send_secrets_verify_totp_result(&mut writer, false, Some(&format!("Error: {}", e))).await?,
                                };
                            }
                            ClientPayload::SecretsRemoveTotp => {
                                let mut v = vault.lock().await;
                                let result = v.remove_totp();
                                match result {
                                    Ok(()) => send_secrets_remove_totp_result(&mut writer, true, None).await?,
                                    Err(e) => send_secrets_remove_totp_result(&mut writer, false, Some(&format!("Failed: {}", e))).await?,
                                };
                            }
                            ClientPayload::Reload => {
                                let settings_dir = config.settings_dir.clone();
                                let config_path = settings_dir.join("config.toml");
                                match Config::load(Some(config_path)) {
                                    Ok(new_config) => {
                                        let new_model_ctx = {
                                            let mut v = vault.lock().await;
                                            ModelContext::resolve(&new_config, &mut v).ok().map(Arc::new)
                                        };

                                        let (provider, model) = if let Some(ref ctx) = new_model_ctx {
                                            (ctx.provider.clone(), ctx.model.clone())
                                        } else {
                                            ("(none)".to_string(), "(none)".to_string())
                                        };

                                        {
                                            let mut cfg = shared_config.write().await;
                                            *cfg = new_config;
                                        }
                                        {
                                            let mut ctx = shared_model_ctx.write().await;
                                            *ctx = new_model_ctx.clone();
                                        }

                                        send_reload_result(&mut writer, true, &provider, &model, None).await?;

                                        if let Some(ref ctx) = new_model_ctx {
                                            let display = crate_providers::display_name_for_provider(&ctx.provider);
                                            let detail = format!("{} / {} (reloaded)", display, ctx.model);
                                            protocol::server::send_status(
                                                &mut writer,
                                                StatusType::ModelConfigured,
                                                &detail,
                                            ).await?;
                                        }
                                    }
                                    Err(e) => {
                                        protocol::server::send_error(
                                            &mut writer,
                                            &format!("Failed to reload config: {}", e),
                                        ).await?;
                                    }
                                }
                            }
                            ClientPayload::Chat { messages } => {
                                // Re-read model_ctx from shared state for each dispatch
                                let current_model_ctx = shared_model_ctx.read().await.clone();
                                let workspace_dir = config.workspace_dir();

                                // Build a ChatRequest from the messages
                                let chat_request = ChatRequest {
                                    msg_type: "chat".to_string(),
                                    messages,
                                    model: None,
                                    provider: None,
                                    base_url: None,
                                    api_key: None,
                                };

                                if let Err(err) = dispatch_text_message(
                                    &http,
                                    &chat_request,
                                    current_model_ctx.as_deref(),
                                    copilot_session.as_deref(),
                                    &mut writer,
                                    &workspace_dir,
                                    &vault,
                                    &skill_mgr,
                                    &tool_cancel,
                                    &shared_config,
                                    &approval_rx,
                                    &user_prompt_rx,
                                )
                                .await
                                {
                                    let error_frame = ServerFrame {
                                        frame_type: ServerFrameType::Error,
                                        payload: ServerPayload::Error {
                                            ok: false,
                                            message: err.to_string(),
                                        },
                                    };
                                    send_frame(&mut writer, &error_frame).await?;
                                }
                            }
                            ClientPayload::Empty | ClientPayload::AuthChallenge { .. } | ClientPayload::AuthResponse { .. } | ClientPayload::ToolApprovalResponse { .. } | ClientPayload::UserPromptResponse { .. } => {
                                // AuthChallenge/AuthResponse handled in auth phase.
                                // ToolApprovalResponse handled by the reader task.
                                // UserPromptResponse handled by the reader task.
                            }
                        }
                    }
                    Message::Text(_) => {
                        // Reject text frames - only binary is supported
                        let error_frame = ServerFrame {
                            frame_type: ServerFrameType::Error,
                            payload: ServerPayload::Error {
                                ok: false,
                                message: "Text frames are not supported. Use binary protocol.".to_string(),
                            },
                        };
                        send_frame(&mut writer, &error_frame).await?;
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

/// Execute the `ask_user` tool by sending a prompt to the TUI and waiting
/// for the user's response on the user_prompt channel.
async fn execute_user_prompt(
    writer: &mut WsWriter,
    call_id: &str,
    arguments: &serde_json::Value,
    user_prompt_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, bool, serde_json::Value)>>>,
) -> (String, bool) {
    use crate::dialogs::user_prompt::{FormField, PromptOption, PromptType, UserPrompt};

    // Parse arguments into a UserPrompt
    let prompt_type_str = arguments
        .get("prompt_type")
        .and_then(|v| v.as_str())
        .unwrap_or("text");
    let title = arguments
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Question")
        .to_string();
    let description = arguments
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let options: Vec<PromptOption> = arguments
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|o| {
                    if let Some(s) = o.as_str() {
                        PromptOption {
                            label: s.to_string(),
                            description: None,
                            value: None,
                        }
                    } else {
                        PromptOption {
                            label: o
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .to_string(),
                            description: o
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            value: o
                                .get("value")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let prompt_type = match prompt_type_str {
        "select" => {
            let default = arguments
                .get("default_value")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            PromptType::Select { options, default }
        }
        "multi_select" => {
            let defaults: Vec<usize> = arguments
                .get("default_value")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as usize))
                        .collect()
                })
                .unwrap_or_default();
            PromptType::MultiSelect { options, defaults }
        }
        "confirm" => {
            let default = arguments
                .get("default_value")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            PromptType::Confirm { default }
        }
        "text" => {
            let placeholder = arguments
                .get("placeholder")
                .or_else(|| arguments.get("description"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let default = arguments
                .get("default_value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            PromptType::TextInput {
                placeholder,
                default,
            }
        }
        "form" => {
            let fields: Vec<FormField> = arguments
                .get("fields")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|f| FormField {
                            name: f
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("field")
                                .to_string(),
                            label: f
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Field")
                                .to_string(),
                            placeholder: f
                                .get("placeholder")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            default: f
                                .get("default")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            required: f
                                .get("required")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default();
            PromptType::Form { fields }
        }
        _ => PromptType::TextInput {
            placeholder: None,
            default: None,
        },
    };

    let prompt = UserPrompt {
        id: call_id.to_string(),
        title,
        description,
        prompt_type,
    };

    // Serialize the prompt to JSON and send it to the TUI.
    let prompt_json = match serde_json::to_string(&prompt) {
        Ok(json) => json,
        Err(e) => {
            return (format!("Failed to serialize user prompt: {}", e), true);
        }
    };

    if let Err(e) = protocol::server::send_user_prompt_request(
        writer,
        call_id,
        &prompt_json,
    )
    .await
    {
        return (format!("Failed to send user prompt: {}", e), true);
    }

    // Wait for the user's response (with 5 minute timeout).
    let rx_result = {
        let mut rx = user_prompt_rx.lock().await;
        tokio::time::timeout(std::time::Duration::from_secs(300), rx.recv()).await
    };

    match rx_result {
        Ok(Some((id, dismissed, value))) if id == call_id => {
            if dismissed {
                ("User dismissed the prompt without answering.".to_string(), false)
            } else {
                (serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()), false)
            }
        }
        Ok(Some(_)) => ("Mismatched prompt response ID.".to_string(), true),
        Ok(None) => ("User prompt channel closed.".to_string(), true),
        Err(_) => ("User prompt timed out after 5 minutes.".to_string(), true),
    }
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
    req: &ChatRequest,
    model_ctx: Option<&ModelContext>,
    copilot_session: Option<&CopilotSession>,
    writer: &mut WsWriter,
    workspace_dir: &std::path::Path,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    tool_cancel: &ToolCancelFlag,
    shared_config: &SharedConfig,
    approval_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, bool)>>>,
    user_prompt_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, bool, serde_json::Value)>>>,
) -> Result<()> {
    let mut resolved = match providers::resolve_request(req.clone(), model_ctx) {
        Ok(r) => r,
        Err(msg) => {
            let error_frame = ServerFrame {
                frame_type: ServerFrameType::Error,
                payload: ServerPayload::Error {
                    ok: false,
                    message: msg,
                },
            };
            send_frame(writer, &error_frame).await.context("Failed to send error frame")?;
            return Ok(());
        }
    };

    // If we still don't have an API key, try fetching it fresh from
    // the vault.  This handles the case where a key was stored after
    // the gateway started (e.g. user entered it via the TUI dialog).
    if resolved.api_key.is_none() {
        if let Some(key_name) = crate::providers::secret_key_for_provider(&resolved.provider) {
            let mut v = vault.lock().await;
            if let Ok(Some(key)) = v.get_secret(key_name, true) {
                resolved.api_key = Some(key);
            }
        }
    }

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

    // Memory flush controller - tracks whether we've flushed this conversation
    use crate::memory_flush::MemoryFlush;
    let flush_config = {
        let cfg = shared_config.read().await;
        cfg.memory_flush.clone()
    };
    let mut memory_flush = MemoryFlush::new(flush_config);

    for _round in 0..MAX_TOOL_ROUNDS {
        // ── Check for cancellation ──────────────────────────────────
        if tool_cancel.load(Ordering::Relaxed) {
            protocol::server::send_info(writer, "Tool loop cancelled by user.").await?;
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
                protocol::server::send_error(writer, &format!("Token refresh failed: {}", err)).await?;
                return Ok(());
            }
        }

        // ── Pre-compaction memory flush ─────────────────────────────
        // Check if we should trigger a memory flush before compaction
        let estimated = helpers::estimate_tokens(&resolved.messages);
        let threshold = (context_limit as f64 * COMPACTION_THRESHOLD) as usize;
        
        if memory_flush.should_flush(estimated, context_limit, COMPACTION_THRESHOLD) {
            let (system_msg, user_msg) = memory_flush.build_flush_messages();
            
            // Inject memory flush prompt
            // The agent will process this and can use tools to write to memory files
            resolved.messages.push(ChatMessage::text("system", &system_msg));
            resolved.messages.push(ChatMessage::text("user", &user_msg));
            
            // Mark as flushed to prevent repeated injections
            memory_flush.mark_flushed();
            
            // Notify the TUI about the memory flush
            let _ = protocol::server::send_info(
                writer,
                "💾 Memory flush triggered before compaction"
            ).await;
        }
        
        // ── Auto-compact if context is getting large ────────────────
        // Proceed with compaction if over threshold
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
                    let _ = protocol::server::send_info(writer, &format!("Context compaction failed: {}", err)).await;
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
                protocol::server::send_error(writer, &err.to_string()).await?;
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
                //
                // Guard: only trigger for SHORT responses (< 500 chars).
                // Long responses (e.g., a capabilities listing) are always
                // complete answers and must never be auto-continued.
                let consider_continuation = model_resp.text.len() < 500
                    && consecutive_continues < MAX_AUTO_CONTINUES;

                let should_continue = if consider_continuation {
                    // Check only the tail of the response for intent patterns.
                    let tail = if model_resp.text.len() > 200 {
                        &model_resp.text[model_resp.text.len() - 200..]
                    } else {
                        &model_resp.text
                    };

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

                    // Phrases that look like intent but are actually polite
                    // closers or conversational filler — never action intent.
                    let text_lower = model_resp.text.to_lowercase();
                    const NON_ACTION_PHRASES: &[&str] = &[
                        "let me know",
                        "i'll help",
                        "i'll guide",
                        "i'll be happy",
                        "i'll be glad",
                        "i'll do my best",
                        "i'll try my best",
                        "i'll assist",
                        "let's get started",
                        "let's begin",
                        "let me help",
                    ];

                    let has_exclusion = NON_ACTION_PHRASES
                        .iter()
                        .any(|p| text_lower.contains(p));

                    let text_suggests_action = !has_exclusion
                        && INTENT_PATTERNS.iter().any(|p| tail.contains(p));

                    // Also trigger if response ends with colon (about to list/show)
                    let ends_with_continuation = tail.trim_end().ends_with(':');

                    text_suggests_action || ends_with_continuation
                } else {
                    false
                };

                if should_continue {
                    consecutive_continues += 1;
                    eprintln!(
                        "[Gateway] Detected incomplete intent ({} chars text, no tool calls, attempt {}/{}), prompting continuation",
                        model_resp.text.len(),
                        consecutive_continues,
                        MAX_AUTO_CONTINUES
                    );

                    // NOTE: do NOT re-send the text here — for non-Anthropic
                    // providers it was already sent above; for Anthropic it was
                    // already streamed via the SSE handler.  Re-sending would
                    // cause the TUI to show the same text twice.

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
                protocol::server::send_info(writer, "Response truncated due to token limit.").await?;
                providers::send_response_done(writer).await?;
                return Ok(());
            } else {
                // Unexpected finish_reason with no tool calls
                // Log it and treat as done (better than looping forever)
                protocol::server::send_info(writer, &format!("Model finished with reason '{}' but no tool calls.", finish_reason)).await?;
                providers::send_response_done(writer).await?;
                return Ok(());
            }
        }

        // Reset continuation counter — model made an actual tool call
        consecutive_continues = 0;

        // ── Execute each requested tool ─────────────────────────────
        let mut tool_results: Vec<ToolCallResult> = Vec::new();

        // Snapshot current tool permissions (cheap clone of a HashMap).
        let tool_permissions = {
            let cfg = shared_config.read().await;
            cfg.tool_permissions.clone()
        };

        for tc in &model_resp.tool_calls {
            // ── Permission check ────────────────────────────────────
            let permission = tool_permissions
                .get(&tc.name)
                .cloned()
                .unwrap_or_default(); // default = Allow

            let (output, is_error) = match permission {
                tools::ToolPermission::Deny => {
                    // Notify the client about the denied tool call.
                    protocol::server::send_tool_call(
                        writer,
                        &tc.id,
                        &tc.name,
                        tc.arguments.clone(),
                    ).await?;

                    let msg = format!(
                        "Tool '{}' is denied by user policy. The user has blocked this tool from being executed.",
                        tc.name
                    );
                    (msg, true)
                }
                tools::ToolPermission::SkillOnly(_) => {
                    // In direct chat, SkillOnly tools are denied.
                    protocol::server::send_tool_call(
                        writer,
                        &tc.id,
                        &tc.name,
                        tc.arguments.clone(),
                    ).await?;

                    let msg = format!(
                        "Tool '{}' is restricted to skill-based invocations only. It cannot be used in direct chat.",
                        tc.name
                    );
                    (msg, true)
                }
                tools::ToolPermission::Ask => {
                    // Send approval request to the TUI and wait for response.
                    protocol::server::send_tool_approval_request(
                        writer,
                        &tc.id,
                        &tc.name,
                        tc.arguments.clone(),
                    ).await?;

                    // Wait for the user's response (with timeout).
                    let approved = {
                        let mut rx = approval_rx.lock().await;
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(120),
                            rx.recv(),
                        ).await {
                            Ok(Some((id, approved))) if id == tc.id => approved,
                            Ok(Some(_)) => false, // Mismatched ID — treat as denied
                            Ok(None) => false,    // Channel closed
                            Err(_) => false,      // Timeout
                        }
                    };

                    if !approved {
                        // Notify the client about the denied tool call.
                        protocol::server::send_tool_call(
                            writer,
                            &tc.id,
                            &tc.name,
                            tc.arguments.clone(),
                        ).await?;

                        let msg = format!(
                            "Tool '{}' was denied by the user.",
                            tc.name
                        );
                        (msg, true)
                    } else {
                        // User approved — proceed with execution.
                        protocol::server::send_tool_call(
                            writer,
                            &tc.id,
                            &tc.name,
                            tc.arguments.clone(),
                        ).await?;

                        if tools::is_user_prompt_tool(&tc.name) {
                            execute_user_prompt(writer, &tc.id, &tc.arguments, user_prompt_rx).await
                        } else if tools::is_secrets_tool(&tc.name) {
                            match secrets_handler::execute_secrets_tool(&tc.name, &tc.arguments, vault).await {
                                Ok(text) => (text, false),
                                Err(err) => (err, true),
                            }
                        } else if tools::is_skill_tool(&tc.name) {
                            match skills_handler::execute_skill_tool(&tc.name, &tc.arguments, skill_mgr).await {
                                Ok(text) => (text, false),
                                Err(err) => (err, true),
                            }
                        } else {
                            match tools::execute_tool(&tc.name, &tc.arguments, workspace_dir) {
                                Ok(text) => (text, false),
                                Err(err) => (err, true),
                            }
                        }
                    }
                }
                tools::ToolPermission::Allow => {
                    // Notify the client about the tool call.
                    protocol::server::send_tool_call(
                        writer,
                        &tc.id,
                        &tc.name,
                        tc.arguments.clone(),
                    ).await?;

                    // Execute the tool.
                    if tools::is_user_prompt_tool(&tc.name) {
                        execute_user_prompt(writer, &tc.id, &tc.arguments, user_prompt_rx).await
                    } else if tools::is_secrets_tool(&tc.name) {
                        match secrets_handler::execute_secrets_tool(&tc.name, &tc.arguments, vault).await {
                            Ok(text) => (text, false),
                            Err(err) => (err, true),
                        }
                    } else if tools::is_skill_tool(&tc.name) {
                        match skills_handler::execute_skill_tool(&tc.name, &tc.arguments, skill_mgr).await {
                            Ok(text) => (text, false),
                            Err(err) => (err, true),
                        }
                    } else {
                        match tools::execute_tool(&tc.name, &tc.arguments, workspace_dir) {
                            Ok(text) => (text, false),
                            Err(err) => (err, true),
                        }
                    }
                }
            };

            // Sanitize the output (truncate large outputs, warn about garbage).
            let output = tools::sanitize_tool_output(output);

            // Notify the client about the result.
            protocol::server::send_tool_result(
                writer,
                &tc.id,
                &tc.name,
                &output,
                is_error,
            ).await?;

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
    protocol::server::send_error(
        writer,
        &format!("Safety limit reached ({} tool rounds) — stopping to prevent infinite loop.", MAX_TOOL_ROUNDS),
    ).await?;
    providers::send_response_done(writer).await?;
    Ok(())
}

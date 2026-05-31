//! Gateway server engine.
//!
//! The core session loop: accepts transports, authenticates connections,
//! dispatches chat/messenger requests to model providers, and streams results
//! back. Driven by [`run_gateway`], which accepts both networked and
//! SSH-subsystem stdio transports. Invoked from the binary entry point in
//! `main.rs`.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{
    ChatMessage, ChatRequest, ClientFrame, ClientFrameType, ClientPayload, CopilotSession,
    GatewayOptions, ModelContext, ProbeResult, ProviderRequest, ScopedTransportWriter,
    SecretEntryDto, ServerFrame, ServerFrameType, ServerPayload, StatusType, Transport,
    TransportAcceptor, WireFrame, deserialize_frame, protocol, transport,
};
use rustyclaw_core::providers as crate_providers;
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::tools;

use protocol::server::{
    send_frame, send_reload_result, send_secrets_delete_credential_result,
    send_secrets_delete_result, send_secrets_get_result, send_secrets_has_totp_result,
    send_secrets_list_result, send_secrets_peek_result, send_secrets_remove_totp_result,
    send_secrets_set_disabled_result, send_secrets_set_policy_result,
    send_secrets_setup_totp_result, send_secrets_store_result, send_secrets_verify_totp_result,
    send_vault_unlocked,
};

use crate::dispatch::dispatch_text_message;
use crate::messenger_handler::SharedMessengerManager;
use crate::ssh::{SshConfig, SshServer, StdioTransport};
use crate::thread_updates::{send_thread_messages_update, send_threads_update};
use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedModelRegistry, SharedObserver,
    SharedSkillManager, SharedTaskManager, SharedVault, TOTP_LOCKOUT_SECS, ToolCancelFlag, auth,
    concurrent, messenger_handler, providers, system_prompt,
};

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
        debug!("OpenClaw token also expired");
        return None;
    }

    info!(
        remaining_hours = remaining / 3600,
        "Auto-imported fresh token from OpenClaw"
    );

    // Store in our vault for next time
    let session_data = serde_json::json!({
        "session_token": token,
        "expires_at": expires_at,
    });
    if let Err(e) = vault.store_secret("GITHUB_COPILOT_SESSION", &session_data.to_string()) {
        warn!(error = %e, "Failed to cache imported token in vault");
    }

    Some(CopilotSession::from_session_token(
        token.to_string(),
        expires_at,
    ))
}

/// Initialize a CopilotSession if the provider requires one.
///
/// Checks multiple sources in order:
/// 1. Imported session token from vault (GITHUB_COPILOT_SESSION)
/// 2. Fresh token from OpenClaw (~/.openclaw/credentials/github-copilot.token.json)
/// 3. OAuth token from model context
async fn init_copilot_session(
    provider: &str,
    api_key: Option<&str>,
    vault: &SharedVault,
) -> Option<Arc<CopilotSession>> {
    if !crate_providers::needs_copilot_session(provider) {
        return None;
    }

    let mut vault_guard = vault.lock().await;
    let session_result = vault_guard.get_secret("GITHUB_COPILOT_SESSION", true);

    let mut session_from_import = match &session_result {
        Ok(Some(json_str)) => {
            debug!("Found GITHUB_COPILOT_SESSION in vault");
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
                    debug!(remaining_seconds = remaining, "Session expiry check");

                    if remaining > 60 {
                        Some(CopilotSession::from_session_token(token, expires_at))
                    } else {
                        debug!("Session expired or expiring soon");
                        None
                    }
                })
        }
        Ok(None) => {
            debug!("GITHUB_COPILOT_SESSION not found in vault");
            None
        }
        Err(e) => {
            warn!(error = %e, "Failed to read GITHUB_COPILOT_SESSION");
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
        debug!("Using imported session token");
        Some(Arc::new(session))
    } else if let Some(oauth) = api_key {
        debug!("Falling back to OAuth token");
        Some(Arc::new(CopilotSession::new(oauth.to_string())))
    } else {
        warn!("No OAuth token available for Copilot provider");
        None
    }
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
    task_mgr: Option<SharedTaskManager>,
    model_registry: Option<SharedModelRegistry>,
    observer: Option<SharedObserver>,
    cancel: CancellationToken,
) -> Result<()> {
    // Create task manager if not provided
    let task_mgr = task_mgr.unwrap_or_else(|| Arc::new(rustyclaw_core::tasks::TaskManager::new()));

    // Create model registry if not provided
    let model_registry =
        model_registry.unwrap_or_else(rustyclaw_core::models::create_model_registry);

    // Populate the registry from the configured provider's live model
    // list so the catalog is a single source of truth (same data the
    // `/model` slash command and onboarding see).
    if let Some(ref ctx) = model_ctx {
        let base = if ctx.base_url.is_empty() {
            None
        } else {
            Some(ctx.base_url.as_str())
        };
        let mut reg = model_registry.write().await;
        match reg
            .populate_from_provider(&ctx.provider, ctx.api_key.as_deref(), base)
            .await
        {
            Ok(n) => {
                tracing::info!(
                    target: "rustyclaw::models",
                    provider = %ctx.provider,
                    count = n,
                    "Populated model registry from provider"
                );
                // Mark the configured model as active (if present).
                if !ctx.model.is_empty() {
                    let qualified = if ctx.model.starts_with(&format!("{}/", ctx.provider)) {
                        ctx.model.clone()
                    } else {
                        format!("{}/{}", ctx.provider, ctx.model)
                    };
                    let _ = reg.set_active(&qualified);
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "rustyclaw::models",
                    provider = %ctx.provider,
                    error = %format!("{:#}", e),
                    "Failed to populate model registry from provider; registry will be empty until a successful fetch"
                );
            }
        }
    }

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

    // SSH-only transport: websocket listen/TLS options are ignored.

    // Initialize Copilot session if needed (uses the new helper function)
    let copilot_session: Option<Arc<CopilotSession>> = if let Some(ref ctx) = model_ctx {
        init_copilot_session(&ctx.provider, ctx.api_key.as_deref(), &vault).await
    } else {
        None
    };
    // Wrap in shared type so it can be updated when models change
    let shared_copilot_session: SharedCopilotSession =
        Arc::new(RwLock::new(copilot_session.clone()));

    let model_ctx = model_ctx.map(Arc::new);

    // Store model info in global runtime context for tool access
    if let Some(ref ctx) = model_ctx {
        rustyclaw_core::runtime_ctx::set_model_info(&ctx.provider, &ctx.model, &ctx.base_url);
    }

    let shared_config: SharedConfig = Arc::new(RwLock::new(config.clone()));
    let shared_model_ctx: SharedModelCtx = Arc::new(RwLock::new(model_ctx.clone()));
    let rate_limiter = auth::new_rate_limiter();

    if options.ssh_stdio {
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("SSH_USER"))
            .ok();
        let transport = Box::new(StdioTransport::new(username));

        info!("Gateway running in SSH stdio mode");
        return handle_transport_connection(
            transport,
            shared_config,
            shared_model_ctx,
            shared_copilot_session,
            vault,
            skill_mgr,
            task_mgr,
            model_registry,
            observer,
            rate_limiter,
            cancel,
        )
        .await;
    }

    // ── Initialize and start messenger loop ─────────────────────────
    //
    // If messengers are configured, we poll them for incoming messages
    // and route them through the model.
    eprintln!("DEBUG: messengers configured: {}", config.messengers.len());
    let messenger_mgr = if !config.messengers.is_empty() {
        eprintln!("DEBUG: Creating messenger manager...");
        match messenger_handler::create_messenger_manager(&config).await {
            Ok(mgr) => {
                eprintln!("DEBUG: Messenger manager created successfully");
                let shared_mgr: SharedMessengerManager = Arc::new(Mutex::new(mgr));

                // Spawn messenger loop
                let messenger_config = config.clone();
                let messenger_ctx = model_ctx.clone();
                let messenger_vault = vault.clone();
                let messenger_skills = skill_mgr.clone();
                let messenger_tasks = task_mgr.clone();
                let messenger_models = model_registry.clone();
                let messenger_cancel = cancel.child_token();
                let mgr_clone = shared_mgr.clone();
                // Read current copilot session from shared state
                let messenger_copilot = shared_copilot_session.read().await.clone();

                eprintln!("DEBUG: Spawning messenger loop task...");
                tokio::spawn(async move {
                    eprintln!("DEBUG: Messenger loop task started");
                    eprintln!(
                        "DEBUG: messenger_ctx.is_some() = {}",
                        messenger_ctx.is_some()
                    );
                    if let Err(e) = messenger_handler::run_messenger_loop(
                        messenger_config,
                        mgr_clone,
                        messenger_ctx,
                        messenger_vault,
                        messenger_skills,
                        messenger_tasks,
                        messenger_models,
                        messenger_copilot,
                        messenger_cancel,
                    )
                    .await
                    {
                        eprintln!("DEBUG: Messenger loop error: {}", e);
                        error!(error = %e, "Messenger loop error");
                    }
                    eprintln!("DEBUG: Messenger loop exited");
                });

                Some(shared_mgr)
            }
            Err(e) => {
                error!(error = %e, "Failed to initialize messengers");
                None
            }
        }
    } else {
        None
    };

    // Determine SSH listen address from CLI option or config.
    let ssh_listen = options
        .ssh_listen
        .clone()
        .or_else(|| {
            config.ssh.as_ref().and_then(|ssh_cfg| {
                if ssh_cfg.enabled && ssh_cfg.mode == "standalone" {
                    Some(ssh_cfg.bind.clone())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "0.0.0.0:2222".to_string());

    let bind_addr: SocketAddr = ssh_listen
        .parse()
        .with_context(|| format!("Invalid SSH listen address: {}", ssh_listen))?;

    let ssh_cfg = SshConfig {
        listen_addr: bind_addr,
        host_key_path: options
            .ssh_host_key
            .clone()
            .or_else(|| {
                config
                    .ssh
                    .as_ref()
                    .map(|s| s.host_key_path(&config.settings_dir))
            })
            .unwrap_or_else(|| config.settings_dir.join("ssh_host_key")),
        authorized_clients_path: options
            .ssh_authorized_clients
            .clone()
            .or_else(|| {
                config
                    .ssh
                    .as_ref()
                    .map(|s| s.authorized_keys_path(&config.settings_dir))
            })
            .unwrap_or_else(|| config.settings_dir.join("authorized_clients")),
        allow_password: false,
        require_pubkey: true,
        allow_unknown_keys_with_totp: config.totp_enabled,
    };

    let mut ssh_server = SshServer::new(ssh_cfg).await?;
    ssh_server.listen(bind_addr).await?;

    info!(address = %bind_addr, "Gateway listening (SSH-only)");
    if messenger_mgr.is_some() {
        info!("Messenger polling enabled");
    }

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = ssh_server.accept() => {
                match accepted {
                    Ok(transport) => {
                        let peer_info = transport.peer_info().clone();
                        info!(
                            transport = %peer_info.transport_type,
                            user = ?peer_info.username,
                            fingerprint = ?peer_info.key_fingerprint,
                            "SSH connection accepted"
                        );

                        let shared_cfg = shared_config.clone();
                        let shared_ctx = shared_model_ctx.clone();
                        let shared_session = shared_copilot_session.clone();
                        let vault_clone = vault.clone();
                        let skill_clone = skill_mgr.clone();
                        let task_mgr_clone = task_mgr.clone();
                        let model_reg_clone = model_registry.clone();
                        let observer_clone = observer.clone();
                        let rate_limiter_clone = rate_limiter.clone();
                        let child_cancel = cancel.child_token();

                        tokio::spawn(async move {
                            if let Err(err) = handle_transport_connection(
                                transport,
                                shared_cfg,
                                shared_ctx,
                                shared_session,
                                vault_clone,
                                skill_clone,
                                task_mgr_clone,
                                model_reg_clone,
                                observer_clone,
                                rate_limiter_clone,
                                child_cancel,
                            ).await {
                                debug!(error = %err, "SSH connection error");
                            }
                        });
                    }
                    Err(e) => warn!(error = %e, "SSH accept error"),
                }
            }
        }
    }

    Ok(())
}

/// Handle a connection using the Transport trait.
///
/// This is the transport-agnostic connection handler that works with any
/// transport implementation (SSH, stdio, future transports). For SSH
/// connections, authentication is already completed at the transport layer
/// via public key, so we skip TOTP.
async fn handle_transport_connection(
    transport: Box<dyn Transport>,
    shared_config: SharedConfig,
    shared_model_ctx: SharedModelCtx,
    shared_copilot_session: SharedCopilotSession,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    task_mgr: SharedTaskManager,
    model_registry: SharedModelRegistry,
    observer: Option<SharedObserver>,
    rate_limiter: auth::RateLimiter,
    cancel: CancellationToken,
) -> Result<()> {
    handle_connection(
        transport,
        shared_config,
        shared_model_ctx,
        shared_copilot_session,
        vault,
        skill_mgr,
        task_mgr,
        model_registry,
        observer,
        rate_limiter,
        cancel,
    )
    .await
}

async fn handle_connection(
    conn: Box<dyn transport::Transport>,
    shared_config: SharedConfig,
    shared_model_ctx: SharedModelCtx,
    shared_copilot_session: SharedCopilotSession,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    task_mgr: SharedTaskManager,
    model_registry: SharedModelRegistry,
    observer: Option<SharedObserver>,
    rate_limiter: auth::RateLimiter,
    cancel: CancellationToken,
) -> Result<()> {
    let peer_info = conn.peer_info().clone();
    let (mut reader, mut writer) = conn.into_split();
    let peer_ip = peer_info.addr.map(|a| a.ip());

    // Snapshot config and model context for this connection.
    // Reload updates the shared state; new connections pick up changes.
    let mut config = shared_config.read().await.clone();
    let model_ctx = shared_model_ctx.read().await.clone();

    // Thread manager for multi-task conversations.
    // Load from persistent storage or create new with default "Main" thread.
    let threads_path = config.sessions_dir().join("threads.json");
    let mut thread_mgr = rustyclaw_core::threads::ThreadManager::load_or_default(&threads_path);

    // Subscribe to thread events for push-based sidebar updates
    let mut thread_events_rx = thread_mgr.subscribe();

    // ── TOTP authentication challenge ───────────────────────────────
    //
    // If TOTP 2FA is enabled, require it for every transport.
    // SSH public-key auth is necessary but not sufficient.
    if config.totp_enabled {
        // Rate limiting requires a peer IP.
        let rate_ip = match peer_ip {
            Some(ip) => ip,
            None => {
                warn!("TOTP required but no peer IP available");
                writer.close().await?;
                return Ok(());
            }
        };

        // Check rate limit first.
        if let Some(remaining) = auth::check_rate_limit(&rate_limiter, rate_ip).await {
            send_frame(
                &mut *writer,
                &ServerFrame {
                    frame_type: ServerFrameType::AuthLocked,
                    payload: ServerPayload::AuthLocked {
                        message: format!("Too many failed attempts. Try again in {}s.", remaining),
                        retry_after: Some(remaining),
                    },
                },
            )
            .await?;
            writer.close().await?;
            return Ok(());
        }

        // Send challenge.
        protocol::server::send_auth_challenge(&mut *writer, "totp")
            .await
            .context("Failed to send auth_challenge")?;

        // Allow up to 3 attempts before closing the connection.
        const MAX_TOTP_ATTEMPTS: u8 = 3;
        let mut attempts = 0u8;

        loop {
            // Wait for auth_response (with a timeout).
            let auth_result = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                auth::wait_for_auth_response(&mut *reader),
            )
            .await;

            match auth_result {
                Ok(Ok(code)) => {
                    let valid = {
                        let mut v = vault.lock().await;
                        match v.verify_totp(code.trim()) {
                            Ok(result) => result,
                            Err(e) => {
                                warn!(error = %e, "TOTP verification error (vault issue?)");
                                false
                            }
                        }
                    };
                    if valid {
                        auth::clear_rate_limit(&rate_limiter, rate_ip).await;
                        protocol::server::send_auth_result(&mut *writer, true, None, None).await?;
                        break; // Authentication successful, continue to main loop
                    } else {
                        attempts += 1;
                        let locked_out = auth::record_totp_failure(&rate_limiter, rate_ip).await;

                        if locked_out {
                            let msg = format!(
                                "Invalid code. Too many failures — locked out for {}s.",
                                TOTP_LOCKOUT_SECS,
                            );
                            protocol::server::send_auth_result(
                                &mut *writer,
                                false,
                                Some(&msg),
                                None,
                            )
                            .await?;
                            writer.close().await?;
                            return Ok(());
                        } else if attempts >= MAX_TOTP_ATTEMPTS {
                            let msg = "Invalid code. Maximum attempts exceeded.";
                            protocol::server::send_auth_result(
                                &mut *writer,
                                false,
                                Some(msg),
                                None,
                            )
                            .await?;
                            writer.close().await?;
                            return Ok(());
                        } else {
                            let remaining = MAX_TOTP_ATTEMPTS - attempts;
                            let msg = format!(
                                "Invalid 2FA code. {} attempt{} remaining.",
                                remaining,
                                if remaining == 1 { "" } else { "s" }
                            );
                            protocol::server::send_auth_result(
                                &mut *writer,
                                false,
                                Some(&msg),
                                Some(true),
                            )
                            .await?;
                            // Continue loop to allow retry
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!(peer = ?peer_info.addr, error = %e, "Authentication error");
                    return Ok(());
                }
                Err(_) => {
                    protocol::server::send_auth_result(
                        &mut *writer,
                        false,
                        Some("Authentication timed out."),
                        None,
                    )
                    .await?;
                    writer.close().await?;
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
        &mut *writer,
        &config.agent_name,
        &config.settings_dir.to_string_lossy(),
        vault_is_locked,
        model_ctx.as_ref().map(|c| c.provider.as_str()),
        model_ctx.as_ref().map(|c| c.model.as_str()),
    )
    .await
    .context("Failed to send hello message")?;

    if vault_is_locked {
        protocol::server::send_status(
            &mut *writer,
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
            protocol::server::send_status(&mut *writer, StatusType::ModelConfigured, &detail)
                .await
                .context("Failed to send model_configured status")?;

            // 2. Credentials
            if ctx.api_key.is_some() {
                protocol::server::send_status(
                    &mut *writer,
                    StatusType::CredentialsLoaded,
                    &format!("{} API key loaded", display),
                )
                .await
                .context("Failed to send credentials_loaded status")?;
            } else if crate_providers::secret_key_for_provider(&ctx.provider).is_some()
                && crate_providers::provider_by_id(&ctx.provider).map(|p| p.auth_method)
                    != Some(crate_providers::AuthMethod::OptionalApiKey)
            {
                protocol::server::send_status(
                    &mut *writer,
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
                &mut *writer,
                StatusType::ModelConnecting,
                &format!("Probing {} …", ctx.base_url),
            )
            .await
            .context("Failed to send model_connecting status")?;

            // Read current copilot session from shared state
            let copilot_session = shared_copilot_session.read().await.clone();

            match providers::validate_model_connection(
                &http,
                &probe_ctx,
                copilot_session.as_deref(),
            )
            .await
            {
                ProbeResult::Ready => {
                    protocol::server::send_status(
                        &mut *writer,
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
                        &mut *writer,
                        StatusType::ModelReady,
                        &format!("{} / {} connected (probe: {})", display, ctx.model, warning),
                    )
                    .await
                    .context("Failed to send model_ready status")?;
                }
                ProbeResult::AuthError { detail } => {
                    protocol::server::send_status(
                        &mut *writer,
                        StatusType::ModelError,
                        &format!("{} auth failed: {}", display, detail),
                    )
                    .await
                    .context("Failed to send model_error status")?;
                }
                ProbeResult::Unreachable { detail } => {
                    protocol::server::send_status(
                        &mut *writer,
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
                &mut *writer,
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
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<WireFrame<ClientFrame>>(32);

    // Channel for tool-approval responses (used by the Ask permission flow).
    let (approval_tx, approval_rx) = tokio::sync::mpsc::channel::<(String, bool)>(4);
    let approval_rx = Arc::new(Mutex::new(approval_rx));

    // Channel for user-prompt responses (used by the ask_user tool).
    let (user_prompt_tx, user_prompt_rx) = tokio::sync::mpsc::channel::<(
        String,
        bool,
        rustyclaw_core::user_prompt_types::PromptResponseValue,
    )>(4);
    let user_prompt_rx = Arc::new(Mutex::new(user_prompt_rx));

    // Channel for credential responses (used when auth fails mid-conversation).
    let (credential_tx, credential_rx) =
        tokio::sync::mpsc::channel::<(String, bool, Option<String>)>(4);
    let credential_rx = Arc::new(Mutex::new(credential_rx));

    // Channel for DOM query responses (used by the client_dom_query tool).
    let (dom_query_tx, dom_query_rx) = tokio::sync::mpsc::channel::<(String, String, bool)>(4);
    let dom_query_rx = Arc::new(Mutex::new(dom_query_rx));

    // Channel for model task responses (concurrent execution).
    let (_model_task_tx, mut model_task_rx) = concurrent::channel();

    // Track active model tasks per thread.
    let mut active_tasks = concurrent::ActiveTasks::new();

    // ── Send initial thread list ───────────────────────────────────
    // Freshly-connected clients need to know the current thread state.
    if let Err(e) = send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await {
        warn!(error = %e, "Failed to send initial thread list");
    }

    let reader_cancel = cancel.clone();
    let reader_tool_cancel = tool_cancel.clone();
    let reader_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = reader_cancel.cancelled() => break,
                result = reader.recv() => {
                    match result {
                        Ok(Some(envelope)) => {
                            let stream_id = envelope.stream_id;
                            let frame = envelope.frame.clone();
                            trace!(stream_id, frame_type = ?frame.frame_type, "Received client frame");
                            // Intercept cancel, approval, and prompt responses
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
                            if frame.frame_type == ClientFrameType::CredentialResponse {
                                if let ClientPayload::CredentialResponse { id, dismissed, value } = frame.payload {
                                    let _ = credential_tx.send((id, dismissed, value)).await;
                                    continue;
                                }
                            }
                            if frame.frame_type == ClientFrameType::DomQueryResponse {
                                if let ClientPayload::DomQueryResponse { id, result, is_error } = frame.payload {
                                    let _ = dom_query_tx.send((id, result, is_error)).await;
                                    continue;
                                }
                            }
                            // Forward all other frames to the main loop
                            if frame_tx.send(envelope).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => break, // Clean disconnect
                        Err(e) => {
                            trace!(error = %e, "Error reading from transport");
                            break;
                        }
                    }
                }
            }
        }
    });

    // Main message handling loop — receives from channel
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = writer.close().await;
                break;
            }
            msg = frame_rx.recv() => {
                let envelope = match msg {
                    Some(f) => f,
                    None => break, // Channel closed (reader exited)
                };
                let stream_id = envelope.stream_id;
                let frame = envelope.frame;

                trace!(stream_id, frame_type = ?frame.frame_type, "Handling client frame");
                // Reset cancel flag for new request
                tool_cancel.store(false, Ordering::Relaxed);

                        // Handle the frame based on type
                        match frame.payload {
                            ClientPayload::UnlockVault { password } => {
                                let mut v = vault.lock().await;
                                v.set_password(password);
                                match v.get_secret("__vault_check__", true) {
                                    Ok(_) => {
                                        send_vault_unlocked(&mut *writer, true, None).await?;
                                    }
                                    Err(e) => {
                                        v.clear_password();
                                        send_vault_unlocked(
                                            &mut *writer,
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
                                        kind: entry.kind.to_string(),
                                        policy: entry.policy.badge().to_string(),
                                        disabled: entry.disabled,
                                    })
                                    .collect();
                                send_secrets_list_result(&mut *writer, true, dto_entries).await?;
                            }
                            ClientPayload::SecretsStore { key, value } => {
                                let mut v = vault.lock().await;
                                let result = v.store_secret(&key, &value);
                                match result {
                                    Ok(()) => send_secrets_store_result(
                                        &mut *writer,
                                        true,
                                        &format!("Secret '{}' stored.", key),
                                    ).await?,
                                    Err(e) => send_secrets_store_result(
                                        &mut *writer,
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
                                        send_secrets_get_result(&mut *writer, true, &key, Some(&value), None).await?
                                    }
                                    Ok(None) => {
                                        send_secrets_get_result(&mut *writer, false, &key, None, Some(&format!("Secret '{}' not found.", key))).await?
                                    }
                                    Err(e) => {
                                        send_secrets_get_result(&mut *writer, false, &key, None, Some(&format!("Failed to get secret: {}", e))).await?
                                    }
                                };
                            }
                            ClientPayload::SecretsDelete { key } => {
                                let mut v = vault.lock().await;
                                let result = v.delete_secret(&key);
                                match result {
                                    Ok(()) => send_secrets_delete_result(&mut *writer, true, None).await?,
                                    Err(e) => send_secrets_delete_result(
                                        &mut *writer,
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
                                        send_secrets_peek_result(&mut *writer, true, field_tuples, None)
                                            .await?
                                    }
                                    Err(e) => send_secrets_peek_result(
                                        &mut *writer,
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
                                    "always" => Some(rustyclaw_core::secrets::AccessPolicy::Always),
                                    "ask" => Some(rustyclaw_core::secrets::AccessPolicy::WithApproval),
                                    "auth" => Some(rustyclaw_core::secrets::AccessPolicy::WithAuth),
                                    "skill_only" => Some(rustyclaw_core::secrets::AccessPolicy::SkillOnly(skills)),
                                    _ => None,
                                };
                                if let Some(policy) = policy {
                                    let result = v.set_credential_policy(&name, policy);
                                    match result {
                                        Ok(()) => send_secrets_set_policy_result(&mut *writer, true, None).await?,
                                        Err(e) => send_secrets_set_policy_result(
                                            &mut *writer,
                                            false,
                                            Some(&format!("Failed to set policy: {}", e)),
                                        ).await?,
                                    }
                                } else {
                                    send_secrets_set_policy_result(
                                        &mut *writer,
                                        false,
                                        Some(&format!("Unknown policy: {}", policy_str)),
                                    ).await?;
                                }
                            }
                            ClientPayload::SecretsSetDisabled { name, disabled } => {
                                let mut v = vault.lock().await;
                                let result = v.set_credential_disabled(&name, disabled);
                                match result {
                                    Ok(()) => send_secrets_set_disabled_result(&mut *writer, true, None).await?,
                                    Err(e) => send_secrets_set_disabled_result(&mut *writer, false, Some(&format!("Failed: {}", e))).await?,
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
                                    Ok(()) => send_secrets_delete_credential_result(&mut *writer, true, None).await?,
                                    Err(e) => send_secrets_delete_credential_result(&mut *writer, false, Some(&format!("Failed: {}", e))).await?,
                                };
                            }
                            ClientPayload::SecretsHasTotp => {
                                let mut v = vault.lock().await;
                                let has_totp = v.has_totp();
                                send_secrets_has_totp_result(&mut *writer, has_totp).await?;
                            }
                            ClientPayload::SecretsSetupTotp => {
                                let mut v = vault.lock().await;
                                let result = v.setup_totp("rustyclaw");
                                match result {
                                    Ok(uri) => send_secrets_setup_totp_result(&mut *writer, true, Some(&uri), None).await?,
                                    Err(e) => send_secrets_setup_totp_result(&mut *writer, false, None, Some(&format!("Failed: {}", e))).await?,
                                };
                            }
                            ClientPayload::SecretsVerifyTotp { code } => {
                                let mut v = vault.lock().await;
                                let result = v.verify_totp(&code);
                                match result {
                                    Ok(valid) => send_secrets_verify_totp_result(&mut *writer, valid, None).await?,
                                    Err(e) => send_secrets_verify_totp_result(&mut *writer, false, Some(&format!("Error: {}", e))).await?,
                                };
                            }
                            ClientPayload::SecretsRemoveTotp => {
                                let mut v = vault.lock().await;
                                let result = v.remove_totp();
                                match result {
                                    Ok(()) => send_secrets_remove_totp_result(&mut *writer, true, None).await?,
                                    Err(e) => send_secrets_remove_totp_result(&mut *writer, false, Some(&format!("Failed: {}", e))).await?,
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

                                        // Reinitialize Copilot session if the new model needs it
                                        if let Some(ref ctx) = new_model_ctx {
                                            let new_session = init_copilot_session(
                                                &ctx.provider,
                                                ctx.api_key.as_deref(),
                                                &vault,
                                            ).await;
                                            let mut session = shared_copilot_session.write().await;
                                            *session = new_session;
                                        }

                                        // Refresh the model registry from the new provider so
                                        // the catalog matches the active connection.
                                        if let Some(ref ctx) = new_model_ctx {
                                            let base = if ctx.base_url.is_empty() {
                                                None
                                            } else {
                                                Some(ctx.base_url.as_str())
                                            };
                                            let mut reg = model_registry.write().await;
                                            if let Err(e) = reg
                                                .populate_from_provider(
                                                    &ctx.provider,
                                                    ctx.api_key.as_deref(),
                                                    base,
                                                )
                                                .await
                                            {
                                                tracing::warn!(
                                                    target: "rustyclaw::models",
                                                    provider = %ctx.provider,
                                                    error = %format!("{:#}", e),
                                                    "Failed to refresh model registry after Reload"
                                                );
                                            } else if !ctx.model.is_empty() {
                                                let qualified = if ctx
                                                    .model
                                                    .starts_with(&format!("{}/", ctx.provider))
                                                {
                                                    ctx.model.clone()
                                                } else {
                                                    format!("{}/{}", ctx.provider, ctx.model)
                                                };
                                                let _ = reg.set_active(&qualified);
                                            }
                                        }

                                        {
                                            let mut cfg = shared_config.write().await;
                                            *cfg = new_config;
                                        }
                                        {
                                            let mut ctx = shared_model_ctx.write().await;
                                            *ctx = new_model_ctx.clone();
                                        }

                                        send_reload_result(&mut *writer, true, &provider, &model, None).await?;

                                        if let Some(ref ctx) = new_model_ctx {
                                            let display = crate_providers::display_name_for_provider(&ctx.provider);
                                            let detail = format!("{} / {} (reloaded)", display, ctx.model);
                                            protocol::server::send_status(
                                                &mut *writer,
                                                StatusType::ModelConfigured,
                                                &detail,
                                            ).await?;
                                        }
                                    }
                                    Err(e) => {
                                        protocol::server::send_error(
                                            &mut *writer,
                                            &format!("Failed to reload config: {}", e),
                                        ).await?;
                                    }
                                }
                            }
                            ClientPayload::Chat { messages } => {
                                // Check for auto-switch: find better matching thread
                                if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
                                    if let Some(better_thread_id) = thread_mgr.find_best_match(&last_user.content) {
                                        // Found a better match — switch threads
                                        if thread_mgr.switch_foreground(better_thread_id) {
                                            // Get the context summary from the new foreground thread
                                            let context_summary = thread_mgr
                                                .foreground()
                                                .and_then(|t| t.compact_summary.clone());
                                            // Send ThreadSwitched notification
                                            let frame = ServerFrame {
                                                frame_type: ServerFrameType::ThreadSwitched,
                                                payload: ServerPayload::ThreadSwitched {
                                                    thread_id: better_thread_id.0,
                                                    context_summary,
                                                },
                                            };
                                            send_frame(&mut *writer, &frame).await?;
                                            // Update thread list
                                            send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                                            send_thread_messages_update(&mut *writer, better_thread_id, &thread_mgr).await?;
                                        }
                                    }
                                }

                                // Add user message to current thread's history
                                let mut did_auto_label = false;
                                let mut needs_caption = false;
                                let mut did_append_user_message = false;
                                let mut active_thread_id = None;
                                if let Some(thread) = thread_mgr.foreground_mut() {
                                    active_thread_id = Some(thread.id);
                                    // Find the last user message (typically the new one)
                                    if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
                                        // Check if this is the first message in a new thread
                                        let is_first_message = thread.message_count() == 0
                                            && (thread.label.is_empty()
                                                || thread.label.starts_with("Session #")
                                                || thread.label == "Main");
                                        thread.add_message(
                                            rustyclaw_core::threads::MessageRole::User,
                                            &last_user.content,
                                        );
                                        did_append_user_message = true;
                                        if is_first_message {
                                            // Set a temporary auto-label as fallback
                                            let label = auto_thread_label(&last_user.content);
                                            thread.label = label;
                                            did_auto_label = true;
                                            // Flag for agent captioning
                                            needs_caption = true;
                                        }
                                    }
                                }
                                if did_append_user_message
                                    && let Err(e) = thread_mgr.save_to_file(&threads_path)
                                {
                                    warn!(error = %e, path = ?threads_path, "Failed to persist user message to thread history");
                                }
                                if did_auto_label {
                                    send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                                }

                                // Auto-ingest user message into Steel Memory
                                if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
                                    let ws = config.workspace_dir().to_path_buf();
                                    let text = last_user.content.clone();
                                    tokio::spawn(async move {
                                        if let Ok(mem) = rustyclaw_core::steel_memory::SteelMemory::new(&ws) {
                                            let _ = mem.add_memory(&text, "conversations", "user", None).await;
                                        }
                                    });
                                }
                                if let Some(thread_id) = active_thread_id {
                                    send_thread_messages_update(&mut *writer, thread_id, &thread_mgr).await?;
                                }

                                // Re-read model_ctx from shared state for each dispatch
                                let current_model_ctx = shared_model_ctx.read().await.clone();
                                // Re-read copilot session from shared state
                                let copilot_session = shared_copilot_session.read().await.clone();
                                let workspace_dir = config.workspace_dir();

                                // Ensure a system prompt is present. The TUI
                                // sends the full conversation (including a
                                // system message), but the desktop client
                                // only sends the user message. When missing,
                                // build one from the workspace context so
                                // that SOUL.md, IDENTITY.md, etc. are
                                // included.
                                let mut messages = messages;
                                let client_sent_history = !messages.is_empty() && messages[0].role == "system";
                                if !client_sent_history {
                                    let sys = system_prompt::build_system_prompt(
                                        &config,
                                        &task_mgr,
                                        &skill_mgr,
                                    ).await;
                                    messages.insert(0, ChatMessage::text("system", &sys));

                                    // Inject conversation history from the
                                    // thread. The desktop client only sends
                                    // the current user message; we need to
                                    // include prior turns so the model has
                                    // context of the conversation.
                                    if let Some(thread) = thread_mgr.foreground() {
                                        let history = &thread.messages;
                                        // history includes the message we just
                                        // added — skip it (last element) to
                                        // avoid duplication with the client's
                                        // user message already in `messages`.
                                        let prior_count = history.len().saturating_sub(1);
                                        if prior_count > 0 {
                                            // Optionally include compact summary as context
                                            if let Some(summary) = &thread.compact_summary {
                                                messages.insert(1, ChatMessage::text(
                                                    "system",
                                                    &format!("# Previous conversation summary\n\n{}", summary),
                                                ));
                                            }
                                            let insert_pos = if thread.compact_summary.is_some() { 2 } else { 1 };
                                            // Reconstruct the history with structured
                                            // tool_call / tool_result payloads so that
                                            // assistant messages keep their `tool_calls`
                                            // and following tool results stay anchored
                                            // to them. Flattening to plain text would
                                            // produce orphan `tool` messages that the
                                            // provider rejects.
                                            let provider_name = current_model_ctx
                                                .as_deref()
                                                .map(|c| c.provider.as_str())
                                                .unwrap_or("openai");
                                            let history_slice: Vec<rustyclaw_core::threads::ThreadMessage> = history
                                                .iter()
                                                .take(prior_count)
                                                .cloned()
                                                .collect();
                                            let history_msgs: Vec<ChatMessage> =
                                                providers::thread_history_to_chat_messages(
                                                    provider_name,
                                                    &history_slice,
                                                );
                                            // Insert history between system prompt and current user message
                                            let tail = messages.split_off(insert_pos);
                                            messages.extend(history_msgs);
                                            messages.extend(tail);
                                        }
                                    }
                                }

                                // Inject thread context into system prompt if available
                                let mut messages_with_context = {
                                    let global_ctx = thread_mgr.build_global_context();
                                    let provider_name = current_model_ctx
                                        .as_deref()
                                        .map(|c| c.provider.as_str())
                                        .unwrap_or("openai");
                                    let mut msgs = active_thread_id
                                        .and_then(|thread_id| {
                                            thread_mgr.get(thread_id).map(|thread| {
                                                let history: Vec<rustyclaw_core::threads::ThreadMessage> =
                                                    thread.messages.iter().cloned().collect();
                                                providers::thread_history_to_chat_messages(
                                                    provider_name,
                                                    &history,
                                                )
                                            })
                                        })
                                        .unwrap_or_else(|| messages.clone());
                                    if let Some(system_message) = messages.first().filter(|m| m.role == "system") {
                                        if msgs.first().map(|m| m.role.as_str()) != Some("system") {
                                            msgs.insert(0, system_message.clone());
                                        }
                                    }
                                    if !global_ctx.is_empty() && !msgs.is_empty() && msgs[0].role == "system" {
                                        msgs[0].content = format!(
                                            "{}\n\n# Background Tasks\n\n{}",
                                            msgs[0].content,
                                            global_ctx
                                        );
                                        msgs
                                    } else {
                                        msgs
                                    }
                                };

                                // Inject captioning instruction for new threads
                                if needs_caption && !messages_with_context.is_empty() && messages_with_context[0].role == "system" {
                                    messages_with_context[0].content = format!(
                                        "{}\n\n## Thread Captioning\n\
                                        This is the first message in a new conversation thread. \
                                        After responding, call `set_thread_caption` with a short \
                                        2-6 word caption that summarises the topic of this conversation.",
                                        messages_with_context[0].content
                                    );
                                }

                                // Inject relevant memory context from Steel Memory
                                if !messages_with_context.is_empty() && messages_with_context[0].role == "system" {
                                    if let Some(last_user) = messages_with_context.iter().rev().find(|m| m.role == "user") {
                                        let query = last_user.content.clone();
                                        let ws = config.workspace_dir().to_path_buf();
                                        if let Ok(mem) = rustyclaw_core::steel_memory::SteelMemory::new(&ws) {
                                            if let Ok(results) = mem.search(&query, 3, Some(0.4)).await {
                                                if !results.is_empty() {
                                                    let mut ctx = String::from("\n\n## Relevant Memories\n");
                                                    for r in &results {
                                                        let snippet = if r.content.len() > 300 {
                                                            format!("{}…", &r.content[..300])
                                                        } else {
                                                            r.content.clone()
                                                        };
                                                        ctx.push_str(&format!(
                                                            "- (similarity {:.2}) {}\n",
                                                            r.similarity, snippet
                                                        ));
                                                    }
                                                    messages_with_context[0].content.push_str(&ctx);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Build a ChatRequest from the messages
                                let chat_request = ChatRequest {
                                    msg_type: "chat".to_string(),
                                    messages: messages_with_context,
                                    model: None,
                                    provider: None,
                                    base_url: None,
                                    api_key: None,
                                };

                                let mut stream_writer = ScopedTransportWriter::new(&mut *writer, stream_id);
                                if let Err(err) = dispatch_text_message(
                                    &http,
                                    &chat_request,
                                    current_model_ctx.as_deref(),
                                    copilot_session.as_deref(),
                                    &mut stream_writer,
                                    &workspace_dir,
                                    &vault,
                                    &skill_mgr,
                                    &task_mgr,
                                    observer.as_ref(),
                                    &tool_cancel,
                                    &shared_config,
                                    &shared_copilot_session,
                                    &approval_rx,
                                    &user_prompt_rx,
                                    &credential_rx,
                                    &dom_query_rx,
                                    &mut thread_mgr,
                                    &threads_path,
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
                                    send_frame(&mut stream_writer, &error_frame).await?;
                                }
                            }
                            ClientPayload::TasksRequest { session } => {
                                // Build task list and send back
                                let tasks = if let Some(ref sess) = session {
                                    task_mgr.for_session(sess).await
                                } else {
                                    task_mgr.active().await
                                };
                                let dto_tasks: Vec<protocol::TaskInfoDto> = tasks
                                    .iter()
                                    .map(|t| protocol::TaskInfoDto {
                                        id: t.id.0,
                                        label: t.display_label(),
                                        description: t.description.clone(),
                                        status: format!("{:?}", t.status)
                                            .split('{')
                                            .next()
                                            .unwrap_or("Unknown")
                                            .trim()
                                            .to_string(),
                                        is_foreground: t.status.is_foreground(),
                                    })
                                    .collect();
                                let frame = ServerFrame {
                                    frame_type: ServerFrameType::TasksUpdate,
                                    payload: ServerPayload::TasksUpdate { tasks: dto_tasks },
                                };
                                send_frame(&mut *writer, &frame).await?;
                            }
                            ClientPayload::ThreadCreate { label } => {
                                let label = if label.is_empty() {
                                    format!("Session #{}", thread_mgr.list().len() + 1)
                                } else {
                                    label
                                };
                                debug!("Thread create request: {}", label);
                                let thread_id = thread_mgr.create_thread(&label);
                                let frame = ServerFrame {
                                    frame_type: ServerFrameType::ThreadCreated,
                                    payload: ServerPayload::ThreadCreated {
                                        thread_id: thread_id.0,
                                        label,
                                    },
                                };
                                send_frame(&mut *writer, &frame).await?;
                                // Send updated thread list
                                send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                                // Persist thread state
                                let _ = thread_mgr.save_to_file(&threads_path);
                            }
                            ClientPayload::ThreadSwitch { thread_id } => {
                                debug!("Thread switch request: {}", thread_id);

                                // thread_id == 0 is a sentinel meaning "background current thread"
                                if thread_id == 0 {
                                    // Clear foreground — no thread is active
                                    thread_mgr.clear_foreground();
                                    let frame = ServerFrame {
                                        frame_type: ServerFrameType::ThreadSwitched,
                                        payload: ServerPayload::ThreadSwitched {
                                            thread_id: 0,
                                            context_summary: None,
                                        },
                                    };
                                    send_frame(&mut *writer, &frame).await?;
                                    send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                                    let frame = ServerFrame {
                                        frame_type: ServerFrameType::ThreadMessages,
                                        payload: ServerPayload::ThreadMessages {
                                            thread_id: 0,
                                            messages: Vec::new(),
                                        },
                                    };
                                    send_frame(&mut *writer, &frame).await?;
                                    let _ = thread_mgr.save_to_file(&threads_path);
                                    continue;
                                }

                                let target_id = rustyclaw_core::threads::ThreadId(thread_id);

                                // Get current foreground thread for compaction
                                let current_fg_id = thread_mgr.foreground().map(|t| t.task_id());

                                // Compact the current thread if it has messages
                                if let Some(fg_id) = current_fg_id {
                                    if fg_id != target_id {
                                        if let Some(thread) = thread_mgr.get_mut(fg_id) {
                                            if thread.messages.len() > 3 && thread.compact_summary.is_none() {
                                                // Generate compaction prompt
                                                let prompt = thread.compaction_prompt();

                                                // Notify client about compaction
                                                protocol::server::send_info(
                                                    &mut *writer,
                                                    &format!("Compacting thread '{}'...", thread.label),
                                                )
                                                .await?;

                                                // Call LLM to summarize
                                                let current_model_ctx = shared_model_ctx.read().await.clone();
                                                if let Some(ref ctx) = current_model_ctx {
                                                    let summary_req = ProviderRequest {
                                                        messages: vec![ChatMessage::text("user", &prompt)],
                                                        model: ctx.model.clone(),
                                                        provider: ctx.provider.clone(),
                                                        base_url: ctx.base_url.clone(),
                                                        api_key: ctx.api_key.clone(),
                                                    };

                                                    let summary_result = if ctx.provider == "anthropic" {
                                                        providers::call_anthropic_with_tools(&http, &summary_req, None).await
                                                    } else if ctx.provider == "google" {
                                                        providers::call_google_with_tools(&http, &summary_req).await
                                                    } else {
                                                        providers::call_openai_with_tools(&http, &summary_req, None).await
                                                    };

                                                    match summary_result {
                                                        Ok(resp) if !resp.text.is_empty() => {
                                                            thread.apply_compaction(resp.text);
                                                            debug!(thread = %thread.label, "Thread compacted");
                                                        }
                                                        Ok(_) => {
                                                            debug!(thread = %thread.label, "Empty summary from LLM");
                                                        }
                                                        Err(e) => {
                                                            debug!(thread = %thread.label, error = %e, "Compaction failed");
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Get summary of thread being switched to
                                let context_summary = thread_mgr
                                    .get(target_id)
                                    .and_then(|t| t.compact_summary.clone());

                                // Perform the switch (use switch_foreground which returns bool,
                                // not switch_to which returns old foreground ID — the latter
                                // returns None when there is no previous foreground, e.g. after /thread bg)
                                if thread_mgr.switch_foreground(target_id) {
                                    let frame = ServerFrame {
                                        frame_type: ServerFrameType::ThreadSwitched,
                                        payload: ServerPayload::ThreadSwitched {
                                            thread_id,
                                            context_summary,
                                        },
                                    };
                                    send_frame(&mut *writer, &frame).await?;
                                    // Send updated thread list
                                    send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                                    send_thread_messages_update(&mut *writer, target_id, &thread_mgr).await?;
                                    // Persist thread state (includes compaction summary)
                                    let _ = thread_mgr.save_to_file(&threads_path);
                                } else {
                                    let frame = ServerFrame {
                                        frame_type: ServerFrameType::Error,
                                        payload: ServerPayload::Error {
                                            ok: false,
                                            message: format!("Thread {} not found", thread_id),
                                        },
                                    };
                                    send_frame(&mut *writer, &frame).await?;
                                }
                            }
                            ClientPayload::ThreadList => {
                                debug!("Thread list request");
                                send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                                if let Some(thread) = thread_mgr.foreground() {
                                    send_thread_messages_update(&mut *writer, thread.id, &thread_mgr).await?;
                                }
                            }
                            ClientPayload::ThreadHistoryRequest { thread_id } => {
                                debug!("Thread history request: {}", thread_id);
                                let target_id = rustyclaw_core::threads::ThreadId(thread_id);
                                let (ok, messages, error) = match thread_mgr.get(target_id) {
                                    Some(thread) => {
                                        let wire: Vec<ChatMessage> = thread
                                            .messages
                                            .iter()
                                            .map(|m| {
                                                let role = match m.role {
                                                    rustyclaw_core::threads::MessageRole::User => "user",
                                                    rustyclaw_core::threads::MessageRole::Assistant => "assistant",
                                                    rustyclaw_core::threads::MessageRole::System => "system",
                                                    rustyclaw_core::threads::MessageRole::Tool => "tool",
                                                };
                                                ChatMessage {
                                                    role: role.to_string(),
                                                    content: m.content.clone(),
                                                    tool_calls: m.tool_calls.clone(),
                                                    tool_call_id: m.tool_call_id.clone(),
                                                    media: None,
                                                }
                                            })
                                            .collect();
                                        info!(
                                            thread_id,
                                            caption = %thread.label,
                                            message_count = wire.len(),
                                            "Gateway loaded thread history"
                                        );
                                        (true, wire, None)
                                    }
                                    None => (
                                        false,
                                        Vec::new(),
                                        Some(format!("Thread {} not found", thread_id)),
                                    ),
                                };
                                let frame = ServerFrame {
                                    frame_type: ServerFrameType::ThreadHistoryReply,
                                    payload: ServerPayload::ThreadHistoryReply {
                                        thread_id,
                                        ok,
                                        messages,
                                        error,
                                    },
                                };
                                debug!(thread_id, ok, "Sending ThreadHistoryReply");
                                send_frame(&mut *writer, &frame).await?;
                            }
                            ClientPayload::ThreadClose { thread_id } => {
                                debug!("Thread close request: {}", thread_id);
                                let task_id = rustyclaw_core::threads::ThreadId(thread_id);
                                thread_mgr.remove(task_id);
                                // Send updated thread list
                                send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                                // Persist thread state
                                let _ = thread_mgr.save_to_file(&threads_path);
                            }
                            ClientPayload::ThreadRename { thread_id, new_label } => {
                                debug!("Thread rename request: {} -> {}", thread_id, new_label);
                                let task_id = rustyclaw_core::threads::ThreadId(thread_id);
                                if thread_mgr.rename(task_id, &new_label) {
                                    // Send updated thread list
                                    send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                                    // Persist thread state
                                    let _ = thread_mgr.save_to_file(&threads_path);
                                } else {
                                    let frame = ServerFrame {
                                        frame_type: ServerFrameType::Error,
                                        payload: ServerPayload::Error {
                                            ok: false,
                                            message: format!("Thread {} not found", thread_id),
                                        },
                                    };
                                    send_frame(&mut *writer, &frame).await?;
                                }
                            }
                            ClientPayload::ModelSwitch { provider, model } => {
                                debug!("Model switch request: {} / {}", provider, model);
                                let base_url = crate_providers::base_url_for_provider(&provider)
                                    .unwrap_or("")
                                    .to_string();
                                let api_key = {
                                    let key_name = crate_providers::secret_key_for_provider(&provider);
                                    if let Some(name) = key_name {
                                        let mut v = vault.lock().await;
                                        v.get_secret(name, true)
                                            .ok()
                                            .flatten()
                                            .or_else(|| std::env::var(name).ok())
                                    } else {
                                        None
                                    }
                                };

                                let new_ctx = Arc::new(ModelContext {
                                    provider: provider.clone(),
                                    model: model.clone(),
                                    base_url,
                                    api_key: api_key.clone(),
                                });

                                // Reinitialize Copilot session if needed
                                let new_session = init_copilot_session(
                                    &provider,
                                    api_key.as_deref(),
                                    &vault,
                                ).await;
                                {
                                    let mut session = shared_copilot_session.write().await;
                                    *session = new_session;
                                }
                                {
                                    let mut ctx = shared_model_ctx.write().await;
                                    *ctx = Some(new_ctx);
                                }

                                // Also update the config so it persists across restarts
                                {
                                    let mut cfg = shared_config.write().await;
                                    let base = crate_providers::base_url_for_provider(&provider)
                                        .map(String::from);
                                    cfg.model = Some(rustyclaw_core::config::ModelProvider {
                                        provider: provider.clone(),
                                        model: Some(model.clone()),
                                        base_url: base,
                                    });
                                    let _ = cfg.save(None);
                                }

                                let display = crate_providers::display_name_for_provider(&provider);
                                send_reload_result(&mut *writer, true, &provider, &model, None).await?;
                                let detail = format!("{} / {}", display, model);
                                protocol::server::send_status(
                                    &mut *writer,
                                    StatusType::ModelConfigured,
                                    &detail,
                                ).await?;
                            }
                            ClientPayload::SetAgentName { name } => {
                                debug!("Agent name change: {}", name);
                                {
                                    let mut cfg = shared_config.write().await;
                                    cfg.agent_name = name.clone();
                                    let _ = cfg.save(None);
                                }
                                config.agent_name = name;
                            }
                            ClientPayload::SetWorkingDirectory { path } => {
                                debug!("Working directory change: {}", path);
                                let new_dir = std::path::PathBuf::from(&path);
                                config.workspace_dir = Some(new_dir.clone());
                                // Re-register sandbox with the new workspace dir so tool
                                // access controls apply to the new location.
                                let sandbox_mode = config.sandbox.mode.parse().unwrap_or_default();
                                tools::init_sandbox(
                                    sandbox_mode,
                                    new_dir,
                                    config.credentials_dir(),
                                    config.sandbox.deny_paths.clone(),
                                );
                            }
                            ClientPayload::Empty | ClientPayload::AuthChallenge { .. } | ClientPayload::AuthResponse { .. } | ClientPayload::ToolApprovalResponse { .. } | ClientPayload::UserPromptResponse { .. } | ClientPayload::CredentialResponse { .. } | ClientPayload::DomQueryResponse { .. } => {
                                // AuthChallenge/AuthResponse handled in auth phase.
                                // ToolApprovalResponse handled by the reader task.
                                // UserPromptResponse handled by the reader task.
                                // CredentialResponse handled by the reader task.
                                // DomQueryResponse handled by the reader task.
                            }
                        }
            }
            // Handle messages from spawned model tasks
            model_msg = model_task_rx.recv() => {
                if let Some(task_msg) = model_msg {
                    match task_msg {
                        concurrent::ModelTaskMessage::Frame(data) => {
                            // Deserialize and forward frame to client
                            if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                                send_frame(&mut *writer, &frame).await?;
                            }
                        }
                        concurrent::ModelTaskMessage::Done { thread_id, response } => {
                            // Task completed - remove from active tasks
                            active_tasks.remove(&thread_id);

                            // Record assistant response in thread history if provided
                            if let Some(text) = response {
                                if let Some(thread) = thread_mgr.get_mut(thread_id) {
                                    thread.add_message(rustyclaw_core::threads::MessageRole::Assistant, &text);
                                }
                                send_thread_messages_update(&mut *writer, thread_id, &thread_mgr).await?;
                            }

                            // Send updated thread list (status may have changed)
                            send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;

                            // Persist thread state
                            let _ = thread_mgr.save_to_file(&threads_path);
                        }
                        concurrent::ModelTaskMessage::Error { thread_id, message } => {
                            // Task failed - remove from active tasks
                            active_tasks.remove(&thread_id);

                            // Send error frame
                            let error_frame = ServerFrame {
                                frame_type: ServerFrameType::Error,
                                payload: ServerPayload::Error {
                                    ok: false,
                                    message,
                                },
                            };
                            send_frame(&mut *writer, &error_frame).await?;

                            // Send updated thread list
                            send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                        }
                    }
                }
            }
            // Handle thread events for push-based sidebar updates
            thread_event = thread_events_rx.recv() => {
                if let Ok(event) = thread_event {
                    // Only send updates for events that affect sidebar display
                    if event.triggers_sidebar_update() {
                        send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                    }
                }
            }
        }
    }

    // Clean up reader task
    reader_handle.abort();

    // Persist thread state on disconnect
    let _ = thread_mgr.save_to_file(&threads_path);

    Ok(())
}

/// Derive a short thread label from the first user message.
fn auto_thread_label(content: &str) -> String {
    let trimmed = content.trim();
    // Use the first line, capped at 50 chars on a word boundary.
    let first_line = trimmed.lines().next().unwrap_or(trimmed);
    if first_line.len() <= 50 {
        first_line.to_string()
    } else {
        match first_line[..50].rfind(' ') {
            Some(pos) if pos > 20 => format!("{}…", &first_line[..pos]),
            _ => format!("{}…", &first_line[..50]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rustyclaw_core::gateway::{PeerInfo, TransportReader, TransportType, TransportWriter};
    use rustyclaw_core::skills::SkillManager;
    use std::collections::VecDeque;
    use tempfile::tempdir;

    struct MockTransport {
        peer: PeerInfo,
        incoming: Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
        outgoing: Arc<Mutex<Vec<ServerFrame>>>,
    }

    struct MockReader {
        peer: PeerInfo,
        incoming: Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
    }

    struct MockWriter {
        outgoing: Arc<Mutex<Vec<ServerFrame>>>,
    }

    impl MockTransport {
        fn with_frames(
            peer: PeerInfo,
            frames: Vec<Option<ClientFrame>>,
        ) -> (Self, Arc<Mutex<Vec<ServerFrame>>>) {
            let outgoing = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    peer,
                    incoming: Arc::new(Mutex::new(VecDeque::from(frames))),
                    outgoing: outgoing.clone(),
                },
                outgoing,
            )
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        fn peer_info(&self) -> &PeerInfo {
            &self.peer
        }

        async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>> {
            Ok(self
                .incoming
                .lock()
                .await
                .pop_front()
                .unwrap_or(None)
                .map(WireFrame::control))
        }

        async fn send_on_stream(&mut self, _stream_id: u64, frame: &ServerFrame) -> Result<()> {
            self.outgoing.lock().await.push(frame.clone());
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn into_split(self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>) {
            (
                Box::new(MockReader {
                    peer: self.peer.clone(),
                    incoming: self.incoming.clone(),
                }),
                Box::new(MockWriter {
                    outgoing: self.outgoing.clone(),
                }),
            )
        }
    }

    #[async_trait]
    impl TransportReader for MockReader {
        async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>> {
            Ok(self
                .incoming
                .lock()
                .await
                .pop_front()
                .unwrap_or(None)
                .map(WireFrame::control))
        }

        fn peer_info(&self) -> &PeerInfo {
            &self.peer
        }
    }

    #[async_trait]
    impl TransportWriter for MockWriter {
        async fn send_on_stream(&mut self, _stream_id: u64, frame: &ServerFrame) -> Result<()> {
            self.outgoing.lock().await.push(frame.clone());
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn test_config_with_temp_state() -> Result<(tempfile::TempDir, Config)> {
        let tmp = tempdir()?;
        let cfg = Config {
            settings_dir: tmp.path().join("state"),
            ..Config::default()
        };

        std::fs::create_dir_all(cfg.settings_dir.clone())?;
        std::fs::create_dir_all(cfg.workspace_dir())?;
        std::fs::create_dir_all(cfg.credentials_dir())?;
        std::fs::create_dir_all(cfg.sessions_dir())?;
        std::fs::create_dir_all(cfg.skills_dir())?;

        Ok((tmp, cfg))
    }

    #[tokio::test]
    async fn ssh_connection_requires_totp_when_enabled() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = true;

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };

        // Disconnect immediately after first server write.
        let (mock_transport, outgoing) = MockTransport::with_frames(peer, vec![None]);

        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();

        handle_transport_connection(
            Box::new(mock_transport),
            Arc::new(RwLock::new(cfg)),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
            vault,
            skill_mgr,
            task_mgr,
            model_registry,
            None,
            auth::new_rate_limiter(),
            CancellationToken::new(),
        )
        .await?;

        let frames = outgoing.lock().await;
        assert!(
            frames
                .iter()
                .any(|f| matches!(f.frame_type, ServerFrameType::AuthChallenge)),
            "Expected TOTP auth challenge for SSH connection when totp_enabled=true"
        );

        Ok(())
    }

    #[tokio::test]
    async fn transport_connection_processes_chat_frames() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;

        let chat = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", "Hello?")],
            },
        };

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };

        let (mock_transport, outgoing) = MockTransport::with_frames(peer, vec![Some(chat), None]);

        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();

        handle_transport_connection(
            Box::new(mock_transport),
            Arc::new(RwLock::new(cfg)),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
            vault,
            skill_mgr,
            task_mgr,
            model_registry,
            None,
            auth::new_rate_limiter(),
            CancellationToken::new(),
        )
        .await?;

        let frames = outgoing.lock().await;
        assert!(
            frames
                .iter()
                .any(|f| matches!(f.frame_type, ServerFrameType::Hello)),
            "Expected hello frame"
        );
        assert!(
            frames
                .iter()
                .any(|f| matches!(f.frame_type, ServerFrameType::Error)),
            "Expected chat request to be processed and produce an error frame when model context is missing"
        );

        Ok(())
    }
}

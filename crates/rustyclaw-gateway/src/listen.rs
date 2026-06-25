//! Gateway listener / acceptor layer.
//!
//! [`run_gateway`] is the networked entry point: it bootstraps shared state
//! (model registry, copilot session, sandbox), optionally starts the messenger
//! loop, then accepts SSH (or stdio) transports and hands each one to the
//! per-connection engine in [`crate::server`]. Invoked from the binary entry
//! point in `main.rs`.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{
    CopilotSession, GatewayOptions, ModelContext, Transport, TransportAcceptor,
};
use rustyclaw_core::tools;

use crate::messenger_handler::SharedMessengerManager;
use crate::server::handle_connection;
use crate::session::init_copilot_session;
use crate::ssh::{SshConfig, SshServer, StdioTransport};
use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedModelRegistry, SharedObserver,
    SharedSkillManager, SharedTaskManager, SharedVault, auth, messenger_handler,
};

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

    // ── Host introspection & load tracking ─────────────────────────
    //
    // Detect hardware capabilities once, then start a background sampler
    // that periodically records CPU / memory load.  Both are stored in
    // the global runtime context so tools and peer-status queries can
    // access them without extra plumbing through every call site.
    let host_caps = rustyclaw_core::host::detect_host();
    info!(
        hostname = %host_caps.hostname,
        cpus = host_caps.cpu_cores_logical,
        ram_gb = host_caps.total_memory_bytes / (1024 * 1024 * 1024),
        gpus = host_caps.gpus.len(),
        "Host capabilities detected"
    );
    rustyclaw_core::runtime_ctx::set_host(host_caps);

    let load_tracker = rustyclaw_core::load::create_load_tracker();
    let _load_sampler_handle = rustyclaw_core::load::spawn_load_sampler(
        load_tracker.clone(),
        None, // use default 5 s interval
    );
    rustyclaw_core::runtime_ctx::set_load_tracker(load_tracker);

    // ── Managed services ────────────────────────────────────────────
    //
    // If the config has [services.*] entries, create a ServiceManager,
    // store it in the global runtime context, and auto-start any
    // services marked with `auto_start = true`.
    if !config.services.is_empty() {
        let svc_config = rustyclaw_core::services::ServicesConfig {
            services: config.services.clone(),
        };
        let svc_mgr = rustyclaw_core::services::create_service_manager(svc_config);
        info!(count = config.services.len(), "Managed services configured");
        // Auto-start services
        {
            let mut mgr = svc_mgr.write().await;
            mgr.auto_start_all().await;
        }
        // Spawn background poller for lifecycle management and health checks
        let _svc_poller_handle = rustyclaw_core::services::spawn_service_poller(
            svc_mgr.clone(),
            None, // use default 2 s interval
        );
        rustyclaw_core::runtime_ctx::set_service_manager(svc_mgr);
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

    // Graceful shutdown: stop all managed services.
    if let Some(svc_mgr) = rustyclaw_core::runtime_ctx::get_service_manager() {
        info!("Stopping managed services…");
        let mut mgr = svc_mgr.write().await;
        mgr.stop_all().await;
    }

    Ok(())
}

/// Handle a connection using the Transport trait.
///
/// This is the transport-agnostic connection handler that works with any
/// transport implementation (SSH, stdio, future transports). For SSH
/// connections, authentication is already completed at the transport layer
/// via public key, so we skip TOTP.
pub(crate) async fn handle_transport_connection(
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

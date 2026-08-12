//! Gateway listener / acceptor layer.
//!
//! [`run_gateway`] is the networked entry point: it bootstraps shared state
//! (model registry, copilot session, sandbox), optionally starts the messenger
//! loop, then accepts SSH (or stdio) transports and hands each one to the
//! per-connection engine in [`crate::server`]. Invoked from the binary entry
//! point in `main.rs`.

use rustyclaw_core::ignore::Ignore;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{
    CopilotSession, GatewayOptions, ModelContext, Transport, TransportAcceptor,
};
use rustyclaw_core::theme as t;
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
                    reg.set_active(&qualified).ignore();
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

    // Publish the configured tool budgets before anything can run a tool.
    // `RUSTYCLAW_RATE_LIMIT` predates the config section and still works, so
    // existing deployments that set it keep the behaviour they tuned.
    let mut tool_limits = config.tool_limits.clone();
    if let Some(env_max) = std::env::var("RUSTYCLAW_RATE_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        tool_limits.default_max_calls = env_max;
    }
    info!(
        window_secs = tool_limits.window_secs,
        default_max_calls = tool_limits.default_max_calls,
        max_background_processes = tool_limits.max_background_processes,
        max_subagents = tool_limits.max_subagents,
        max_background_sessions = tool_limits.max_background_sessions,
        max_rounds_per_minute = tool_limits.max_rounds_per_minute,
        "Tool budgets installed"
    );
    rustyclaw_core::tool_limits::install(tool_limits);

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
    {
        // Merge explicit [services.*] with auto-start engine service defs.
        let mut all_services = config.services.clone();
        let engine_svcs = rustyclaw_core::engines::engine_service_defs(&config.engines);
        if !engine_svcs.is_empty() {
            info!(
                count = engine_svcs.len(),
                "Registering auto-start engine services"
            );
            for (name, def) in engine_svcs {
                all_services.entry(name).or_insert(def);
            }
        }

        if !all_services.is_empty() {
            let svc_count = all_services.len();
            let svc_config = rustyclaw_core::services::ServicesConfig {
                services: all_services,
            };
            let svc_mgr = rustyclaw_core::services::create_service_manager(svc_config);
            info!(count = svc_count, "Managed services configured");
            // Auto-start services, in every mode including `--ssh-stdio`.
            //
            // Skipping stdio looks tempting — one gateway runs per SSH
            // connection there, so each connect starts its own copy of every
            // service — but the OpenSSH subsystem deployment documented in
            // `crate::ssh` has *no* standalone daemon: the stdio instance is
            // the only gateway there is. Not starting them leaves that
            // installation with no local inference server and a gateway that
            // cannot reach a model, which is worse than starting a duplicate.
            //
            // What must not happen is leaving them behind, and that is
            // handled at the other end: `stop_managed_services` runs on both
            // ways out of this function, and `ServiceManager::stop_all`
            // iterates only what this manager itself started, so an instance
            // cleans up its own children and never reaps another session's.
            //
            // Concurrent stdio sessions therefore still duplicate — a second
            // copy of a port-bound server will fail to bind and fall to its
            // restart policy. That is pre-existing and inherent to a
            // deployment with no process that outlives the connection;
            // solving it needs a cross-process notion of "already running"
            // (a health probe, or a lock in the settings dir with all the
            // staleness that implies), which is a supervision feature, not a
            // line in this function.
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
    }

    // ── MCP servers ─────────────────────────────────────────────────
    //
    // Register the shared MCP manager (used by the /mcp panel and tool
    // dispatch) and connect any [mcp.servers.*] entries in the background
    // so a slow server doesn't hold up gateway startup.
    #[cfg(feature = "mcp")]
    {
        let mcp_mgr: rustyclaw_core::mcp::SharedMcpManager = std::sync::Arc::new(
            tokio::sync::Mutex::new(rustyclaw_core::mcp::McpManager::new(config.mcp.clone())),
        );
        rustyclaw_core::runtime_ctx::set_mcp_manager(mcp_mgr.clone());
        if config.mcp.has_servers() {
            let server_count = config.mcp.servers.len();
            tokio::spawn(async move {
                let mgr = mcp_mgr.lock().await;
                if let Err(e) = mgr.connect_all().await {
                    tracing::warn!(error = %e, "Failed to connect MCP servers");
                } else {
                    info!(count = server_count, "MCP servers connected");
                }
            });
        }
    }

    // Register the credentials directory so file-access tools can enforce
    // the vault boundary (blocks read_file, execute_command, etc.).
    tools::set_credentials_dir(config.credentials_dir());

    // Register the vault so web_fetch can access the cookie jar.
    tools::set_vault(vault.clone());

    // Initialize sandbox for command execution
    let sandbox_mode = config.sandbox.mode.parse().unwrap_or_else(|e| {
        tracing::warn!(mode = %config.sandbox.mode, error = %e, "Invalid sandbox mode in config; falling back to auto-detection");
        Default::default()
    });
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

    // Give `sessions_spawn` something to actually run. Core owns the session
    // records but has no model client, so without this the tool refuses
    // rather than filing a record for work that never happens.
    rustyclaw_core::sessions::set_spawn_runner(Arc::new(
        crate::spawn_runner::GatewaySpawnRunner::new(
            rustyclaw_core::providers::http_client(),
            shared_config.clone(),
            shared_model_ctx.clone(),
            vault.clone(),
            skill_mgr.clone(),
            task_mgr.clone(),
            shared_copilot_session.clone(),
            tokio::runtime::Handle::current(),
        ),
    ));

    if options.ssh_stdio {
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("SSH_USER"))
            .ok();
        let transport = Box::new(StdioTransport::new(username));

        info!("Gateway running in SSH stdio mode");
        let served = handle_transport_connection(
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
        // The other early return from this function, and it needs the same
        // shutdown the listener path gets. This mode auto-starts services
        // like any other, and the `ServiceManager` that owns them lives in a
        // `'static` runtime context that is never dropped — so without this,
        // every SSH connection left a full set of service processes behind
        // with nobody managing them.
        stop_managed_services().await;
        return served;
    }

    // ── Initialize and start messenger loop ─────────────────────────
    //
    // Spawned unconditionally, not just when messengers exist at boot: the
    // setup panel adds accounts at runtime, and the loop is what notices
    // them (it re-reads the shared config each tick). Gating on the boot
    // config meant a first account saved through the panel could never
    // connect until a restart. With nothing configured, polling an empty
    // manager is a no-op.
    let messenger_mgr = {
        match messenger_handler::create_messenger_manager(&config, &vault).await {
            Ok(mgr) => {
                let shared_mgr: SharedMessengerManager = Arc::new(Mutex::new(mgr));

                // Spawn messenger loop
                let messenger_config = shared_config.clone();
                let messenger_ctx = model_ctx.clone();
                let messenger_vault = vault.clone();
                let messenger_skills = skill_mgr.clone();
                let messenger_tasks = task_mgr.clone();
                let messenger_models = model_registry.clone();
                let messenger_cancel = cancel.child_token();
                let mgr_clone = shared_mgr.clone();
                // Read current copilot session from shared state
                let messenger_copilot = shared_copilot_session.read().await.clone();

                tokio::spawn(async move {
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
                        error!(error = %e, "Messenger loop error");
                    }
                });

                Some(shared_mgr)
            }
            Err(e) => {
                error!(error = %e, "Failed to initialize messengers");
                None
            }
        }
    };

    // ── External triggers ───────────────────────────────────────────
    //
    // Start the trigger manager: it runs each enabled trigger's code as a
    // child process for the gateway's lifetime and exposes the localhost
    // fire endpoint. Fires are consumed here and dispatched as headless
    // agent turns. (Only in standalone mode — a per-connection stdio
    // gateway must not spawn duplicate trigger processes.)
    {
        let (fire_tx, mut fire_rx) =
            tokio::sync::mpsc::channel::<crate::trigger_manager::TriggerFire>(16);
        let trigger_notify = Arc::new(tokio::sync::Notify::new());
        rustyclaw_core::runtime_ctx::set_trigger_notify(trigger_notify.clone());

        let mgr_settings_dir = config.settings_dir.clone();
        let mgr_workspace = config.workspace_dir();
        let mgr_vault = vault.clone();
        let mgr_cancel = cancel.child_token();
        tokio::spawn(async move {
            if let Err(e) = crate::trigger_manager::run_trigger_manager(
                mgr_settings_dir,
                mgr_workspace,
                mgr_vault,
                fire_tx,
                trigger_notify,
                mgr_cancel,
            )
            .await
            {
                error!(error = %e, "Trigger manager error");
            }
        });

        // Fire consumer: runs the target agent for each validated fire.
        let fire_config = shared_config.clone();
        let fire_model_ctx = shared_model_ctx.clone();
        let fire_copilot = shared_copilot_session.clone();
        let fire_vault = vault.clone();
        let fire_skills = skill_mgr.clone();
        let fire_tasks = task_mgr.clone();
        let fire_models = model_registry.clone();
        let fire_cancel = cancel.child_token();
        tokio::spawn(async move {
            let http = rustyclaw_core::providers::http_client();
            loop {
                tokio::select! {
                    _ = fire_cancel.cancelled() => break,
                    fire = fire_rx.recv() => {
                        let Some(fire) = fire else { break };
                        let config = fire_config.read().await.clone();
                        let model_ctx = fire_model_ctx.read().await.clone();
                        let copilot = fire_copilot.read().await.clone();
                        crate::trigger_dispatch::run_trigger_fire(
                            &http,
                            &config,
                            fire,
                            model_ctx,
                            &fire_vault,
                            &fire_skills,
                            &fire_tasks,
                            &fire_models,
                            copilot.as_deref(),
                        )
                        .await;
                    }
                }
            }
        });
    }

    // ── Cron scheduler ──────────────────────────────────────────────
    //
    // Fires stored wake schedules as headless agent turns. Standalone
    // mode only, same as the trigger manager: a per-connection stdio
    // gateway must not run a duplicate scheduler against the same store.
    {
        let cron_notify = Arc::new(tokio::sync::Notify::new());
        rustyclaw_core::runtime_ctx::set_cron_notify(cron_notify.clone());
        let deps = crate::cron_runtime::CronDeps {
            config: shared_config.clone(),
            model_ctx: shared_model_ctx.clone(),
            copilot: shared_copilot_session.clone(),
            vault: vault.clone(),
            skill_mgr: skill_mgr.clone(),
            task_mgr: task_mgr.clone(),
            model_registry: model_registry.clone(),
        };
        let cron_cancel = cancel.child_token();
        tokio::spawn(crate::cron_runtime::run_cron_scheduler(
            deps,
            cron_notify,
            cron_cancel,
        ));
    }

    // Determine SSH listen address from CLI option or config.
    let ssh_listen = options
        .ssh_listen
        .clone()
        .or_else(|| {
            config.ssh.as_ref().and_then(|ssh_cfg| {
                if ssh_cfg.enabled && ssh_cfg.mode == rustyclaw_core::config::SshMode::Standalone {
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

    // Everything from here on runs inside the block, so every way out of it
    // — a failed bind, a dead acceptor, a cancelled token — reaches the
    // shutdown below. The managed services are already running by this point
    // and the `ServiceManager` that owns them lives in a `'static`
    // `runtime_ctx` that is never dropped, so a `?` that stepped over the
    // shutdown would leave them orphaned: `kill_on_drop` cannot fire on a
    // child whose manager outlives the process. That was survivable while a
    // failed bind left the process running; it is not now that the bind
    // fails fast, and "start a second gateway on a busy port" is the common
    // way to hit it.
    let serve_result: Result<()> = async {
        let mut ssh_server = SshServer::new(ssh_cfg).await?;
        // Binds before returning, so a failure here is reported instead of
        // becoming a gateway that runs and answers nothing — see
        // `SshServer::listen`.
        ssh_server.listen(bind_addr).await?;
        let bound_addr = ssh_server.local_addr().unwrap_or(bind_addr);

        info!(address = %bound_addr, "Gateway listening (SSH-only)");
        // The one line an operator watching a foreground run needs, and it is
        // printed only now that a socket is actually bound. It used to be
        // printed from `main` before any of this ran, against the address the
        // config asked for rather than the one in use.
        if !options.ssh_stdio {
            println!(
                "{}",
                t::icon_ok(&format!(
                    "Gateway listening on SSH {}",
                    t::info(&bound_addr.to_string())
                ))
            );
        }
        if messenger_mgr.is_some() {
            info!("Messenger polling enabled");
        }

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
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
                        // Not a per-connection hiccup: the acceptor only fails
                        // once the server behind it is gone, and every later
                        // call fails the same way. Logging and looping turned
                        // that into a spin at full CPU on a gateway nobody could
                        // reach; stop instead, and say why.
                        Err(e) => {
                            error!(
                                error = %e,
                                "SSH acceptor stopped; the gateway can no longer accept connections"
                            );
                            return Err(e);
                        }
                    }
                }
            }
        }
    }
    .await;

    stop_managed_services().await;

    serve_result
}

/// Stop every managed service this process started.
///
/// `run_gateway` has exactly two ways out — the stdio branch and the
/// listener block — and both must come through here. The `ServiceManager`
/// lives in a `'static` runtime context that is never dropped, so
/// `kill_on_drop` cannot collect its children: a return that skips this
/// leaves them running with nobody managing them.
async fn stop_managed_services() {
    if let Some(svc_mgr) = rustyclaw_core::runtime_ctx::get_service_manager() {
        info!("Stopping managed services…");
        let mut mgr = svc_mgr.write().await;
        mgr.stop_all().await;
    }
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

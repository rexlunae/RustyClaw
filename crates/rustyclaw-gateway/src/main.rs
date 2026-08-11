//! RustyClaw gateway server.
//!
//! Session handling, model dispatch, messenger and tool orchestration, and the
//! SSH server. The client-facing wire protocol and transport interface live in
//! [`rustyclaw_core::gateway`], which this crate builds upon.

mod admin;
mod agent_handler;
mod auth;
mod canvas_handler;
mod chat;
mod cli;
mod command_wrapper;
mod concurrent;
mod cron_runtime;
mod dispatch;
mod download_handler;
mod engine_handler;
mod errors;
mod helpers;
mod kernel_handler;
mod listen;
mod logging;
mod mcp_handler;
mod messenger_config_handler;
mod messenger_handler;
mod model_handler;
mod panel_handler;
mod pending;
mod plugin_handler;
mod project_handler;
mod providers;
mod secrets_handler;
mod server;
mod service_handler;
mod session;
mod skills_handler;
mod spawn_runner;
mod ssh;
mod subagent_runner;
mod system_prompt;
mod task_handler;
mod thread_handler;
mod thread_updates;
mod tool_executor;
mod trigger_dispatch;
mod trigger_manager;

use std::io::IsTerminal;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use anyhow::Result;
use clap::Parser;
use rustyclaw_core::config::Config;
use rustyclaw_core::daemon;
use rustyclaw_core::gateway::{CopilotSession, GatewayOptions, ModelContext};
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::skills::SkillManager;
use rustyclaw_core::theme as t;

use cli::{GatewayBind, GatewayCli, GatewayCommands, RunArgs, handle_pair_command};
use listen::run_gateway;

// ── Shared state aliases (referenced by the server engine and submodules) ────

/// Shared flag for cancelling the tool loop from another task.
pub type ToolCancelFlag = Arc<AtomicBool>;

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

/// Shared Copilot session, updated when provider changes.
pub type SharedCopilotSession = Arc<RwLock<Option<Arc<CopilotSession>>>>;

/// Shared task manager for first-class task orchestration.
pub type SharedTaskManager = Arc<rustyclaw_core::tasks::TaskManager>;

/// The connection's thread manager, shared with the model task.
///
/// A turn runs in its own task so the connection loop stays free to answer
/// everything else — thread switches, history requests, project changes —
/// while the model works or waits on an `ask_user` question. Both sides
/// touch the thread list, so it lives behind a mutex rather than being
/// borrowed exclusively for the length of a turn. Hold the guard for single
/// operations only: never across a model call.
pub type SharedThreadMgr = Arc<Mutex<rustyclaw_core::threads::ThreadManager>>;

/// The thread *this connection's client* is looking at.
///
/// Deliberately not on [`SharedThreadMgr`]: that manager is shared by every
/// window open on the agent (see [`rustyclaw_core::threads::manager_for`]),
/// while a foreground is a statement about one client's view. Keeping it
/// there made one window's thread switch drag the others onto it.
///
/// A cell rather than a plain field because a running turn reports sidebar
/// updates back to this client, and the client reads the `foreground_id` in
/// them as "switch to this conversation". A value captured when the turn was
/// spawned goes stale the moment the user switches threads mid-answer, and
/// sending it would pull them back. Read it live instead — the same reason
/// the reader holds its thread manager in a cell.
///
/// A `std::sync::RwLock`, not tokio's: every access is a field read or write
/// with no await between, and the turn machinery reads it from paths that are
/// not all async.
pub type ForegroundCell = Arc<std::sync::RwLock<Option<rustyclaw_core::threads::ThreadId>>>;

/// Read a [`ForegroundCell`], recovering from a poisoned lock.
///
/// A `PoisonError` here says some thread panicked while holding the lock, not
/// that the value is meaningless — it is one `Option<ThreadId>` written under
/// the lock in a single step. Propagating instead would make one unrelated
/// panic permanently break every sidebar update on the connection, since
/// poison never clears.
pub fn foreground_of(cell: &ForegroundCell) -> Option<rustyclaw_core::threads::ThreadId> {
    *cell.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Write a [`ForegroundCell`], recovering from a poisoned lock — see
/// [`foreground_of`].
pub fn set_foreground(cell: &ForegroundCell, id: Option<rustyclaw_core::threads::ThreadId>) {
    *cell
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = id;
}

/// Shared model registry for model management.
pub type SharedModelRegistry = rustyclaw_core::models::SharedModelRegistry;

/// Shared observer for recording telemetry events.
pub type SharedObserver = Arc<dyn rustyclaw_core::observability::Observer>;

// ── Constants (shared with the server engine via `crate::`) ──────────────────

/// Duration of the lockout after exceeding the failure limit.
pub(crate) const TOTP_LOCKOUT_SECS: u64 = 30;

/// Compaction fires when estimated usage exceeds this fraction of the context window.
pub(crate) const COMPACTION_THRESHOLD: f64 = 0.75;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = GatewayCli::parse();
    t::init_color(cli.common.no_color);
    let config_path = cli.common.config_path();
    let mut config = Config::load(config_path)?;
    cli.common.apply_overrides(&mut config);

    let args = match cli.command {
        Some(GatewayCommands::Run(args)) => args,
        Some(GatewayCommands::Status { json }) => {
            let url = config
                .gateway_url
                .as_deref()
                .unwrap_or("ws://127.0.0.1:9001");
            if json {
                println!("{{ \"gateway_url\": \"{}\" }}", url);
            } else {
                println!("{}", t::label_value("Gateway URL", url));
                println!(
                    "  {}",
                    t::muted("(detailed status probe not yet implemented)")
                );
            }
            return Ok(());
        }
        Some(GatewayCommands::Pair(pair_cmd)) => {
            return handle_pair_command(pair_cmd).await;
        }
        None => RunArgs::default(),
    };

    let protocol_stdio = args.ssh_stdio;

    // Before anything else the daemon does: `tracing` discards every event
    // until a subscriber exists, so a line emitted above this one is gone.
    logging::init(&config, protocol_stdio);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        pid = std::process::id(),
        stdio = protocol_stdio,
        "Gateway starting"
    );

    let host = match args.bind {
        GatewayBind::Loopback => "127.0.0.1",
        GatewayBind::Lan => "0.0.0.0",
        _ => "127.0.0.1",
    };

    let listen = args
        .listen
        .unwrap_or_else(|| format!("{}:{}", host, args.port));

    // Resolve TLS paths: CLI args override config
    let tls_cert = args.tls_cert.or(config.tls_cert.clone());
    let tls_key = args.tls_key.or(config.tls_key.clone());
    let scheme = if tls_cert.is_some() { "wss" } else { "ws" };

    // Determine the actual SSH listen address (CLI arg > config > default)
    let ssh_addr = args
        .ssh_listen
        .clone()
        .or_else(|| {
            config.ssh.as_ref().and_then(|s| {
                if s.enabled && s.mode == rustyclaw_core::config::SshMode::Standalone {
                    Some(s.bind.clone())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "0.0.0.0:2222".to_string());

    if !protocol_stdio {
        println!(
            "{}",
            t::icon_ok(&format!("Gateway listening on SSH {}", t::info(&ssh_addr)))
        );
    }
    // Keep the ws:// listen var for run_gateway options but don't surface it.
    let _ = scheme;

    // ── Open the secrets vault ───────────────────────────────────────────
    //
    // The gateway owns the secrets vault.  It uses the vault to:
    //   1. Resolve model API keys (if not injected via env var)
    //   2. Verify TOTP codes during client authentication
    //
    // When launched as a daemon, the parent may inject the vault password
    // via RUSTYCLAW_VAULT_PASSWORD so the gateway can unlock non-
    // interactively.  In foreground mode, we prompt on stdin.
    //
    // If no password is available for a password-protected vault, the
    // gateway starts in a "vault locked" state — authenticated clients
    // can unlock it later via a control message.
    let vault = {
        let creds_dir = config.credentials_dir();
        let env_password = std::env::var("RUSTYCLAW_VAULT_PASSWORD").ok();
        if env_password.is_some() {
            // SAFETY: single-threaded at this point.
            unsafe {
                std::env::remove_var("RUSTYCLAW_VAULT_PASSWORD");
            }
        }

        if config.secrets_password_protected {
            if let Some(pw) = env_password {
                if !protocol_stdio {
                    println!("  {} Vault password provided by launcher", t::icon_ok(""));
                }
                SecretsManager::with_password(&creds_dir, pw)
            } else if std::io::stdin().is_terminal() {
                // Interactive foreground mode — prompt for password.
                let password =
                    rpassword::prompt_password(format!("{} Vault password: ", t::info("🔑")))
                        .unwrap_or_default();
                SecretsManager::with_password(&creds_dir, password)
            } else {
                // Daemon mode with no password — start locked.
                if !protocol_stdio {
                    println!(
                        "  {} Vault locked (no password provided — clients can unlock via SSH)",
                        t::muted("🔒")
                    );
                }
                SecretsManager::locked(&creds_dir)
            }
        } else {
            SecretsManager::new(&creds_dir)
        }
    };

    let shared_vault: crate::SharedVault = std::sync::Arc::new(tokio::sync::Mutex::new(vault));

    // ── Resolve model context ────────────────────────────────────────────
    //
    // When launched as a daemon, the CLI extracts just the provider's API
    // key and passes it via RUSTYCLAW_MODEL_API_KEY so the gateway can
    // avoid opening the vault just for the API key.
    //
    // When running interactively (foreground) or when no env key is set,
    // resolve from the vault (which we just opened above).
    let model_ctx = {
        let env_key = std::env::var("RUSTYCLAW_MODEL_API_KEY").ok();

        if let Some(ref key) = env_key {
            // Key was injected by the parent process — use it directly.
            // SAFETY: single-threaded at this point.
            unsafe {
                std::env::remove_var("RUSTYCLAW_MODEL_API_KEY");
            }

            let api_key = if key.is_empty() {
                None
            } else {
                Some(key.clone())
            };
            match ModelContext::from_config(&config, api_key) {
                Ok(ctx) => {
                    if !protocol_stdio {
                        println!(
                            "{} {} via {} ({})",
                            t::icon_ok("Model:"),
                            t::info(&ctx.model),
                            t::info(&ctx.provider),
                            t::muted(&ctx.base_url),
                        );
                    }
                    if ctx.api_key.is_some() && !protocol_stdio {
                        println!("  {} API key provided by launcher", t::icon_ok(""));
                    }
                    Some(ctx)
                }
                Err(err) => {
                    eprintln!("{} Could not resolve model context: {}", t::muted("⚠"), err);
                    None
                }
            }
        } else {
            // Resolve from the vault.
            let mut v = shared_vault.lock().await;
            match ModelContext::resolve(&config, &mut v) {
                Ok(ctx) => {
                    if !protocol_stdio {
                        println!(
                            "{} {} via {} ({})",
                            t::icon_ok("Model:"),
                            t::info(&ctx.model),
                            t::info(&ctx.provider),
                            t::muted(&ctx.base_url),
                        );
                    }
                    if ctx.api_key.is_some() && !protocol_stdio {
                        println!("  {} API key loaded from vault", t::icon_ok(""));
                    }
                    Some(ctx)
                }
                Err(err) => {
                    eprintln!("{} Could not resolve model context: {}", t::muted("⚠"), err,);
                    eprintln!(
                        "  {}",
                        t::muted("The gateway will rely on clients sending full credentials."),
                    );
                    None
                }
            }
        }
    };

    // Write PID file so `rustyclaw gateway stop` can find us.
    let pid = std::process::id();
    daemon::write_pid(&config.settings_dir, pid)?;

    // Set up graceful shutdown on Ctrl+C (all platforms).
    let cancel = CancellationToken::new();
    let cancel_for_signal = cancel.clone();
    let settings_dir = config.settings_dir.clone();
    tokio::spawn(async move {
        // The `Result` decides whether to stop; it used to be discarded with
        // `.ignore()` and the cancel ran either way. Failing to *listen* for
        // Ctrl+C is not a reason to shut down, but that is what it did —
        // silently, with no console output and without touching the PID file,
        // so it looked exactly like a clean operator-requested stop.
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                tracing::info!("Received Ctrl+C — shutting down");
                cancel_for_signal.cancel();
            }
            Err(e) => tracing::error!(
                error = %e,
                "Cannot listen for Ctrl+C; the gateway keeps running, but Ctrl+C \
                 will not stop it"
            ),
        }
    });

    // On Unix, also handle SIGTERM for graceful shutdown when stopped via
    // `rustyclaw gateway stop` (which sends SIGTERM through sysinfo).
    // Windows doesn't have SIGTERM — sysinfo uses TerminateProcess there,
    // so no signal handler is needed; the PID-file cleanup below covers it.
    #[cfg(unix)]
    {
        let cancel_for_term = cancel.clone();
        let settings_dir_term = settings_dir.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            match signal(SignalKind::terminate()) {
                // `recv()` yields `None` when the stream closes, which is not
                // a SIGTERM. Discarding the `Option` meant a closed stream
                // shut the gateway down as if one had arrived.
                Ok(mut sig) => match sig.recv().await {
                    Some(()) => {
                        tracing::info!("Received SIGTERM — shutting down");
                        cancel_for_term.cancel();
                        daemon::remove_pid(&settings_dir_term);
                    }
                    None => tracing::error!(
                        "SIGTERM stream closed; the gateway keeps running, but \
                         `rustyclaw gateway stop` will not stop it"
                    ),
                },
                Err(e) => tracing::error!(error = %e, "Cannot listen for SIGTERM"),
            }
        });
    }

    let result = {
        // Load skills for the gateway.
        let skills_dir = config.skills_dir();
        let mut sm = rustyclaw_core::skills::SkillManager::new(skills_dir);
        if let Err(e) = sm.load_skills() {
            eprintln!("⚠ Could not load skills: {}", e);
        }
        if let Some(url) = config.clawhub_url.as_deref() {
            sm.set_registry(url, config.clawhub_token.clone());
        }
        let shared_skills: crate::SharedSkillManager =
            std::sync::Arc::new(tokio::sync::Mutex::new(sm));

        // Initialize plugin manager (dynamic UI panels).
        rustyclaw_core::tools::init_plugin_manager(&config.workspace_dir());

        // Telemetry: aggregate usage stats and keep a log ring for the
        // analytics/logs panels, plus tracing output for operators. The
        // stats handle is registered globally so the panel handler can
        // query it.
        let stats = std::sync::Arc::new(rustyclaw_core::observability::StatsObserver::new());
        rustyclaw_core::runtime_ctx::set_stats_observer(stats.clone());

        // Publish the settings dir so tools (agents_list, agents_create,
        // sessions_spawn validation) can reach the agent registry even on
        // flows that never touch a client connection (messengers, cron).
        rustyclaw_core::runtime_ctx::set_agent_registry_info(
            &config.settings_dir,
            &config.agent_name,
        );
        let observer: crate::SharedObserver =
            std::sync::Arc::new(rustyclaw_core::observability::CompositeObserver::new(vec![
                std::sync::Arc::new(rustyclaw_core::observability::LogObserver::new()),
                stats,
            ]));

        run_gateway(
            config,
            GatewayOptions {
                listen,
                tls_cert,
                tls_key,
                ssh_listen: args.ssh_listen.clone(),
                ssh_stdio: args.ssh_stdio,
                ssh_host_key: args.ssh_host_key.clone(),
                ssh_authorized_clients: args.ssh_authorized_clients.clone(),
            },
            model_ctx,
            shared_vault,
            shared_skills,
            None,
            None,
            Some(observer),
            cancel,
        )
        .await
    };
    daemon::remove_pid(&settings_dir);

    result
}

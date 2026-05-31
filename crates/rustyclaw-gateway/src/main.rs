//! RustyClaw gateway server.
//!
//! Session handling, model dispatch, messenger and tool orchestration, and the
//! SSH server. The client-facing wire protocol and transport interface live in
//! [`rustyclaw_core::gateway`], which this crate builds upon.

mod auth;
mod canvas_handler;
mod cli;
mod command_wrapper;
mod concurrent;
mod dispatch;
mod errors;
mod helpers;
mod mcp_handler;
mod messenger_handler;
mod model_handler;
mod providers;
mod secrets_handler;
mod server;
mod skills_handler;
mod ssh;
mod system_prompt;
mod task_handler;
mod thread_updates;
mod tool_executor;

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
use server::run_gateway;

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
                if s.enabled && s.mode == "standalone" {
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
        let _ = tokio::signal::ctrl_c().await;
        cancel_for_signal.cancel();
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
            if let Ok(mut sig) = signal(SignalKind::terminate()) {
                sig.recv().await;
                cancel_for_term.cancel();
                daemon::remove_pid(&settings_dir_term);
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
            None, // observer
            cancel,
        )
        .await
    };
    daemon::remove_pid(&settings_dir);

    result
}

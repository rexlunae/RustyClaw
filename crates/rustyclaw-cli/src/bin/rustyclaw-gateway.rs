use std::io::IsTerminal;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use rustyclaw_core::args::CommonArgs;
use rustyclaw_core::config::Config;
use rustyclaw_core::daemon;
use rustyclaw_core::gateway::{run_gateway, GatewayOptions, ModelContext};
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::theme as t;
use tokio_util::sync::CancellationToken;

// ── Gateway bind modes ──────────────────────────────────────────────────────

#[derive(Debug, Clone, ValueEnum)]
enum GatewayBind {
    Loopback,
    Lan,
    Tailnet,
    Auto,
    Custom,
}

// ── Gateway auth modes ──────────────────────────────────────────────────────

#[derive(Debug, Clone, ValueEnum)]
enum GatewayAuth {
    Token,
    Password,
}

// ── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(
    name = "rustyclaw-gateway",
    version,
    about = "RustyClaw gateway — run the WebSocket gateway in the foreground"
)]
struct GatewayCli {
    #[command(flatten)]
    common: CommonArgs,
    #[command(subcommand)]
    command: Option<GatewayCommands>,
}

#[derive(Debug, Subcommand)]
enum GatewayCommands {
    /// Run the gateway (default when no subcommand is given)
    Run(RunArgs),
    /// Show gateway status
    Status {
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    /// Gateway port
    #[arg(long, value_name = "PORT", default_value_t = 9001)]
    port: u16,
    /// Bind mode (loopback, lan, tailnet, auto, custom)
    #[arg(long, value_enum, default_value_t = GatewayBind::Loopback)]
    bind: GatewayBind,
    /// Auth token
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Auth mode
    #[arg(long, value_enum)]
    auth: Option<GatewayAuth>,
    /// Auth password
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,
    /// Overwrite existing configuration
    #[arg(long)]
    force: bool,
    /// Verbose logging
    #[arg(long, short)]
    verbose: bool,
    /// WebSocket listen URL (ws://host:port) — overrides --bind/--port
    #[arg(long = "listen", alias = "url", alias = "ws", value_name = "WS_URL")]
    listen: Option<String>,
    /// Path to TLS certificate file (PEM) for WSS connections
    #[arg(long, value_name = "PATH")]
    tls_cert: Option<std::path::PathBuf>,
    /// Path to TLS private key file (PEM) for WSS connections
    #[arg(long, value_name = "PATH")]
    tls_key: Option<std::path::PathBuf>,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            port: 9001,
            bind: GatewayBind::Loopback,
            token: None,
            auth: None,
            password: None,
            force: false,
            verbose: false,
            listen: None,
            tls_cert: None,
            tls_key: None,
        }
    }
}

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
            let url = config.gateway_url.as_deref().unwrap_or("ws://127.0.0.1:9001");
            if json {
                println!("{{ \"gateway_url\": \"{}\" }}", url);
            } else {
                println!("{}", t::label_value("Gateway URL", url));
                println!("  {}", t::muted("(detailed status probe not yet implemented)"));
            }
            return Ok(());
        }
        None => RunArgs::default(),
    };

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

    println!("{}", t::icon_ok(&format!("Gateway listening on {}", t::info(&format!("{}://{}", scheme, listen)))));

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
            unsafe { std::env::remove_var("RUSTYCLAW_VAULT_PASSWORD"); }
        }

        if config.secrets_password_protected {
            if let Some(pw) = env_password {
                println!("  {} Vault password provided by launcher", t::icon_ok(""));
                SecretsManager::with_password(&creds_dir, pw)
            } else if std::io::stdin().is_terminal() {
                // Interactive foreground mode — prompt for password.
                let password = rpassword::prompt_password(
                    format!("{} Vault password: ", t::info("🔑")),
                )
                .unwrap_or_default();
                SecretsManager::with_password(&creds_dir, password)
            } else {
                // Daemon mode with no password — start locked.
                println!("  {} Vault locked (no password provided — clients can unlock via WebSocket)", t::muted("🔒"));
                SecretsManager::locked(&creds_dir)
            }
        } else {
            SecretsManager::new(&creds_dir)
        }
    };

    let shared_vault: rustyclaw_core::gateway::SharedVault =
        std::sync::Arc::new(tokio::sync::Mutex::new(vault));

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
            unsafe { std::env::remove_var("RUSTYCLAW_MODEL_API_KEY"); }

            let api_key = if key.is_empty() { None } else { Some(key.clone()) };
            match ModelContext::from_config(&config, api_key) {
                Ok(ctx) => {
                    println!(
                        "{} {} via {} ({})",
                        t::icon_ok("Model:"),
                        t::info(&ctx.model),
                        t::info(&ctx.provider),
                        t::muted(&ctx.base_url),
                    );
                    if ctx.api_key.is_some() {
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
                    println!(
                        "{} {} via {} ({})",
                        t::icon_ok("Model:"),
                        t::info(&ctx.model),
                        t::info(&ctx.provider),
                        t::muted(&ctx.base_url),
                    );
                    if ctx.api_key.is_some() {
                        println!("  {} API key loaded from vault", t::icon_ok(""));
                    }
                    Some(ctx)
                }
                Err(err) => {
                    eprintln!(
                        "{} Could not resolve model context: {}",
                        t::muted("⚠"),
                        err,
                    );
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
            use tokio::signal::unix::{signal, SignalKind};
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
        let shared_skills: rustyclaw_core::gateway::SharedSkillManager =
            std::sync::Arc::new(tokio::sync::Mutex::new(sm));

        run_gateway(config, GatewayOptions { listen, tls_cert, tls_key }, model_ctx, shared_vault, shared_skills, None, None, cancel).await
    };
    daemon::remove_pid(&settings_dir);

    result
}

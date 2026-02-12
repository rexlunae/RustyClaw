use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use rustyclaw::args::CommonArgs;
use rustyclaw::config::Config;
use rustyclaw::daemon;
use rustyclaw::gateway::{run_gateway, GatewayOptions, ModelContext};
use rustyclaw::secrets::SecretsManager;
use rustyclaw::theme as t;
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

    println!("{}", t::icon_ok(&format!("Gateway listening on {}", t::info(&format!("ws://{}", listen)))));

    // ── Resolve model context ────────────────────────────────────────────
    //
    // When launched as a daemon, the CLI extracts just the provider's API
    // key and passes it via RUSTYCLAW_MODEL_API_KEY so the gateway never
    // needs access to the full secrets vault.
    //
    // When running interactively (foreground), fall back to opening the
    // vault directly — prompting for the password if needed.
    let model_ctx = {
        let env_key = std::env::var("RUSTYCLAW_MODEL_API_KEY").ok();

        if let Some(ref key) = env_key {
            // Key was injected by the parent process — use it directly.
            // Clear the env var so child processes don't inherit it.
            // SAFETY: We are single-threaded at this point (before tokio
            // spawns any tasks), so mutating the environment is safe.
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
            // Interactive (foreground) mode — open the vault directly.
            let creds_dir = config.credentials_dir();
            let mut secrets = if config.secrets_password_protected {
                let password = rpassword::prompt_password(
                    &format!("{} Vault password: ", t::info("🔑")),
                )
                .unwrap_or_default();
                SecretsManager::with_password(&creds_dir, password)
            } else {
                SecretsManager::new(&creds_dir)
            };

            match ModelContext::resolve(&config, &mut secrets) {
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

    let result = run_gateway(config, GatewayOptions { listen }, model_ctx, cancel).await;

    // Clean up PID file on exit.
    daemon::remove_pid(&settings_dir);

    result
}

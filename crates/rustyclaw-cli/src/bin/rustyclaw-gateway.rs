use std::io::IsTerminal;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use rustyclaw_core::args::CommonArgs;
use rustyclaw_core::config::Config;
use rustyclaw_core::daemon;
use rustyclaw_core::gateway::{GatewayOptions, ModelContext, run_gateway};
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
    /// Manage SSH pairing and authorized clients
    #[command(subcommand)]
    Pair(PairCommands),
}

#[derive(Debug, Subcommand)]
enum PairCommands {
    /// List authorized clients
    List,
    /// Add a new authorized client
    Add {
        /// Public key in OpenSSH format (ssh-ed25519 AAAA...)
        #[arg(value_name = "PUBLIC_KEY")]
        key: String,
        /// Optional name/comment for the client
        #[arg(long, short)]
        name: Option<String>,
    },
    /// Remove an authorized client by fingerprint
    Remove {
        /// Key fingerprint (SHA256:...)
        #[arg(value_name = "FINGERPRINT")]
        fingerprint: String,
    },
    /// Show pairing QR code for this gateway
    Qr {
        /// Gateway host:port (required for QR generation)
        #[arg(long, value_name = "HOST:PORT")]
        host: String,
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
    /// SSH server listen address (e.g., 0.0.0.0:2222)
    #[arg(long, value_name = "ADDR")]
    ssh_listen: Option<String>,
    /// Run as SSH subsystem (stdio mode for OpenSSH integration)
    #[arg(long)]
    ssh_stdio: bool,
    /// Path to SSH host key (default: ~/.rustyclaw/ssh_host_key)
    #[arg(long, value_name = "PATH")]
    ssh_host_key: Option<std::path::PathBuf>,
    /// Path to authorized_clients file (default: ~/.rustyclaw/authorized_clients)
    #[arg(long, value_name = "PATH")]
    ssh_authorized_clients: Option<std::path::PathBuf>,
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
            ssh_listen: None,
            ssh_stdio: false,
            ssh_host_key: None,
            ssh_authorized_clients: None,
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

    // SSH stdio mode runs without any TCP listeners — just stdin/stdout
    if args.ssh_stdio {
        println!("{}", t::icon_ok("Running in SSH subsystem mode (stdio)"));
        return run_ssh_stdio_mode(config, args).await;
    }

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

    println!(
        "{}",
        t::icon_ok(&format!(
            "Gateway listening on {}",
            t::info(&format!("{}://{}", scheme, listen))
        ))
    );

    // Print SSH listen address if configured
    if let Some(ref ssh_addr) = args.ssh_listen {
        println!(
            "{}",
            t::icon_ok(&format!(
                "SSH server listening on {}",
                t::info(ssh_addr)
            ))
        );
    }

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
                println!("  {} Vault password provided by launcher", t::icon_ok(""));
                SecretsManager::with_password(&creds_dir, pw)
            } else if std::io::stdin().is_terminal() {
                // Interactive foreground mode — prompt for password.
                let password =
                    rpassword::prompt_password(format!("{} Vault password: ", t::info("🔑")))
                        .unwrap_or_default();
                SecretsManager::with_password(&creds_dir, password)
            } else {
                // Daemon mode with no password — start locked.
                println!(
                    "  {} Vault locked (no password provided — clients can unlock via WebSocket)",
                    t::muted("🔒")
                );
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
        let shared_skills: rustyclaw_core::gateway::SharedSkillManager =
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

/// Run the gateway in SSH subsystem (stdio) mode.
///
/// This mode is used when the gateway is invoked via OpenSSH's subsystem
/// mechanism. Instead of listening on TCP, we read/write frames on stdin/stdout.
async fn run_ssh_stdio_mode(config: Config, _args: RunArgs) -> Result<()> {
    use rustyclaw_core::gateway::{StdioTransport, Transport};
    
    // Get username from SSH environment
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("SSH_USER"))
        .ok();
    
    // Create the stdio transport
    let mut transport = StdioTransport::new(username);
    
    // ── Open the secrets vault ───────────────────────────────────────────
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
                SecretsManager::with_password(&creds_dir, pw)
            } else {
                // In stdio mode, we can't prompt for password
                SecretsManager::locked(&creds_dir)
            }
        } else {
            SecretsManager::new(&creds_dir)
        }
    };

    let shared_vault: rustyclaw_core::gateway::SharedVault =
        std::sync::Arc::new(tokio::sync::Mutex::new(vault));

    // ── Resolve model context ────────────────────────────────────────────
    let _model_ctx = {
        let env_key = std::env::var("RUSTYCLAW_MODEL_API_KEY").ok();

        if let Some(ref key) = env_key {
            unsafe {
                std::env::remove_var("RUSTYCLAW_MODEL_API_KEY");
            }

            let api_key = if key.is_empty() { None } else { Some(key.clone()) };
            ModelContext::from_config(&config, api_key).ok()
        } else {
            let mut v = shared_vault.lock().await;
            ModelContext::resolve(&config, &mut v).ok()
        }
    };

    // Load skills
    let skills_dir = config.skills_dir();
    let mut sm = rustyclaw_core::skills::SkillManager::new(skills_dir);
    let _ = sm.load_skills();
    if let Some(url) = config.clawhub_url.as_deref() {
        sm.set_registry(url, config.clawhub_token.clone());
    }
    let _shared_skills: rustyclaw_core::gateway::SharedSkillManager =
        std::sync::Arc::new(tokio::sync::Mutex::new(sm));

    // Set up cancellation
    let cancel = CancellationToken::new();
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancel_for_signal.cancel();
    });

    // TODO: Run the actual connection handler with the stdio transport
    // For now, this is a stub that just logs and exits
    eprintln!("SSH stdio mode not yet fully implemented");
    eprintln!("Transport peer info: {:?}", transport.peer_info());
    
    transport.close().await?;
    Ok(())
}

/// Handle pairing subcommands.
async fn handle_pair_command(cmd: PairCommands) -> Result<()> {
    use rustyclaw_core::pairing::{
        default_authorized_clients_path,
        load_authorized_clients,
        add_authorized_client,
        remove_authorized_client,
    };
    
    let auth_path = default_authorized_clients_path();
    
    match cmd {
        PairCommands::List => {
            let clients = load_authorized_clients(&auth_path)?;
            
            if clients.clients.is_empty() {
                println!("{}", t::muted("No authorized clients"));
                println!();
                println!("Add a client with:");
                println!("  {} pair add <PUBLIC_KEY> --name <NAME>", t::info("rustyclaw-gateway"));
                return Ok(());
            }
            
            println!("{}", t::heading("Authorized Clients"));
            println!();
            
            for (i, client) in clients.clients.iter().enumerate() {
                let name = client.comment.as_deref().unwrap_or("(unnamed)");
                println!(
                    "{}. {} {}",
                    i + 1,
                    t::info(name),
                    t::muted(&format!("({})", &client.fingerprint))
                );
            }
            
            println!();
            println!(
                "{} {}",
                t::muted("File:"),
                auth_path.display()
            );
        }
        
        PairCommands::Add { key, name } => {
            match add_authorized_client(&auth_path, &key, name.as_deref()) {
                Ok(client) => {
                    println!(
                        "{} Added client: {}",
                        t::icon_ok(""),
                        t::info(client.comment.as_deref().unwrap_or("(unnamed)"))
                    );
                    println!(
                        "  {} {}",
                        t::muted("Fingerprint:"),
                        client.fingerprint
                    );
                }
                Err(e) => {
                    eprintln!("{} Failed to add client: {}", t::icon_fail(""), e);
                    std::process::exit(1);
                }
            }
        }
        
        PairCommands::Remove { fingerprint } => {
            match remove_authorized_client(&auth_path, &fingerprint) {
                Ok(true) => {
                    println!(
                        "{} Removed client with fingerprint: {}",
                        t::icon_ok(""),
                        fingerprint
                    );
                }
                Ok(false) => {
                    eprintln!(
                        "{} No client found with fingerprint: {}",
                        t::icon_fail(""),
                        fingerprint
                    );
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("{} Failed to remove client: {}", t::icon_fail(""), e);
                    std::process::exit(1);
                }
            }
        }
        
        PairCommands::Qr { host } => {
            use rustyclaw_core::pairing::{PairingData, generate_pairing_qr_ascii};
            
            // Generate gateway pairing data
            // For now, we use a placeholder key - in production, this would be the host key's public part
            let data = PairingData::gateway(
                "ssh-ed25519 (host key would go here)",
                &host,
                Some("RustyClaw Gateway".to_string()),
            );
            
            match generate_pairing_qr_ascii(&data) {
                Ok(qr) => {
                    println!("{}", t::heading("Gateway Pairing QR Code"));
                    println!();
                    println!("{}", qr);
                    println!();
                    println!("Scan this QR code with a RustyClaw client to pair.");
                    println!("Gateway address: {}", t::info(&host));
                }
                Err(e) => {
                    eprintln!("{} Failed to generate QR code: {}", t::icon_fail(""), e);
                    std::process::exit(1);
                }
            }
        }
    }
    
    Ok(())
}

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
#[cfg(feature = "tui")]
use rustyclaw::app::App;
use rustyclaw::args::CommonArgs;
use rustyclaw::commands::{handle_command, CommandAction, CommandContext};
use rustyclaw::config::Config;
#[cfg(feature = "tui")]
use rustyclaw::onboard::run_onboard_wizard;
use rustyclaw::providers;
use rustyclaw::secrets::SecretsManager;
use rustyclaw::skills::SkillManager;
use std::path::PathBuf;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

// ── Top-level CLI ───────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(
    name = "rustyclaw",
    version,
    about = "RustyClaw — lightweight agentic assistant",
    long_about = "RustyClaw — a super-lightweight super-capable agentic tool with improved security.\n\n\
                  Run without a subcommand to launch the TUI."
)]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,
    #[command(subcommand)]
    command: Option<Commands>,
}

// ── Subcommands (mirrors openclaw command tree) ─────────────────────────────

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialise config + workspace (runs the wizard when wizard flags are present)
    Setup(SetupArgs),

    /// Interactive onboarding wizard — set up gateway, workspace, and skills
    Onboard(OnboardArgs),

    /// Import an existing OpenClaw installation into RustyClaw
    Import(ImportArgs),

    /// Interactive configuration wizard (models, gateway, skills)
    Configure,

    /// Config helpers: get / set / unset
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Health checks + quick fixes for gateway and configuration
    Doctor(DoctorArgs),

    /// Launch the terminal UI
    #[command(alias = "ui")]
    Tui(TuiArgs),

    /// Send a one-shot message / slash-command
    #[command(alias = "cmd", alias = "run", alias = "message")]
    Command(CommandArgs),

    /// Show system status (gateway, model, workspace)
    Status(StatusArgs),

    /// Gateway management (start / stop / restart / status)
    #[command(subcommand)]
    Gateway(GatewayCommands),

    /// List / manage skills
    #[command(subcommand)]
    Skills(SkillsCommands),

    /// Refresh the GitHub Copilot session token from OpenClaw
    #[command(alias = "refresh")]
    RefreshToken(RefreshTokenArgs),
}

// ── Setup ───────────────────────────────────────────────────────────────────

#[derive(Debug, Args, Default)]
struct SetupArgs {
    /// Agent workspace directory (default: ~/.rustyclaw/workspace)
    #[arg(long, value_name = "DIR")]
    workspace: Option<String>,
    /// Run the onboarding wizard
    #[arg(long)]
    wizard: bool,
    /// Run wizard without prompts
    #[arg(long)]
    non_interactive: bool,
    /// Wizard mode
    #[arg(long, value_enum)]
    mode: Option<OnboardMode>,
    /// Remote gateway WebSocket URL
    #[arg(long, value_name = "URL")]
    remote_url: Option<String>,
    /// Remote gateway token (optional)
    #[arg(long, value_name = "TOKEN")]
    remote_token: Option<String>,
}

// ── RefreshToken ────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct RefreshTokenArgs {
    /// Path to the OpenClaw directory (default: ~/.openclaw)
    #[arg(long, value_name = "PATH")]
    openclaw_dir: Option<String>,
    /// Restart the gateway after refreshing
    #[arg(long)]
    restart: bool,
}

// ── Import ──────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct ImportArgs {
    /// Path to the OpenClaw directory to import (default: ~/.openclaw)
    #[arg(value_name = "PATH")]
    source: Option<String>,
    /// RustyClaw settings directory (default: ~/.rustyclaw)
    #[arg(long, value_name = "DIR")]
    target: Option<String>,
    /// Overwrite existing files without prompting
    #[arg(long)]
    force: bool,
    /// Dry run — show what would be imported without making changes
    #[arg(long)]
    dry_run: bool,
}

// ── Onboard ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, ValueEnum)]
enum OnboardMode {
    Local,
    Remote,
}

#[derive(Debug, Clone, ValueEnum)]
enum OnboardFlow {
    Quickstart,
    Advanced,
    Manual,
}

#[derive(Debug, Clone, ValueEnum)]
enum GatewayAuthMode {
    Token,
    Password,
}

#[derive(Debug, Clone, ValueEnum, Default)]
enum GatewayBind {
    #[default]
    Loopback,
    Lan,
    Tailnet,
    Auto,
    Custom,
}

#[derive(Debug, Args, Default)]
struct OnboardArgs {
    /// Agent workspace directory
    #[arg(long, value_name = "DIR")]
    workspace: Option<String>,
    /// Reset config + credentials + sessions before wizard
    #[arg(long)]
    reset: bool,
    /// Run without prompts
    #[arg(long)]
    non_interactive: bool,
    /// Wizard mode
    #[arg(long, value_enum)]
    mode: Option<OnboardMode>,
    /// Wizard flow
    #[arg(long, value_enum)]
    flow: Option<OnboardFlow>,
    /// Auth provider choice (e.g. apiKey, openai-api-key, anthropic-api-key, …)
    #[arg(long, value_name = "CHOICE")]
    auth_choice: Option<String>,
    /// Output JSON summary
    #[arg(long)]
    json: bool,

    // ── Provider API-key flags (mirrors openclaw) ────────────────
    /// Anthropic API key
    #[arg(long, value_name = "KEY", env = "ANTHROPIC_API_KEY")]
    anthropic_api_key: Option<String>,
    /// OpenAI API key
    #[arg(long, value_name = "KEY", env = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    /// OpenRouter API key
    #[arg(long, value_name = "KEY", env = "OPENROUTER_API_KEY")]
    openrouter_api_key: Option<String>,
    /// OpenCode Zen API key
    #[arg(long, value_name = "KEY", env = "OPENCODE_API_KEY")]
    opencode_api_key: Option<String>,
    /// Gemini API key
    #[arg(long, value_name = "KEY", env = "GEMINI_API_KEY")]
    gemini_api_key: Option<String>,
    /// xAI API key
    #[arg(long, value_name = "KEY", env = "XAI_API_KEY")]
    xai_api_key: Option<String>,

    // ── Gateway flags (inline, mirrors openclaw) ────────────────
    /// Gateway port
    #[arg(long, value_name = "PORT")]
    gateway_port: Option<u16>,
    /// Gateway bind mode
    #[arg(long, value_enum)]
    gateway_bind: Option<GatewayBind>,
    /// Gateway auth mode
    #[arg(long, value_enum)]
    gateway_auth: Option<GatewayAuthMode>,
    /// Gateway token (token auth)
    #[arg(long, value_name = "TOKEN")]
    gateway_token: Option<String>,
    /// Gateway password (password auth)
    #[arg(long, value_name = "PASSWORD")]
    gateway_password: Option<String>,
    /// Remote gateway URL
    #[arg(long, value_name = "URL")]
    remote_url: Option<String>,
    /// Remote gateway token
    #[arg(long, value_name = "TOKEN")]
    remote_token: Option<String>,
    /// Skip skills setup
    #[arg(long)]
    skip_skills: bool,
    /// Skip health check
    #[arg(long)]
    skip_health: bool,
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
enum ConfigCommands {
    /// Print a config value (dot path, e.g. model.provider)
    Get {
        /// Dot-separated config path
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// Set a config value
    Set {
        /// Dot-separated config path
        #[arg(value_name = "PATH")]
        path: String,
        /// Value to set
        #[arg(value_name = "VALUE")]
        value: String,
    },
    /// Remove a config value
    Unset {
        /// Dot-separated config path
        #[arg(value_name = "PATH")]
        path: String,
    },
}

// ── Doctor ──────────────────────────────────────────────────────────────────

#[derive(Debug, Args, Default)]
struct DoctorArgs {
    /// Accept defaults without prompting
    #[arg(long)]
    yes: bool,
    /// Apply recommended repairs without prompting
    #[arg(long)]
    repair: bool,
    /// Run without prompts (safe migrations only)
    #[arg(long)]
    non_interactive: bool,
    /// Output JSON
    #[arg(long)]
    json: bool,
}

// ── TUI ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Args, Default)]
struct TuiArgs {
    /// Gateway WebSocket URL (overrides config)
    #[arg(long = "url", value_name = "URL")]
    url: Option<String>,
    /// Gateway token
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Gateway password
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,
    /// Session key to resume
    #[arg(long, value_name = "KEY")]
    session: Option<String>,
    /// Send an initial message instead of entering interactive mode
    #[arg(long, value_name = "TEXT")]
    message: Option<String>,
}

// ── Command / Message ───────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct CommandArgs {
    /// Command text to execute
    #[arg(value_name = "COMMAND", trailing_var_arg = true)]
    command: Vec<String>,
    /// Gateway WebSocket URL (ws://…)
    #[arg(long = "gateway", alias = "url", alias = "ws", value_name = "WS_URL", env = "RUSTYCLAW_GATEWAY")]
    gateway: Option<String>,
}

// ── Status ──────────────────────────────────────────────────────────────────

#[derive(Debug, Args, Default)]
struct StatusArgs {
    /// Output JSON
    #[arg(long)]
    json: bool,
    /// Show all available information
    #[arg(long)]
    all: bool,
    /// Include usage statistics
    #[arg(long)]
    usage: bool,
    /// Verbose output
    #[arg(long, short)]
    verbose: bool,
}

// ── Gateway subcommands ─────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
enum GatewayCommands {
    /// Start the gateway (daemon/background)
    Start,
    /// Stop a running gateway
    Stop,
    /// Restart the gateway
    Restart,
    /// Show gateway status
    Status {
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Reload gateway configuration without restarting
    Reload,
    /// Run the gateway in the foreground (like `rustyclaw-gateway`)
    Run(GatewayRunArgs),
}

#[derive(Debug, Args, Default)]
struct GatewayRunArgs {
    /// Gateway port
    #[arg(long, value_name = "PORT", default_value_t = 9001)]
    port: u16,
    /// Bind mode
    #[arg(long, value_enum, default_value_t = GatewayBind::Loopback)]
    bind: GatewayBind,
    /// Auth token
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Auth mode
    #[arg(long, value_enum)]
    auth: Option<GatewayAuthMode>,
    /// Auth password
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,
    /// Overwrite existing configuration
    #[arg(long)]
    force: bool,
    /// Verbose logging
    #[arg(long, short)]
    verbose: bool,
}

// ── Skills subcommands ──────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
enum SkillsCommands {
    /// List installed skills
    List,
    /// Show info about a skill
    Info {
        /// Skill name
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Check skills for issues
    Check,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Entrypoint
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialise colour output (respects --no-color / NO_COLOR).
    rustyclaw::theme::init_color(cli.common.no_color);

    let config_path = cli.common.config_path();
    let mut config = Config::load(config_path)?;
    cli.common.apply_overrides(&mut config);

    match cli.command.unwrap_or(Commands::Tui(TuiArgs::default())) {
        // ── Setup ───────────────────────────────────────────────
        Commands::Setup(args) => {
            // If any wizard-style flag is present, delegate to onboard.
            let has_wizard_flags = args.wizard
                || args.non_interactive
                || args.mode.is_some()
                || args.remote_url.is_some()
                || args.remote_token.is_some();

            if has_wizard_flags {
                #[cfg(feature = "tui")]
                {
                    let mut secrets = open_secrets(&config)?;
                    run_onboard_wizard(&mut config, &mut secrets, false)?;
                }
                #[cfg(not(feature = "tui"))]
                {
                    eprintln!("Onboarding wizard is not available in this build. Build with --features tui to enable.");
                    std::process::exit(1);
                }
            } else {
                // Minimal setup: ensure directory skeleton + default config.
                if let Some(ws) = args.workspace {
                    config.workspace_dir = Some(ws.into());
                }
                config.ensure_dirs()?;
                config.save(None)?;
                println!("{}", rustyclaw::theme::icon_ok(
                    &format!("Initialised config + workspace at {}", rustyclaw::theme::info(&config.settings_dir.display().to_string()))
                ));
            }
        }

        // ── Onboard ─────────────────────────────────────────────
        Commands::Onboard(_args) => {
            #[cfg(feature = "tui")]
            {
                let mut secrets = open_secrets(&config)?;
                run_onboard_wizard(&mut config, &mut secrets, _args.reset)?;
            }
            #[cfg(not(feature = "tui"))]
            {
                eprintln!("Onboarding wizard is not available in this build. Build with --features tui to enable.");
                std::process::exit(1);
            }
        }

        // ── Import ──────────────────────────────────────────────
        Commands::Import(args) => {
            run_import(&args, &mut config)?;
        }

        // ── RefreshToken ────────────────────────────────────────
        Commands::RefreshToken(args) => {
            run_refresh_token(&args, &mut config)?;
        }

        // ── Configure ───────────────────────────────────────────
        Commands::Configure => {
            #[cfg(feature = "tui")]
            {
                let mut secrets = open_secrets(&config)?;
                run_onboard_wizard(&mut config, &mut secrets, false)?;
            }
            #[cfg(not(feature = "tui"))]
            {
                eprintln!("Configuration wizard is not available in this build. Build with --features tui to enable.");
                std::process::exit(1);
            }
        }

        // ── Config get / set / unset ────────────────────────────
        Commands::Config(sub) => {
            match sub {
                ConfigCommands::Get { path } => {
                    let value = config_get(&config, &path);
                    println!("{}", value);
                }
                ConfigCommands::Set { path, value } => {
                    config_set(&mut config, &path, &value)?;
                    config.save(None)?;
                    println!("{}", rustyclaw::theme::icon_ok(
                        &format!("Set {} = {}", rustyclaw::theme::accent_bright(&path), rustyclaw::theme::info(&value))
                    ));
                }
                ConfigCommands::Unset { path } => {
                    config_unset(&mut config, &path)?;
                    config.save(None)?;
                    println!("{}", rustyclaw::theme::icon_ok(
                        &format!("Unset {}", rustyclaw::theme::accent_bright(&path))
                    ));
                }
            }
        }

        // ── Doctor ──────────────────────────────────────────────
        Commands::Doctor(_args) => {
            use rustyclaw::theme as t;

            let sp = t::spinner("Running health checks…");

            let checks = vec![
                ("Config file", config.settings_dir.join("config.toml").exists()),
                ("Workspace dir", config.workspace_dir().exists()),
                ("Credentials dir", config.credentials_dir().exists()),
                ("SOUL.md", config.soul_path().exists()),
                ("Skills dir", config.skills_dir().exists()),
            ];

            // Brief pause so the spinner is visible.
            std::thread::sleep(std::time::Duration::from_millis(400));
            sp.finish_and_clear();

            let mut all_ok = true;
            for (label, passed) in &checks {
                if *passed {
                    println!("  {}", t::icon_ok(label));
                } else {
                    println!("  {}", t::icon_fail(label));
                    all_ok = false;
                }
            }
            println!();
            if all_ok {
                println!("{}", t::success("All checks passed."));
            } else {
                println!("{}", t::warn("Some checks failed."));
                println!("  Run {} or {} to fix.",
                    t::accent_bright("`rustyclaw setup`"),
                    t::accent_bright("`rustyclaw onboard`"),
                );
            }
        }

        // ── TUI ─────────────────────────────────────────────────
        Commands::Tui(_args) => {
            #[cfg(feature = "tui")]
            {
                // Apply TUI-specific overrides.
                if let Some(url) = &_args.url {
                    config.gateway_url = Some(url.clone());
                }
                // The gateway owns the secrets vault.  The TUI no longer needs
                // a local vault password — it fetches secrets via gateway messages.
                // A --password flag is forwarded to the gateway after connect if
                // the vault is locked.
                let mut app = App::new(config)?;
                if let Some(pw) = _args.password {
                    app.set_deferred_vault_password(pw);
                }
                app.run().await?;
            }
            #[cfg(not(feature = "tui"))]
            {
                eprintln!("TUI is not available in this build. Build with --features tui to enable.");
                std::process::exit(1);
            }
        }

        // ── Command / Message ───────────────────────────────────
        Commands::Command(args) => {
            let input = args.command.join(" ").trim().to_string();
            if input.is_empty() {
                anyhow::bail!("No command provided.");
            }

            if let Some(gateway_url) = args.gateway {
                let response = send_command_via_gateway(&gateway_url, &input).await?;
                println!("{}", response);
            } else {
                run_local_command(&mut config, &input)?;
            }
        }

        // ── Status ──────────────────────────────────────────────
        Commands::Status(args) => {
            print_status(&config, &args);
        }

        // ── Gateway sub-commands ────────────────────────────────
        Commands::Gateway(sub) => {
            match sub {
                GatewayCommands::Start => {
                    use rustyclaw::daemon;
                    use rustyclaw::theme as t;

                    // Under the new security model the gateway daemon owns
                    // the secrets vault — we only forward the vault password
                    // so it can open the vault and extract the API key itself.
                    let vault_password = extract_vault_password(&config);

                    let sp = t::spinner("Starting gateway…");

                    let (port, bind) = parse_gateway_defaults(&config);

                    match daemon::start(
                        &config.settings_dir,
                        port,
                        bind,
                        &[],
                        None,
                        vault_password.as_deref(),
                    ) {
                        Ok(pid) => {
                            t::spinner_ok(&sp, &format!(
                                "Gateway started (PID {}, {})",
                                pid,
                                t::info(&format!("ws://{}:{}",
                                    if bind == "loopback" { "127.0.0.1" } else { bind },
                                    port
                                )),
                            ));
                            println!("  {}", t::muted(&format!(
                                "Logs: {}",
                                daemon::log_path(&config.settings_dir).display()
                            )));
                        }
                        Err(e) => {
                            t::spinner_fail(&sp, &format!("Failed to start gateway: {}", e));
                        }
                    }
                }
                GatewayCommands::Stop => {
                    use rustyclaw::daemon;
                    use rustyclaw::theme as t;

                    let sp = t::spinner("Stopping gateway…");

                    match daemon::stop(&config.settings_dir)? {
                        daemon::StopResult::Stopped { pid } => {
                            t::spinner_ok(&sp, &format!("Gateway stopped (was PID {})", pid));
                        }
                        daemon::StopResult::WasStale { pid } => {
                            t::spinner_warn(&sp, &format!(
                                "Cleaned up stale PID file (PID {} was not running)", pid
                            ));
                        }
                        daemon::StopResult::WasNotRunning => {
                            t::spinner_warn(&sp, "Gateway is not running");
                        }
                    }
                }
                GatewayCommands::Restart => {
                    use rustyclaw::daemon;
                    use rustyclaw::theme as t;

                    // Under the new security model the gateway daemon owns
                    // the secrets vault — we only forward the vault password.
                    let vault_password = extract_vault_password(&config);

                    let sp = t::spinner("Restarting gateway…");

                    // Stop first (ignore "not running" errors).
                    let was_running = match daemon::stop(&config.settings_dir) {
                        Ok(daemon::StopResult::Stopped { pid }) => {
                            sp.set_message(format!("Stopped PID {}. Starting…", pid));
                            true
                        }
                        Ok(_) => false,
                        Err(e) => {
                            t::spinner_fail(&sp, &format!("Failed to stop: {}", e));
                            return Ok(());
                        }
                    };

                    // Brief pause to let the port free up.
                    if was_running {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }

                    let (port, bind) = parse_gateway_defaults(&config);

                    match daemon::start(
                        &config.settings_dir,
                        port,
                        bind,
                        &[],
                        None,
                        vault_password.as_deref(),
                    ) {
                        Ok(pid) => {
                            t::spinner_ok(&sp, &format!(
                                "Gateway restarted (PID {}, {})",
                                pid,
                                t::info(&format!("ws://{}:{}",
                                    if bind == "loopback" { "127.0.0.1" } else { bind },
                                    port
                                )),
                            ));
                        }
                        Err(e) => {
                            t::spinner_fail(&sp, &format!("Failed to start: {}", e));
                        }
                    }
                }
                GatewayCommands::Status { json } => {
                    use rustyclaw::daemon;
                    use rustyclaw::theme as t;

                    let url = config.gateway_url.as_deref().unwrap_or("ws://127.0.0.1:9001");
                    let status = daemon::status(&config.settings_dir);

                    if json {
                        let (running, pid) = match &status {
                            daemon::DaemonStatus::Running { pid } => (true, Some(*pid)),
                            daemon::DaemonStatus::Stale { pid } => (false, Some(*pid)),
                            daemon::DaemonStatus::Stopped => (false, None),
                        };
                        print!("{{ \"running\": {}", running);
                        if let Some(pid) = pid {
                            print!(", \"pid\": {}", pid);
                        }
                        println!(", \"gateway_url\": \"{}\" }}", url);
                    } else {
                        println!("{}", t::label_value("Gateway URL", url));
                        match status {
                            daemon::DaemonStatus::Running { pid } => {
                                println!("{}", t::label_value("Status     ",
                                    &t::success(&format!("running (PID {})", pid))));
                            }
                            daemon::DaemonStatus::Stale { pid } => {
                                println!("{}", t::label_value("Status     ",
                                    &t::warn(&format!("stale PID file (PID {} not running)", pid))));
                            }
                            daemon::DaemonStatus::Stopped => {
                                println!("{}", t::label_value("Status     ",
                                    &t::muted("stopped")));
                            }
                        }
                        let log = daemon::log_path(&config.settings_dir);
                        if log.exists() {
                            println!("{}", t::label_value("Log        ",
                                &log.display().to_string()));
                        }
                    }
                }
                GatewayCommands::Reload => {
                    use rustyclaw::theme as t;

                    let url = config.gateway_url.as_deref().unwrap_or("ws://127.0.0.1:9001");
                    let sp = t::spinner("Reloading gateway configuration\u{2026}");

                    match send_gateway_reload(url, config.totp_enabled).await {
                        Ok((provider, model)) => {
                            t::spinner_ok(&sp, &format!(
                                "Gateway reloaded: {} / {}",
                                t::info(&provider),
                                t::info(&model),
                            ));
                        }
                        Err(e) => {
                            t::spinner_fail(&sp, &format!("Reload failed: {}", e));
                        }
                    }
                }
                GatewayCommands::Run(args) => {
                    use rustyclaw::gateway::{run_gateway, GatewayOptions, ModelContext};
                    use rustyclaw::secrets::SecretsManager;
                    use tokio_util::sync::CancellationToken;

                    let host = match args.bind {
                        GatewayBind::Loopback => "127.0.0.1",
                        GatewayBind::Lan => "0.0.0.0",
                        _ => "127.0.0.1",
                    };
                    let listen = format!("{}:{}", host, args.port);
                    println!("{}", rustyclaw::theme::icon_ok(
                        &format!("RustyClaw gateway listening on {}", rustyclaw::theme::info(&format!("ws://{}", listen)))
                    ));

                    // Open the secrets vault — the gateway owns it.
                    let creds_dir = config.credentials_dir();
                    let vault = if config.secrets_password_protected {
                        let password = rpassword::prompt_password(
                            format!("{} Vault password: ", rustyclaw::theme::info("🔑")),
                        )
                        .unwrap_or_default();
                        SecretsManager::with_password(&creds_dir, password)
                    } else {
                        SecretsManager::new(&creds_dir)
                    };

                    let shared_vault: rustyclaw::gateway::SharedVault =
                        std::sync::Arc::new(tokio::sync::Mutex::new(vault));

                    // Resolve model context from the vault.
                    let model_ctx = {
                        let mut v = shared_vault.blocking_lock();
                        match ModelContext::resolve(&config, &mut v) {
                            Ok(ctx) => {
                                println!(
                                    "{} {} via {} ({})",
                                    rustyclaw::theme::icon_ok("Model:"),
                                    rustyclaw::theme::info(&ctx.model),
                                    rustyclaw::theme::info(&ctx.provider),
                                    rustyclaw::theme::muted(&ctx.base_url),
                                );
                                Some(ctx)
                            }
                            Err(err) => {
                                eprintln!("⚠ Could not resolve model context: {}", err);
                                None
                            }
                        }
                    };

                    let cancel = CancellationToken::new();

                    // Load skills for the gateway from multiple directories.
                    // Order: bundled (lowest priority) → user OpenClaw → user RustyClaw (highest)
                    let mut skills_dirs = Vec::new();
                    let openclaw_bundled = std::path::PathBuf::from("/usr/lib/node_modules/openclaw/skills");
                    if openclaw_bundled.exists() {
                        skills_dirs.push(openclaw_bundled);
                    }
                    if let Some(home) = dirs::home_dir() {
                        let openclaw_user = home.join(".openclaw/workspace/skills");
                        if openclaw_user.exists() {
                            skills_dirs.push(openclaw_user);
                        }
                    }
                    skills_dirs.push(config.skills_dir());
                    
                    let mut sm = SkillManager::with_dirs(skills_dirs);
                    if let Err(e) = sm.load_skills() {
                        eprintln!("⚠ Could not load skills: {}", e);
                    }
                    if let Some(url) = config.clawhub_url.as_deref() {
                        sm.set_registry(url, config.clawhub_token.clone());
                    }
                    let shared_skills: rustyclaw::gateway::SharedSkillManager =
                        std::sync::Arc::new(tokio::sync::Mutex::new(sm));

                    run_gateway(config, GatewayOptions { listen }, model_ctx, shared_vault, shared_skills, cancel).await?;
                }
            }
        }

        // ── Skills sub-commands ─────────────────────────────────
        Commands::Skills(sub) => {
            // Use multiple directories for skills commands too
            let mut skills_dirs = Vec::new();
            let openclaw_bundled = std::path::PathBuf::from("/usr/lib/node_modules/openclaw/skills");
            if openclaw_bundled.exists() {
                skills_dirs.push(openclaw_bundled);
            }
            if let Some(home) = dirs::home_dir() {
                let openclaw_user = home.join(".openclaw/workspace/skills");
                if openclaw_user.exists() {
                    skills_dirs.push(openclaw_user);
                }
            }
            skills_dirs.push(config.skills_dir());
            
            let mut sm = SkillManager::with_dirs(skills_dirs);
            sm.load_skills()?;

            match sub {
                SkillsCommands::List => {
                    use rustyclaw::theme as t;
                    let skills = sm.get_skills();
                    if skills.is_empty() {
                        println!("{}", t::muted("No skills installed."));
                    } else {
                        for s in skills {
                            if s.enabled {
                                println!("  {}", t::icon_ok(&t::accent_bright(&s.name)));
                            } else {
                                println!("  {}", t::icon_muted(&s.name));
                            }
                        }
                    }
                }
                SkillsCommands::Info { name } => {
                    println!("{}", rustyclaw::theme::muted(
                        &format!("Skill info for '{}' is not yet implemented.", name)
                    ));
                }
                SkillsCommands::Check => {
                    println!("{}", rustyclaw::theme::muted("Skill check is not yet implemented."));
                }
            }
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Parse the default gateway port and bind address from Config.
/// If `gateway_url` is set (e.g. "ws://127.0.0.1:9001"), extract host/port
/// from it.  Otherwise fall back to 127.0.0.1:9001.
fn parse_gateway_defaults(config: &Config) -> (u16, &str) {
    if let Some(url) = &config.gateway_url {
        if let Ok(parsed) = url::Url::parse(url) {
            let port = parsed.port().unwrap_or(9001);
            let host = parsed.host_str().unwrap_or("127.0.0.1");
            let bind = if host == "0.0.0.0" { "lan" } else { "loopback" };
            return (port, bind);
        }
    }
    (9001, "loopback")
}

/// Extract the vault password for the gateway daemon.
///
/// If the vault is password-protected, prompt the user for it.  The
/// password will be passed to the daemon via an environment variable
/// so it can open the secrets vault on startup.
fn extract_vault_password(config: &Config) -> Option<String> {
    if !config.secrets_password_protected {
        return None;
    }
    match prompt_password(&format!(
        "{} Vault password (for gateway): ",
        rustyclaw::theme::info("🔑"),
    )) {
        Ok(pw) if !pw.is_empty() => Some(pw),
        _ => None,
    }
}

/// Open the secrets vault, prompting for a password and TOTP if required.
///
/// NOTE: Under the new security model, TOTP is only verified by the
/// gateway at WebSocket connect time.  The CLI `open_secrets` is only
/// used during onboarding and ad-hoc CLI vault access.
fn open_secrets(config: &Config) -> Result<SecretsManager> {
    let mut manager = if config.secrets_password_protected {
        let pw = prompt_password("Enter secrets vault password: ")?;
        SecretsManager::with_password(config.credentials_dir(), pw)
    } else {
        SecretsManager::new(config.credentials_dir())
    };

    // If TOTP 2FA is enabled, verify before returning.
    if config.totp_enabled {
        loop {
            let code = prompt_password("Enter your 2FA code: ")?;
            match manager.verify_totp(code.trim()) {
                Ok(true) => break,
                Ok(false) => {
                    eprintln!("Invalid code. Please try again.");
                }
                Err(e) => {
                    anyhow::bail!("2FA verification failed: {}", e);
                }
            }
        }
    }

    Ok(manager)
}

fn prompt_password(prompt: &str) -> Result<String> {
    use std::io::{self, Write};
    print!("{}", prompt);
    io::stdout().flush()?;
    let input = rpassword::read_password()
        .context("Failed to read password")?;
    Ok(input.trim().to_string())
}

// ── Status ──────────────────────────────────────────────────────────────────

fn print_status(config: &Config, args: &StatusArgs) {
    if args.json {
        // Minimal JSON blob — extend as features land.
        println!("{{");
        println!("  \"settings_dir\": \"{}\",", config.settings_dir.display());
        println!("  \"workspace_dir\": \"{}\",", config.workspace_dir().display());
        if let Some(m) = &config.model {
            println!("  \"provider\": \"{}\",", m.provider);
            if let Some(model) = &m.model {
                println!("  \"model\": \"{}\",", model);
            }
        }
        if let Some(gw) = &config.gateway_url {
            println!("  \"gateway_url\": \"{}\"", gw);
        }
        println!("}}");
    } else {
        use rustyclaw::theme as t;
        println!("{}\n", t::heading("RustyClaw status"));
        println!("{}", t::label_value("Settings dir", &config.settings_dir.display().to_string()));
        println!("{}", t::label_value("Workspace   ", &config.workspace_dir().display().to_string()));
        if let Some(m) = &config.model {
            println!("{}", t::label_value("Provider    ", &m.provider));
            if let Some(model) = &m.model {
                println!("{}", t::label_value("Model       ", model));
            }
        } else {
            println!("  {} : {}", t::muted("Provider    "),
                t::warn(&format!("(not configured — run {})", t::accent_bright("`rustyclaw onboard`"))));
        }
        if let Some(gw) = &config.gateway_url {
            println!("{}", t::label_value("Gateway URL ", gw));
        }
        if args.verbose || args.all {
            println!("{}", t::label_value("SOUL.md     ", &config.soul_path().display().to_string()));
            println!("{}", t::label_value("Skills dir  ", &config.skills_dir().display().to_string()));
            println!("{}", t::label_value("Credentials ", &config.credentials_dir().display().to_string()));
        }
    }
}

// ── Config get / set / unset helpers ────────────────────────────────────────

fn config_get(config: &Config, path: &str) -> String {
    match path {
        "settings_dir" => config.settings_dir.display().to_string(),
        "workspace_dir" | "workspace" => config.workspace_dir().display().to_string(),
        "soul_path" | "soul" => config.soul_path().display().to_string(),
        "skills_dir" | "skills" => config.skills_dir().display().to_string(),
        "gateway_url" | "gateway" => config
            .gateway_url
            .as_deref()
            .unwrap_or("(not set)")
            .to_string(),
        "model.provider" | "provider" => config
            .model
            .as_ref()
            .map(|m| m.provider.clone())
            .unwrap_or_else(|| "(not set)".into()),
        "model.model" | "model" => config
            .model
            .as_ref()
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "(not set)".into()),
        "secrets_password_protected" => config.secrets_password_protected.to_string(),
        _ => format!("(unknown config path: {})", path),
    }
}

fn config_set(config: &mut Config, path: &str, value: &str) -> Result<()> {
    match path {
        "workspace_dir" | "workspace" => {
            config.workspace_dir = Some(value.into());
        }
        "soul_path" | "soul" => {
            config.soul_path = Some(value.into());
        }
        "skills_dir" | "skills" => {
            config.skills_dir = Some(value.into());
        }
        "gateway_url" | "gateway" => {
            config.gateway_url = Some(value.to_string());
        }
        "model.provider" | "provider" => {
            let m = config.model.get_or_insert_with(|| rustyclaw::config::ModelProvider {
                provider: String::new(),
                model: None,
                base_url: None,
            });
            m.provider = value.to_string();
        }
        "model.model" | "model" => {
            let m = config.model.get_or_insert_with(|| rustyclaw::config::ModelProvider {
                provider: String::new(),
                model: None,
                base_url: None,
            });
            m.model = Some(value.to_string());
        }
        _ => anyhow::bail!("Unknown config path: {}", path),
    }
    Ok(())
}

fn config_unset(config: &mut Config, path: &str) -> Result<()> {
    match path {
        "workspace_dir" | "workspace" => config.workspace_dir = None,
        "soul_path" | "soul" => config.soul_path = None,
        "skills_dir" | "skills" => config.skills_dir = None,
        "gateway_url" | "gateway" => config.gateway_url = None,
        "model" | "model.provider" | "model.model" => config.model = None,
        _ => anyhow::bail!("Unknown config path: {}", path),
    }
    Ok(())
}

// ── Refresh Token from OpenClaw ─────────────────────────────────────────────

fn run_refresh_token(args: &RefreshTokenArgs, config: &mut Config) -> Result<()> {
    use colored::Colorize;
    use std::fs;

    let openclaw_dir = args
        .openclaw_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".openclaw"));

    let token_file = openclaw_dir.join("credentials/github-copilot.token.json");

    if !token_file.exists() {
        anyhow::bail!(
            "GitHub Copilot token file not found at: {}\nRun `openclaw onboard` first to authenticate.",
            token_file.display()
        );
    }

    let content = fs::read_to_string(&token_file)
        .with_context(|| format!("Failed to read {}", token_file.display()))?;

    let json: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| "Failed to parse token file as JSON")?;

    let token = json
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Token file missing 'token' field"))?;

    let expires_at = json
        .get("expiresAt")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Token file missing 'expiresAt' field"))?;

    // expiresAt is in milliseconds, convert to seconds
    let expires_at_secs = expires_at / 1000;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if expires_at_secs <= now + 60 {
        anyhow::bail!(
            "OpenClaw token has expired. Run `openclaw onboard` to re-authenticate."
        );
    }

    let hours_left = (expires_at_secs - now) / 3600;
    let mins_left = ((expires_at_secs - now) % 3600) / 60;

    // Store in RustyClaw's vault
    let mut secrets = open_secrets(config)?;
    let session_data = serde_json::json!({
        "session_token": token,
        "expires_at": expires_at_secs,
    });
    secrets.store_secret("GITHUB_COPILOT_SESSION", &session_data.to_string())?;

    println!(
        "{} Imported GitHub Copilot session token (~{}h {}m remaining)",
        "✓".green(),
        hours_left,
        mins_left
    );

    if args.restart {
        println!("{}", "Restarting gateway...".cyan());
        // Send SIGHUP to gateway if running
        let pid_file = config.settings_dir.join("gateway.pid");
        if pid_file.exists() {
            if let Ok(pid_str) = fs::read_to_string(&pid_file) {
                if let Ok(pid) = pid_str.trim().parse::<i32>() {
                    unsafe {
                        libc::kill(pid, libc::SIGHUP);
                    }
                    println!("{} Sent reload signal to gateway (pid {})", "✓".green(), pid);
                }
            }
        } else {
            println!("{}", "Gateway not running (no pid file).".yellow());
        }
    } else {
        println!(
            "{}",
            "Restart the gateway to use the new token: rustyclaw gateway restart".dimmed()
        );
    }

    Ok(())
}

// ── Import from OpenClaw ────────────────────────────────────────────────────

fn run_import(args: &ImportArgs, config: &mut Config) -> Result<()> {
    use colored::Colorize;
    use rpassword::read_password;
    use std::fs;
    use std::io::{BufRead, Write};
    use std::path::PathBuf;

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();

    let home = dirs::home_dir().context("Could not determine home directory")?;
    let source_dir = args
        .source
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".openclaw"));

    if !source_dir.exists() {
        anyhow::bail!(
            "OpenClaw directory not found: {}\n\
             Specify the path with: rustyclaw import /path/to/.openclaw",
            source_dir.display()
        );
    }

    let target_dir = args
        .target
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".rustyclaw"));

    println!();
    println!("{}", "━".repeat(60).dimmed());
    println!("{}", "  RustyClaw Import Wizard".cyan().bold());
    println!("{}", "━".repeat(60).dimmed());
    println!();
    println!(
        "  {} {}",
        "From:".bold(),
        source_dir.display().to_string().yellow()
    );
    println!(
        "  {}   {}",
        "To:".bold(),
        target_dir.display().to_string().green()
    );

    if args.dry_run {
        println!();
        println!("{}", "  (dry run — no changes will be made)".dimmed());
    }

    // ── Detect what's available to import ───────────────────────────────
    let source_workspace = source_dir.join("workspace");
    let source_credentials = source_dir.join("credentials");
    let openclaw_config_path = source_dir.join("openclaw.json");

    let has_workspace = source_workspace.exists();
    let has_credentials = source_credentials.exists();
    let has_config = openclaw_config_path.exists();

    println!();
    println!("{}", "  Available to import:".bold());
    if has_config {
        println!("    {} Configuration (model, settings)", "•".cyan());
    }
    if has_workspace {
        println!("    {} Workspace files (SOUL.md, memory/, etc.)", "•".cyan());
    }
    if has_credentials {
        println!("    {} API credentials", "•".cyan());
    }
    println!();

    // ── Confirm import ──────────────────────────────────────────────────
    print!("{} ", "Proceed with import? [Y/n]:".cyan());
    std::io::stdout().flush()?;
    let mut response = String::new();
    reader.read_line(&mut response)?;
    if response.trim().eq_ignore_ascii_case("n") {
        println!("  {}", "Import cancelled.".yellow());
        return Ok(());
    }

    // Create target directories
    let target_workspace = target_dir.join("workspace");
    let target_credentials = target_dir.join("credentials");

    if !args.dry_run {
        fs::create_dir_all(&target_dir).context("Failed to create target directory")?;
        fs::create_dir_all(&target_workspace).context("Failed to create workspace directory")?;
        fs::create_dir_all(&target_credentials).context("Failed to create credentials directory")?;
    }

    let mut imported_count = 0;
    let mut skipped_count = 0;

    // ── Import configuration ────────────────────────────────────────────
    if has_config {
        println!();
        println!("{}", "━".repeat(60).dimmed());
        println!("{}", "Configuration".cyan().bold());
        println!("{}", "━".repeat(60).dimmed());

        if let Ok(content) = fs::read_to_string(&openclaw_config_path) {
            if let Ok(oc_config) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(model_str) = oc_config
                    .pointer("/agents/defaults/model/primary")
                    .and_then(|v| v.as_str())
                {
                    let parts: Vec<&str> = model_str.splitn(2, '/').collect();
                    if parts.len() == 2 {
                        config.model = Some(rustyclaw::config::ModelProvider {
                            provider: parts[0].to_string(),
                            model: Some(parts[1].to_string()),
                            base_url: None,
                        });
                        println!("  {} Model: {}", "✓".green(), model_str.cyan());
                        imported_count += 1;
                    }
                }
            }
        }
    }

    // ── Import workspace files ──────────────────────────────────────────
    if has_workspace {
        println!();
        println!("{}", "━".repeat(60).dimmed());
        println!("{}", "Workspace Files".cyan().bold());
        println!("{}", "━".repeat(60).dimmed());

        print!("{} ", "Import workspace files (SOUL.md, AGENTS.md, memory/, etc.)? [Y/n]:".cyan());
        std::io::stdout().flush()?;
        let mut response = String::new();
        reader.read_line(&mut response)?;

        if !response.trim().eq_ignore_ascii_case("n") {
            let workspace_files = [
                "SOUL.md",
                "AGENTS.md",
                "TOOLS.md",
                "USER.md",
                "IDENTITY.md",
                "HEARTBEAT.md",
                "MEMORY.md",
            ];

            for file in &workspace_files {
                let src = source_workspace.join(file);
                let dst = target_workspace.join(file);

                if src.exists() {
                    if dst.exists() && !args.force {
                        println!("  {} {} (exists, use --force to overwrite)", "⊘".yellow(), file);
                        skipped_count += 1;
                    } else {
                        if !args.dry_run {
                            fs::copy(&src, &dst)
                                .with_context(|| format!("Failed to copy {}", file))?;
                        }
                        println!("  {} {}", "✓".green(), file);
                        imported_count += 1;
                    }
                }
            }

            // Import memory/ directory
            let src_memory = source_workspace.join("memory");
            let dst_memory = target_workspace.join("memory");
            if src_memory.exists() && src_memory.is_dir() {
                if !args.dry_run {
                    fs::create_dir_all(&dst_memory)?;
                }

                let mut memory_count = 0;
                for entry in fs::read_dir(&src_memory)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_file() {
                        let file_name = path.file_name().unwrap();
                        let dst_file = dst_memory.join(file_name);

                        if dst_file.exists() && !args.force {
                            skipped_count += 1;
                        } else {
                            if !args.dry_run {
                                fs::copy(&path, &dst_file)?;
                            }
                            memory_count += 1;
                        }
                    }
                }
                if memory_count > 0 {
                    println!("  {} memory/ ({} files)", "✓".green(), memory_count);
                    imported_count += memory_count;
                }
            }

            // Extract agent name from IDENTITY.md as default, then prompt
            let mut default_name = String::new();
            let identity_path = source_workspace.join("IDENTITY.md");
            if identity_path.exists() {
                if let Ok(content) = fs::read_to_string(&identity_path) {
                    // Look for "- **Name:** <name>" pattern
                    for line in content.lines() {
                        let line = line.trim();
                        if line.starts_with("- **Name:**") || line.starts_with("**Name:**") {
                            if let Some(name) = line.split(":**").nth(1) {
                                let name = name.trim();
                                if !name.is_empty() {
                                    default_name = name.to_string();
                                }
                            }
                            break;
                        }
                    }
                }
            }

            // Prompt for agent name with default from IDENTITY.md
            println!();
            if default_name.is_empty() {
                print!("{} ", "Agent name:".cyan());
            } else {
                print!("{} ", format!("Agent name [{}]:", default_name).cyan());
            }
            std::io::stdout().flush()?;
            let mut name_input = String::new();
            reader.read_line(&mut name_input)?;
            let name_input = name_input.trim();

            if name_input.is_empty() && !default_name.is_empty() {
                config.agent_name = default_name.clone();
                println!("  {} Agent name: {}", "✓".green(), default_name.cyan());
            } else if !name_input.is_empty() {
                config.agent_name = name_input.to_string();
                println!("  {} Agent name: {}", "✓".green(), name_input.cyan());
            } else {
                println!("  {}", "Using default agent name.".dimmed());
            }
        } else {
            println!("  {}", "Skipping workspace files.".dimmed());
        }
    }

    // ── Credentials import ──────────────────────────────────────────────
    if has_credentials {
        println!();
        println!("{}", "━".repeat(60).dimmed());
        println!("{}", "Credentials".cyan().bold());
        println!("{}", "━".repeat(60).dimmed());

        // List available credentials
        let credential_files = [
            ("github-copilot.token.json", "GitHub Copilot"),
            ("anthropic.key", "Anthropic"),
            ("openai.key", "OpenAI"),
            ("openrouter.key", "OpenRouter"),
            ("opencode.key", "OpenCode Zen"),
            ("gemini.key", "Gemini"),
            ("xai.key", "xAI"),
        ];

        let mut found_creds: Vec<(&str, &str)> = Vec::new();
        for (file, name) in &credential_files {
            if source_credentials.join(file).exists() {
                found_creds.push((file, name));
            }
        }

        if found_creds.is_empty() {
            println!("  {}", "No credentials found to import.".dimmed());
        } else {
            println!("  Found credentials:");
            for (_, name) in &found_creds {
                println!("    {} {}", "•".cyan(), name);
            }
            println!();

            print!("{} ", "Import these credentials? [Y/n]:".cyan());
            std::io::stdout().flush()?;
            let mut response = String::new();
            reader.read_line(&mut response)?;

            if !response.trim().eq_ignore_ascii_case("n") {
                // Need to release stdin lock before password prompt
                drop(reader);

                // ── Vault security setup ────────────────────────────────
                println!();
                println!("{}", "━".repeat(60).dimmed());
                println!("{}", "Vault Security Setup".cyan().bold());
                println!("{}", "━".repeat(60).dimmed());
                println!();
                println!("  Your credentials will be stored in an encrypted vault.");
                println!("  You can add a password for additional security.");
                println!();
                println!("  {}  With a password, you'll need to enter it each time", "⚠".yellow());
                println!("     you start the agent. Without one, an auto-generated");
                println!("     key file protects the vault instead.");
                println!();

                let mut secrets = SecretsManager::new(&target_credentials);

                // Password setup
                print!("{} ", "Vault password (leave blank to skip):".cyan());
                std::io::stdout().flush()?;
                let password = read_password().unwrap_or_default();

                if password.trim().is_empty() {
                    println!("  {}", "✓ Using auto-generated key file.".green());
                    config.secrets_password_protected = false;
                } else {
                    print!("{} ", "Confirm password:".cyan());
                    std::io::stdout().flush()?;
                    let confirm = read_password().unwrap_or_default();

                    if password != confirm {
                        anyhow::bail!("Passwords do not match. Import cancelled.");
                    }

                    secrets.set_password(password);
                    config.secrets_password_protected = true;
                    println!("  {}", "✓ Vault will be password-protected.".green());
                }

                // Re-acquire stdin for TOTP setup
                let stdin = std::io::stdin();
                let mut reader = stdin.lock();

                // TOTP setup
                println!();
                println!("{}", "Two-Factor Authentication (optional)".cyan().bold());
                println!();
                println!("  Add TOTP 2FA using any authenticator app.");
                println!();

                print!("{} ", "Enable 2FA? [y/N]:".cyan());
                std::io::stdout().flush()?;
                let mut response = String::new();
                reader.read_line(&mut response)?;

                if response.trim().eq_ignore_ascii_case("y") {
                    // Initialize vault
                    if !args.dry_run {
                        secrets.store_secret("__init", "")?;
                        secrets.delete_secret("__init")?;
                    }

                    let account = std::env::var("USER")
                        .or_else(|_| std::env::var("USERNAME"))
                        .unwrap_or_else(|_| "user".to_string());
                    let agent_name = config.agent_name.clone();

                    if !args.dry_run {
                        let otpauth_url = secrets.setup_totp_with_issuer(&account, &agent_name)?;

                        println!();
                        println!("  {}", "Scan this QR code:".bold());
                        println!();
                        print_qr_code_import(&otpauth_url);
                        println!();
                        println!("  {}", otpauth_url.dimmed());
                        println!();

                        loop {
                            print!("{} ", "Enter 6-digit code to verify:".cyan());
                            std::io::stdout().flush()?;
                            let mut code = String::new();
                            reader.read_line(&mut code)?;
                            let code = code.trim();

                            if code.is_empty() {
                                println!("  {}", "⚠ 2FA setup cancelled.".yellow());
                                secrets.remove_totp()?;
                                break;
                            }

                            match secrets.verify_totp(code) {
                                Ok(true) => {
                                    config.totp_enabled = true;
                                    println!("  {}", "✓ 2FA enabled.".green());
                                    break;
                                }
                                Ok(false) => {
                                    println!("  {}", "⚠ Invalid code. Try again:".yellow());
                                }
                                Err(e) => {
                                    println!("  {}", format!("⚠ Error: {}. 2FA not enabled.", e).yellow());
                                    secrets.remove_totp()?;
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    println!("  {}", "Skipping 2FA.".dimmed());
                }

                // Now import the credentials
                println!();
                println!("{}", "Importing credentials...".cyan());

                let secret_map = [
                    ("github-copilot.token.json", "GITHUB_COPILOT_TOKEN"),
                    ("anthropic.key", "ANTHROPIC_API_KEY"),
                    ("openai.key", "OPENAI_API_KEY"),
                    ("openrouter.key", "OPENROUTER_API_KEY"),
                    ("opencode.key", "OPENCODE_API_KEY"),
                    ("gemini.key", "GEMINI_API_KEY"),
                    ("xai.key", "XAI_API_KEY"),
                ];

                for (file, secret_name) in &secret_map {
                    // GitHub Copilot: try to import the session token directly
                    if *file == "github-copilot.token.json" {
                        let src = source_credentials.join(file);
                        if src.exists() {
                            if let Ok(content) = fs::read_to_string(&src) {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                                    let token = json.get("token").and_then(|v| v.as_str());
                                    let expires_at = json.get("expiresAt").and_then(|v| v.as_i64());

                                    if let (Some(token), Some(expires_at)) = (token, expires_at) {
                                        // expiresAt is in milliseconds, convert to seconds
                                        let expires_at_secs = expires_at / 1000;
                                        let now = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs() as i64;

                                        if expires_at_secs > now + 300 {
                                            // Token still valid for at least 5 minutes
                                            // Store as JSON with token and expiration
                                            let session_data = serde_json::json!({
                                                "session_token": token,
                                                "expires_at": expires_at_secs,
                                            });
                                            if !args.dry_run {
                                                secrets.store_secret("GITHUB_COPILOT_SESSION", &session_data.to_string())?;
                                            }
                                            let hours_left = (expires_at_secs - now) / 3600;
                                            println!("  {} {} (session token, ~{}h remaining)", "✓".green(), secret_name, hours_left);
                                            imported_count += 1;
                                            continue;
                                        } else {
                                            println!("  {} {} (session expired, needs re-auth)", "⊘".yellow(), secret_name);
                                        }
                                    }
                                }
                            }
                        }
                        // Fall through to re-auth prompt below
                        skipped_count += 1;
                        continue;
                    }

                    let src = source_credentials.join(file);
                    if src.exists() {
                        if let Ok(content) = fs::read_to_string(&src) {
                            let token = if file.ends_with(".json") {
                                serde_json::from_str::<serde_json::Value>(&content)
                                    .ok()
                                    .and_then(|json| {
                                        json.get("access_token")
                                            .or_else(|| json.get("token"))
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .or_else(|| Some(content.trim().to_string()))
                            } else {
                                Some(content.trim().to_string())
                            };

                            if let Some(token) = token {
                                if !token.is_empty() && !args.dry_run {
                                    secrets.store_secret(secret_name, &token)?;
                                    println!("  {} {}", "✓".green(), secret_name);
                                    imported_count += 1;
                                }
                            }
                        }
                    }
                }

                // Prompt for GitHub Copilot re-authentication
                println!();
                println!("{}", "GitHub Copilot Authentication".cyan().bold());
                println!("  OpenClaw stores session tokens that can't be migrated.");
                println!("  You'll need to re-authenticate with GitHub.");
                println!();
                print!("{} ", "Authenticate with GitHub Copilot now? [Y/n]:".cyan());
                std::io::stdout().flush()?;
                let mut response = String::new();
                reader.read_line(&mut response)?;

                if !response.trim().eq_ignore_ascii_case("n") {
                    // Re-use the device flow auth
                    use providers::GITHUB_COPILOT_DEVICE_FLOW;
                    let device_config = &GITHUB_COPILOT_DEVICE_FLOW;

                    println!();
                    println!("{}", "Starting GitHub device flow...".cyan());

                    let handle = tokio::runtime::Handle::current();
                    match tokio::task::block_in_place(|| {
                        handle.block_on(providers::start_device_flow(device_config))
                    }) {
                        Ok(auth_response) => {
                            println!();
                            println!("  {}", "Please complete the following steps:".bold());
                            println!();
                            println!("  1. Visit: {}", auth_response.verification_uri.cyan());
                            println!("  2. Enter code: {}", auth_response.user_code.cyan().bold());
                            println!();

                            print!("{} ", "Press Enter after completing authorization (or type 'cancel'):".cyan());
                            std::io::stdout().flush()?;
                            let mut response = String::new();
                            reader.read_line(&mut response)?;

                            if !response.trim().eq_ignore_ascii_case("cancel") && !response.trim().eq_ignore_ascii_case("c") {
                                println!("  {}", "Waiting for authorization...".dimmed());

                                let interval = std::time::Duration::from_secs(auth_response.interval);
                                let max_attempts = (auth_response.expires_in / auth_response.interval).max(10);

                                let mut token: Option<String> = None;
                                for _attempt in 0..max_attempts {
                                    match tokio::task::block_in_place(|| {
                                        handle.block_on(providers::poll_device_token(device_config, &auth_response.device_code))
                                    }) {
                                        Ok(Some(access_token)) => {
                                            token = Some(access_token);
                                            break;
                                        }
                                        Ok(None) => {
                                            print!(".");
                                            std::io::stdout().flush()?;
                                            std::thread::sleep(interval);
                                        }
                                        Err(e) => {
                                            println!();
                                            println!("  {}", format!("⚠ Authentication failed: {}", e).yellow());
                                            break;
                                        }
                                    }
                                }
                                println!();

                                if let Some(access_token) = token {
                                    if !args.dry_run {
                                        secrets.store_secret("GITHUB_COPILOT_TOKEN", &access_token)?;
                                    }
                                    println!("  {}", "✓ GitHub Copilot authenticated!".green());
                                    imported_count += 1;
                                } else {
                                    println!("  {}", "⚠ Authentication timed out.".yellow());
                                }
                            } else {
                                println!("  {}", "Skipping GitHub Copilot.".dimmed());
                            }
                        }
                        Err(e) => {
                            println!("  {}", format!("⚠ Failed to start device flow: {}", e).yellow());
                        }
                    }
                } else {
                    println!("  {}", "Skipping GitHub Copilot.".dimmed());
                }
            } else {
                println!("  {}", "Skipping credentials.".dimmed());
            }
        }
    }

    // ── Save config ─────────────────────────────────────────────────────
    if !args.dry_run {
        config.settings_dir = target_dir.clone();
        config.workspace_dir = Some(target_workspace.clone());
        config.credentials_dir = Some(target_credentials);
        config.save(Some(target_dir.join("config.toml")))?;
    }

    // ── Summary ─────────────────────────────────────────────────────────
    println!();
    println!("{}", "━".repeat(60).dimmed());
    println!(
        "{} Import complete: {} items imported, {} skipped",
        "✓".green().bold(),
        imported_count.to_string().green(),
        skipped_count.to_string().yellow()
    );

    if config.secrets_password_protected {
        println!("  {} Vault is password-protected", "🔒");
    }
    if config.totp_enabled {
        println!("  {} 2FA is enabled", "🔐");
    }

    if args.dry_run {
        println!();
        println!("{}", "Run without --dry-run to apply changes.".dimmed());
    } else {
        println!();
        println!("{} Saved to {}", "✓".green(), target_dir.join("config.toml").display());
        println!();
        println!(
            "{} Run {} to launch your agent!",
            "→".cyan(),
            "rustyclaw tui".green()
        );
    }

    Ok(())
}

/// Print a QR code to the terminal (simplified version for import)
fn print_qr_code_import(data: &str) {
    use qrcode::{QrCode, render::unicode};

    if let Ok(code) = QrCode::new(data.as_bytes()) {
        let image = code
            .render::<unicode::Dense1x2>()
            .dark_color(unicode::Dense1x2::Light)
            .light_color(unicode::Dense1x2::Dark)
            .build();
        for line in image.lines() {
            println!("    {}", line);
        }
    } else {
        println!("  (Could not generate QR code)");
    }
}

fn run_local_command(config: &mut Config, input: &str) -> Result<()> {
    let mut secrets_manager = open_secrets(config)?;
    let skills_dir = config.skills_dir();
    let mut skill_manager = SkillManager::new(skills_dir);
    skill_manager.load_skills()?;

    let mut context = CommandContext {
        secrets_manager: &mut secrets_manager,
        skill_manager: &mut skill_manager,
        config,
    };

    let response = handle_command(input, &mut context);
    if response.action == CommandAction::ClearMessages {
        for message in response.messages {
            println!("{}", message);
        }
        return Ok(());
    }

    if response.action == CommandAction::Quit {
        return Ok(());
    }

    for message in response.messages {
        println!("{}", message);
    }

    Ok(())
}

/// Send a reload command to the running gateway and wait for the result.
async fn send_gateway_reload(gateway_url: &str, totp_enabled: bool) -> Result<(String, String)> {
    let url = Url::parse(gateway_url).context("Invalid gateway URL")?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(url.to_string())
        .await
        .context("Failed to connect to gateway. Is it running?")?;
    let (mut writer, mut reader) = ws_stream.split();

    // Handle auth challenge if TOTP is enabled
    if totp_enabled {
        while let Some(msg) = reader.next().await {
            let msg = msg.context("Gateway read error")?;
            if let Message::Text(text) = msg {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                    let frame_type = val.get("type").and_then(|t| t.as_str());
                    if frame_type == Some("auth_challenge") {
                        let code = rpassword::prompt_password(
                            format!("{} 2FA code: ", rustyclaw::theme::info("🔑")),
                        )
                        .unwrap_or_default();
                        let auth = serde_json::json!({
                            "type": "auth_response",
                            "code": code.trim(),
                        });
                        writer.send(Message::Text(auth.to_string().into())).await?;
                        continue;
                    }
                    if frame_type == Some("auth_result") {
                        let ok = val.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                        if !ok {
                            let msg = val.get("message").and_then(|m| m.as_str()).unwrap_or("Auth failed");
                            anyhow::bail!("{}", msg);
                        }
                        break; // Auth succeeded, continue to send reload
                    }
                    if frame_type == Some("hello") {
                        break; // No auth needed (shouldn't happen if totp_enabled)
                    }
                }
            }
        }
    }

    // Wait for hello frame (skip status frames)
    loop {
        match reader.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                    let frame_type = val.get("type").and_then(|t| t.as_str());
                    if frame_type == Some("hello") || frame_type == Some("auth_challenge") {
                        if frame_type == Some("auth_challenge") {
                            // Handle TOTP even when totp_enabled was false in config
                            let code = rpassword::prompt_password(
                                format!("{} 2FA code: ", rustyclaw::theme::info("🔑")),
                            )
                            .unwrap_or_default();
                            let auth = serde_json::json!({
                                "type": "auth_response",
                                "code": code.trim(),
                            });
                            writer.send(Message::Text(auth.to_string().into())).await?;
                            continue;
                        }
                        break;
                    }
                    if frame_type == Some("auth_result") {
                        let ok = val.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                        if !ok {
                            let msg = val.get("message").and_then(|m| m.as_str()).unwrap_or("Auth failed");
                            anyhow::bail!("{}", msg);
                        }
                        // auth ok, wait for hello
                        continue;
                    }
                    // Skip status frames while waiting for hello
                    if frame_type == Some("status") {
                        continue;
                    }
                }
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => anyhow::bail!("Gateway error: {}", e),
            None => anyhow::bail!("Gateway closed before hello"),
        }
    }

    // Drain remaining status frames briefly
    let drain_timeout = tokio::time::sleep(std::time::Duration::from_millis(500));
    tokio::pin!(drain_timeout);
    loop {
        tokio::select! {
            _ = &mut drain_timeout => break,
            msg = reader.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                            let ft = val.get("type").and_then(|t| t.as_str());
                            if ft == Some("status") || ft == Some("model_configured") || ft == Some("model_ready") || ft == Some("model_error") {
                                continue; // Skip status frames
                            }
                        }
                        break; // Non-status frame, stop draining
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => anyhow::bail!("Gateway error during drain: {}", e),
                    None => anyhow::bail!("Gateway closed unexpectedly"),
                }
            }
        }
    }

    // Send reload command
    let reload = serde_json::json!({ "type": "reload" });
    writer.send(Message::Text(reload.to_string().into())).await
        .context("Failed to send reload command")?;

    // Wait for reload_result
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => {
                anyhow::bail!("Timeout waiting for reload result");
            }
            msg = reader.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                            let frame_type = val.get("type").and_then(|t| t.as_str());
                            if frame_type == Some("reload_result") {
                                let ok = val.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                                if ok {
                                    let provider = val.get("provider").and_then(|p| p.as_str()).unwrap_or("unknown").to_string();
                                    let model = val.get("model").and_then(|m| m.as_str()).unwrap_or("unknown").to_string();
                                    // Close cleanly
                                    let _ = writer.send(Message::Close(None)).await;
                                    return Ok((provider, model));
                                } else {
                                    let msg = val.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
                                    anyhow::bail!("{}", msg);
                                }
                            }
                            // Skip other frames (status updates from reload)
                            continue;
                        }
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => anyhow::bail!("Gateway error: {}", e),
                    None => anyhow::bail!("Gateway closed without reload result"),
                }
            }
        }
    }
}

async fn send_command_via_gateway(gateway_url: &str, command: &str) -> Result<String> {
    let url = Url::parse(gateway_url).context("Invalid gateway URL")?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(url.to_string())
        .await
        .context("Failed to connect to gateway")?;
    let (mut writer, mut reader) = ws_stream.split();
    writer
        .send(Message::Text(command.to_string().into()))
        .await
        .context("Failed to send command")?;

    while let Some(message) = reader.next().await {
        let message = message.context("Gateway read error")?;
        if let Message::Text(text) = message {
            return Ok(text.to_string());
        }
    }

    anyhow::bail!("Gateway closed without responding")
}

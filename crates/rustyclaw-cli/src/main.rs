//! `rustyclaw` — main CLI entry point for RustyClaw.
//!
//! Provides the `rustyclaw` binary (interactive chat / one-shot commands) and
//! the `rustyclaw-gateway` daemon binary.

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use rustyclaw_core::args::CommonArgs;
use rustyclaw_core::config::Config;
use rustyclaw_core::daemon;
use rustyclaw_core::skills::SkillManager;
use rustyclaw_onboard::{OnboardArgs as WizardArgs, run_onboard_wizard};

mod commands;

use commands::clawhub::ClawHubCommands;
use commands::config::ConfigCommands;
use commands::gateway_client::{
    AskArgs, handle_ask, run_local_command, send_command_via_gateway, send_gateway_reload,
};
use commands::shared::{extract_vault_password, open_secrets};
use commands::swarm::SwarmCommands;

// ── Top-level CLI ───────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(
    name = "rustyclaw",
    version,
    about = "RustyClaw — lightweight agentic assistant",
    long_about = "RustyClaw — a super-lightweight super-capable agentic tool with improved security.\n\n\
                  Run without a subcommand to launch the TUI. Use `rustyclaw desktop` for the desktop client."
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
    Import(commands::import::ImportArgs),

    /// Interactive configuration wizard (models, gateway, skills)
    Configure,

    /// Config helpers: get / set / unset
    #[command(subcommand)]
    Config(commands::config::ConfigCommands),

    /// Health checks + quick fixes for gateway and configuration
    Doctor(DoctorArgs),

    /// Launch the terminal UI
    #[command(alias = "ui")]
    Tui(TuiArgs),

    /// Launch the desktop UI
    Desktop(DesktopArgs),

    /// Send a one-shot message / slash-command
    #[command(alias = "cmd", alias = "run", alias = "message")]
    Command(CommandArgs),

    /// Send a prompt to the model and print the response (headless mode)
    #[command(alias = "chat", alias = "prompt")]
    Ask(AskArgs),

    /// Show system status (gateway, model, workspace)
    Status(commands::status::StatusArgs),

    /// Gateway management (start / stop / restart / status)
    #[command(subcommand)]
    Gateway(GatewayCommands),

    /// List / manage skills
    #[command(subcommand)]
    Skills(SkillsCommands),

    /// Refresh the GitHub Copilot session token from OpenClaw
    #[command(alias = "refresh")]
    RefreshToken(commands::refresh_token::RefreshTokenArgs),

    /// ClawHub skill registry commands (search, install, publish, …)
    #[command(name = "clawhub", alias = "hub", alias = "registry")]
    ClawHub(ClawHubCommands),

    /// Multi-agent swarm management (create, list, status, send, stop)
    #[command(subcommand)]
    Swarm(SwarmCommands),
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
#[command(after_help = "\
SECURITY NOTE:
  API keys passed via CLI flags are visible in `ps aux` and process logs.
  For scripted/automated setups, prefer environment variables:

    ANTHROPIC_API_KEY=sk-xxx rustyclaw onboard
    OPENROUTER_API_KEY=sk-xxx rustyclaw onboard

  Or use the interactive wizard which prompts securely.
")]
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
    // ⚠️ CLI flags are visible in `ps aux`. Prefer env vars for security:
    //    ANTHROPIC_API_KEY, OPENAI_API_KEY, OPENROUTER_API_KEY, etc.
    /// Anthropic API key (prefer ANTHROPIC_API_KEY env var)
    #[arg(long, value_name = "KEY", env = "ANTHROPIC_API_KEY", hide = true)]
    anthropic_api_key: Option<String>,
    /// OpenAI API key (prefer OPENAI_API_KEY env var)
    #[arg(long, value_name = "KEY", env = "OPENAI_API_KEY", hide = true)]
    openai_api_key: Option<String>,
    /// OpenRouter API key (prefer OPENROUTER_API_KEY env var)
    #[arg(long, value_name = "KEY", env = "OPENROUTER_API_KEY", hide = true)]
    openrouter_api_key: Option<String>,
    /// OpenCode Zen API key (prefer OPENCODE_API_KEY env var)
    #[arg(long, value_name = "KEY", env = "OPENCODE_API_KEY", hide = true)]
    opencode_api_key: Option<String>,
    /// Gemini API key (prefer GEMINI_API_KEY env var)
    #[arg(long, value_name = "KEY", env = "GEMINI_API_KEY", hide = true)]
    gemini_api_key: Option<String>,
    /// xAI API key (prefer XAI_API_KEY env var)
    #[arg(long, value_name = "KEY", env = "XAI_API_KEY", hide = true)]
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
    /// Skip the interactive connection dialog and use the saved/default
    /// gateway URL when --url is not provided.
    #[arg(long = "no-dialog", alias = "auto-connect")]
    no_dialog: bool,
}

#[derive(Debug, Args, Default)]
struct DesktopArgs {
    /// Gateway URL (overrides config)
    #[arg(long = "url", value_name = "URL")]
    url: Option<String>,
    /// Skip the connection dialog on startup and connect to the saved
    /// or default URL automatically.
    #[arg(long = "no-dialog", alias = "auto-connect")]
    no_dialog: bool,
}

// ── Command / Message ───────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct CommandArgs {
    /// Command text to execute
    #[arg(value_name = "COMMAND", trailing_var_arg = true)]
    command: Vec<String>,
    /// Gateway WebSocket URL (ws://…)
    #[arg(
        long = "gateway",
        alias = "url",
        alias = "ws",
        value_name = "WS_URL",
        env = "RUSTYCLAW_GATEWAY"
    )]
    gateway: Option<String>,
}

// ── Ask (headless mode) ─────────────────────────────────────────────────────

// ── Gateway subcommands ─────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
enum GatewayCommands {
    /// Start the gateway (daemon/background)
    Start {
        /// Log level filter for the gateway (e.g., "debug", "rustyclaw=debug,info")
        #[arg(long, value_name = "LEVEL")]
        log_level: Option<String>,
    },
    /// Stop a running gateway
    Stop,
    /// Restart the gateway
    Restart {
        /// Log level filter for the gateway (e.g., "debug", "rustyclaw=debug,info")
        #[arg(long, value_name = "LEVEL")]
        log_level: Option<String>,
    },
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
    /// Verbose logging (equivalent to --log-level=debug)
    #[arg(long, short)]
    verbose: bool,
    /// Log level filter (e.g., "debug", "rustyclaw=debug,info", "rustyclaw_core::providers=debug")
    #[arg(long, value_name = "LEVEL")]
    log_level: Option<String>,
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

/// Reconstruct the global `CommonArgs` flags so they can be forwarded to a
/// spawned client binary (`rustyclaw-tui` / `rustyclaw-desktop`), which parse
/// the same flags via their own flattened `CommonArgs`.
fn common_flags(c: &CommonArgs) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(p) = &c.config {
        v.push("--config".to_string());
        v.push(p.display().to_string());
    }
    if let Some(p) = &c.settings_dir {
        v.push("--settings-dir".to_string());
        v.push(p.display().to_string());
    }
    if let Some(p) = &c.profile {
        v.push("--profile".to_string());
        v.push(p.clone());
    }
    if c.no_color {
        v.push("--no-color".to_string());
    }
    if let Some(p) = &c.soul {
        v.push("--soul".to_string());
        v.push(p.display().to_string());
    }
    if let Some(p) = &c.skills_dir {
        v.push("--skills-dir".to_string());
        v.push(p.display().to_string());
    }
    if c.no_secrets {
        v.push("--no-secrets".to_string());
    }
    if let Some(g) = &c.gateway {
        v.push("--gateway".to_string());
        v.push(g.clone());
    }
    v
}

/// Spawn a sibling client binary in the foreground, forwarding `args`, and
/// propagate its exit code on failure.
fn launch_client(name: &str, args: &[String]) -> Result<()> {
    let status = daemon::run_foreground_named(name, args)?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize structured logging from environment variables.
    // Set RUSTYCLAW_LOG=debug or RUST_LOG=debug for verbose output.
    // (The `tui`/`desktop` subcommands spawn separate binaries that configure
    // their own logging.)
    rustyclaw_core::logging::init_from_env();

    // Initialise colour output (respects --no-color / NO_COLOR).
    rustyclaw_core::theme::init_color(cli.common.no_color);

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
                let mut secrets = open_secrets(&config)?;
                let tui_args = WizardArgs {
                    openrouter_api_key: None,
                    anthropic_api_key: None,
                    openai_api_key: None,
                    gemini_api_key: None,
                    xai_api_key: None,
                    reset: false,
                    non_interactive: args.non_interactive,
                };
                run_onboard_wizard(&mut config, &mut secrets, Some(tui_args))?;
                // Optional agent setup step
                let ws_dir = config.workspace_dir();
                match rustyclaw_core::tools::agent_setup::exec_agent_setup(
                    &serde_json::json!({}),
                    &ws_dir,
                ) {
                    Ok(msg) => println!("{}", rustyclaw_core::theme::icon_ok(&msg)),
                    Err(e) => println!(
                        "{}",
                        rustyclaw_core::theme::icon_fail(&format!("Agent setup failed: {}", e))
                    ),
                }
            } else {
                // Minimal setup: ensure directory skeleton + default config.
                if let Some(ws) = args.workspace {
                    config.workspace_dir = Some(ws.into());
                }
                config.ensure_dirs()?;
                config.save(None)?;
                println!(
                    "{}",
                    rustyclaw_core::theme::icon_ok(&format!(
                        "Initialised config + workspace at {}",
                        rustyclaw_core::theme::info(&config.settings_dir.display().to_string())
                    ))
                );
                // Optional agent setup step
                let ws_dir = config.workspace_dir();
                match rustyclaw_core::tools::agent_setup::exec_agent_setup(
                    &serde_json::json!({}),
                    &ws_dir,
                ) {
                    Ok(msg) => println!("{}", rustyclaw_core::theme::icon_ok(&msg)),
                    Err(e) => println!(
                        "{}",
                        rustyclaw_core::theme::icon_fail(&format!("Agent setup failed: {}", e))
                    ),
                }
            }
        }

        // ── Onboard ─────────────────────────────────────────────
        Commands::Onboard(args) => {
            let mut secrets = open_secrets(&config)?;
            let tui_args = WizardArgs {
                openrouter_api_key: args.openrouter_api_key.clone(),
                anthropic_api_key: args.anthropic_api_key.clone(),
                openai_api_key: args.openai_api_key.clone(),
                gemini_api_key: args.gemini_api_key.clone(),
                xai_api_key: args.xai_api_key.clone(),
                reset: args.reset,
                non_interactive: args.non_interactive,
            };
            run_onboard_wizard(&mut config, &mut secrets, Some(tui_args))?;
        }

        // ── Import ──────────────────────────────────────────────
        Commands::Import(args) => {
            commands::run_import(&args, &mut config)?;
        }

        // ── RefreshToken ────────────────────────────────────────
        Commands::RefreshToken(args) => {
            commands::run_refresh_token(&args, &mut config)?;
        }

        // ── Configure ───────────────────────────────────────────
        Commands::Configure => {
            let mut secrets = open_secrets(&config)?;
            run_onboard_wizard(&mut config, &mut secrets, None)?;
        }

        // ── Config get / set / unset ────────────────────────────
        Commands::Config(sub) => match sub {
            ConfigCommands::Get { path } => {
                let value = commands::config_get(&config, &path);
                println!("{}", value);
            }
            ConfigCommands::Set { path, value } => {
                commands::config_set(&mut config, &path, &value)?;
                config.save(None)?;
                println!(
                    "{}",
                    rustyclaw_core::theme::icon_ok(&format!(
                        "Set {} = {}",
                        rustyclaw_core::theme::accent_bright(&path),
                        rustyclaw_core::theme::info(&value)
                    ))
                );
            }
            ConfigCommands::Unset { path } => {
                commands::config_unset(&mut config, &path)?;
                config.save(None)?;
                println!(
                    "{}",
                    rustyclaw_core::theme::icon_ok(&format!(
                        "Unset {}",
                        rustyclaw_core::theme::accent_bright(&path)
                    ))
                );
            }
        },

        // ── Doctor ──────────────────────────────────────────────
        Commands::Doctor(_args) => {
            use rustyclaw_core::theme as t;

            let sp = t::spinner("Running health checks…");

            let checks = vec![
                (
                    "Config file",
                    config.settings_dir.join("config.toml").exists(),
                ),
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
                println!(
                    "  Run {} or {} to fix.",
                    t::accent_bright("`rustyclaw setup`"),
                    t::accent_bright("`rustyclaw onboard`"),
                );
            }
        }

        // ── TUI ─────────────────────────────────────────────────
        // The TUI is a standalone binary (`rustyclaw-tui`); locate and run it
        // in the foreground so it owns the terminal. It loads its own config and
        // talks to the gateway over WebSocket.
        Commands::Tui(args) => {
            let mut fwd = common_flags(&cli.common);
            if let Some(url) = args.url {
                fwd.push("--url".to_string());
                fwd.push(url);
            }
            if let Some(pw) = args.password {
                fwd.push("--password".to_string());
                fwd.push(pw);
            }
            if args.no_dialog {
                fwd.push("--no-dialog".to_string());
            }
            launch_client("rustyclaw-tui", &fwd)?;
        }

        // ── Desktop ─────────────────────────────────────────────
        // The desktop client is a standalone binary (`rustyclaw-desktop`); run
        // it in the foreground and block until the window closes.
        Commands::Desktop(args) => {
            let mut fwd = common_flags(&cli.common);
            if let Some(url) = args.url {
                fwd.push("--url".to_string());
                fwd.push(url);
            }
            if args.no_dialog {
                fwd.push("--no-dialog".to_string());
            }
            launch_client("rustyclaw-desktop", &fwd)?;
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

        // ── Ask (headless model interaction) ────────────────────
        Commands::Ask(args) => {
            handle_ask(&config, args).await?;
        }

        // ── Status ──────────────────────────────────────────────
        Commands::Status(args) => {
            commands::status::run(&config, &args);
        }

        // ── Gateway sub-commands ────────────────────────────────
        Commands::Gateway(sub) => match sub {
            GatewayCommands::Start { log_level } => {
                let vault_password = extract_vault_password(&config);
                let ssh_listen = config
                    .ssh
                    .as_ref()
                    .map(|s| s.bind.clone())
                    .unwrap_or_else(|| "0.0.0.0:2222".to_string());
                commands::handle_start(
                    &config,
                    vault_password.as_deref(),
                    &ssh_listen,
                    log_level.as_deref(),
                )?;
            }
            GatewayCommands::Stop => {
                commands::handle_stop(&config)?;
            }
            GatewayCommands::Restart { log_level } => {
                let vault_password = extract_vault_password(&config);
                let ssh_listen = config
                    .ssh
                    .as_ref()
                    .map(|s| s.bind.clone())
                    .unwrap_or_else(|| "0.0.0.0:2222".to_string());
                commands::handle_restart(
                    &config,
                    vault_password.as_deref(),
                    &ssh_listen,
                    log_level.as_deref(),
                )?;
            }
            GatewayCommands::Status { json } => {
                commands::handle_status(&config, json);
            }
            GatewayCommands::Reload => {
                use rustyclaw_core::theme as t;

                let url = config
                    .gateway_url
                    .as_deref()
                    .unwrap_or("ws://127.0.0.1:9001");
                let sp = t::spinner("Reloading gateway configuration\u{2026}");

                match send_gateway_reload(url, config.totp_enabled).await {
                    Ok((provider, model)) => {
                        t::spinner_ok(
                            &sp,
                            &format!(
                                "Gateway reloaded: {} / {}",
                                t::info(&provider),
                                t::info(&model),
                            ),
                        );
                    }
                    Err(e) => {
                        t::spinner_fail(&sp, &format!("Reload failed: {}", e));
                    }
                }
            }
            GatewayCommands::Run(args) => {
                let bind = match args.bind {
                    GatewayBind::Loopback => "loopback",
                    GatewayBind::Lan => "lan",
                    GatewayBind::Tailnet => "tailnet",
                    GatewayBind::Auto => "auto",
                    GatewayBind::Custom => "custom",
                };
                // Verbose flag overrides log_level
                let log_level = if args.verbose {
                    Some("rustyclaw=debug,info")
                } else {
                    args.log_level.as_deref()
                };
                commands::handle_run(&config, bind, args.port, log_level)?;
            }
        },

        // ── Skills sub-commands ─────────────────────────────────
        Commands::Skills(sub) => {
            // Use consolidated skills_dirs from config
            let skills_dirs = config.skills_dirs();

            let mut sm = SkillManager::with_dirs(skills_dirs);
            sm.load_skills()?;

            match sub {
                SkillsCommands::List => {
                    use rustyclaw_core::theme as t;
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
                    println!(
                        "{}",
                        rustyclaw_core::theme::muted(&format!(
                            "Skill info for '{}' is not yet implemented.",
                            name
                        ))
                    );
                }
                SkillsCommands::Check => {
                    println!(
                        "{}",
                        rustyclaw_core::theme::muted("Skill check is not yet implemented.")
                    );
                }
            }
        }

        // ── ClawHub sub-commands ────────────────────────────────
        Commands::ClawHub(args) => commands::clawhub::run(args, &mut config)?,

        // ── Swarm sub-commands ──────────────────────────────────
        Commands::Swarm(sub) => commands::swarm::run(sub)?,
    }

    Ok(())
}

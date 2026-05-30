//! `rustyclaw` — main CLI entry point for RustyClaw.
//!
//! Provides the `rustyclaw` binary (interactive chat / one-shot commands) and
//! the `rustyclaw-gateway` daemon binary.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use rustyclaw_core::args::CommonArgs;
use rustyclaw_core::commands::{CommandAction, CommandContext, handle_command};
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{
    ClientFrame, ClientFrameType, ClientPayload, ServerFrame, ServerFrameType, ServerPayload,
    deserialize_frame, serialize_frame,
};
use rustyclaw_core::skills::SkillManager;
#[cfg(feature = "desktop")]
use rustyclaw_desktop as desktop_app;
use rustyclaw_onboard::{OnboardArgs as WizardArgs, run_onboard_wizard};
#[cfg(feature = "tui")]
use rustyclaw_tui::app::App;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

mod commands;

use commands::config::ConfigCommands;
use commands::shared::{extract_vault_password, open_secrets};

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

#[derive(Debug, Args)]
#[command(after_help = "\
EXAMPLES:
  rustyclaw ask 'What is 2+2?'
  echo 'Summarize this' | rustyclaw ask --stdin
  rustyclaw ask --model anthropic/claude-haiku 'Quick question'
  rustyclaw ask --no-tools 'Just chat, no actions'
")]
struct AskArgs {
    /// Prompt text (can also be provided via --stdin)
    #[arg(value_name = "PROMPT", trailing_var_arg = true)]
    prompt: Vec<String>,
    /// Read prompt from stdin
    #[arg(long)]
    stdin: bool,
    /// Model to use (overrides default)
    #[arg(long, short, value_name = "MODEL")]
    model: Option<String>,
    /// Disable tool use (pure chat mode)
    #[arg(long)]
    no_tools: bool,
    /// Output raw JSON response
    #[arg(long)]
    json: bool,
    /// System prompt override
    #[arg(long, value_name = "PROMPT")]
    system: Option<String>,
    /// Gateway WebSocket URL (ws://…)
    #[arg(
        long = "gateway",
        alias = "url",
        alias = "ws",
        value_name = "WS_URL",
        env = "RUSTYCLAW_GATEWAY"
    )]
    gateway: Option<String>,
    /// Maximum tokens in response
    #[arg(long, value_name = "TOKENS")]
    max_tokens: Option<u32>,
    /// Temperature (0.0-2.0)
    #[arg(long, value_name = "TEMP")]
    temperature: Option<f32>,
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

// ── Swarm subcommands ───────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
enum SwarmCommands {
    /// Create a new swarm from a template
    Create {
        /// Template name (default: 'swarm')
        #[arg(value_name = "TEMPLATE", default_value = "swarm")]
        template: String,
    },
    /// List all swarms
    List,
    /// Show detailed status of a swarm
    Status {
        /// Swarm name
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Send a message/task to a swarm agent
    Send {
        /// Swarm name
        #[arg(value_name = "SWARM")]
        swarm: String,
        /// Message to send
        #[arg(value_name = "MESSAGE", trailing_var_arg = true)]
        message: Vec<String>,
        /// Target agent ID (default: orchestrator)
        #[arg(long, short, value_name = "AGENT")]
        agent: Option<String>,
    },
    /// Stop a running swarm
    Stop {
        /// Swarm name
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// List available swarm templates
    Templates,
}

// ── ClawHub subcommands ─────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct ClawHubCommands {
    #[command(subcommand)]
    command: Option<ClawHubSub>,
}

#[derive(Debug, Subcommand)]
enum ClawHubSub {
    /// Authenticate with ClawHub
    #[command(subcommand)]
    Auth(ClawHubAuthCommands),

    /// Search skills on ClawHub
    Search {
        /// Search query
        #[arg(value_name = "QUERY", trailing_var_arg = true)]
        query: Vec<String>,
    },

    /// Show trending / popular skills
    Trending {
        /// Filter by category
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Max results to show
        #[arg(long, short = 'n', default_value_t = 15)]
        limit: usize,
    },

    /// List skill categories
    Categories,

    /// Show detailed info about a registry skill
    Info {
        /// Skill name
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Open ClawHub in your browser
    Browse,

    /// Show your ClawHub profile
    Profile,

    /// List your starred skills
    Starred,

    /// Star a skill
    Star {
        /// Skill name to star
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Unstar a skill
    Unstar {
        /// Skill name to unstar
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Install a skill from ClawHub
    Install {
        /// Skill name
        #[arg(value_name = "NAME")]
        name: String,
        /// Version to install (default: latest)
        #[arg(value_name = "VERSION")]
        version: Option<String>,
    },

    /// Publish a local skill to ClawHub
    Publish {
        /// Skill name to publish
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum ClawHubAuthCommands {
    /// Authenticate with an API token
    Login {
        /// API token from https://clawhub.ai/settings/tokens
        #[arg(value_name = "TOKEN")]
        token: String,
    },
    /// Show authentication status
    Status,
    /// Remove stored credentials
    Logout,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Entrypoint
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // For TUI mode, redirect logs to a file so they don't corrupt the terminal.
    // For all other commands, log to stderr as usual.
    #[cfg(feature = "tui")]
    let _is_tui = matches!(
        cli.command
            .as_ref()
            .unwrap_or(&Commands::Tui(TuiArgs::default())),
        Commands::Tui(_)
    );
    #[cfg(not(feature = "tui"))]
    let _is_tui = false;

    if _is_tui {
        #[cfg(feature = "tui")]
        {
            let log_path = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".rustyclaw")
                .join("tui.log");
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            rustyclaw_core::logging::init_for_tui(&log_path);
        }
    } else {
        // Initialize structured logging from environment variables.
        // Set RUSTYCLAW_LOG=debug or RUST_LOG=debug for verbose output.
        rustyclaw_core::logging::init_from_env();
    }

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
                app.set_skip_connection_dialog(_args.no_dialog);
                app.run().await?;
            }
            #[cfg(not(feature = "tui"))]
            {
                eprintln!(
                    "TUI is not available in this build. Build with --features tui to enable."
                );
                std::process::exit(1);
            }
        }

        // ── Desktop ─────────────────────────────────────────────
        Commands::Desktop(args) => {
            #[cfg(feature = "desktop")]
            {
                // Only forward an explicit URL (from --url or config). When
                // neither is set, leave it as None so the desktop client can
                // show its connection dialog with the default pre-filled.
                let gateway_url = args.url.or_else(|| config.gateway_url.clone());
                desktop_app::run(gateway_url, args.no_dialog);
            }
            #[cfg(not(feature = "desktop"))]
            {
                let _ = args;
                eprintln!(
                    "Desktop UI is not available in this build. Build with --features desktop to enable."
                );
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
            GatewayCommands::Start => {
                let vault_password = extract_vault_password(&config);
                let ssh_listen = config
                    .ssh
                    .as_ref()
                    .map(|s| s.bind.clone())
                    .unwrap_or_else(|| "0.0.0.0:2222".to_string());
                commands::handle_start(&config, vault_password.as_deref(), &ssh_listen)?;
            }
            GatewayCommands::Stop => {
                commands::handle_stop(&config)?;
            }
            GatewayCommands::Restart => {
                let vault_password = extract_vault_password(&config);
                let ssh_listen = config
                    .ssh
                    .as_ref()
                    .map(|s| s.bind.clone())
                    .unwrap_or_else(|| "0.0.0.0:2222".to_string());
                commands::handle_restart(&config, vault_password.as_deref(), &ssh_listen)?;
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
                let host = match args.bind {
                    GatewayBind::Loopback => "127.0.0.1",
                    GatewayBind::Lan => "0.0.0.0",
                    _ => "127.0.0.1",
                };
                commands::handle_run(config, host, args.port).await?;
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
        Commands::ClawHub(args) => {
            use rustyclaw_core::theme as t;

            // Use consolidated skills_dirs from config
            let skills_dirs = config.skills_dirs();

            let mut sm = SkillManager::with_dirs(skills_dirs);
            sm.load_skills()?;
            if let Some(url) = config.clawhub_url.as_deref() {
                sm.set_registry(url, config.clawhub_token.clone());
            } else if let Some(ref token) = config.clawhub_token {
                let url = sm.registry_url().to_string();
                sm.set_registry(&url, Some(token.clone()));
            }

            match args.command {
                None => {
                    // No subcommand: show overview
                    println!("{}", t::accent_bright("ClawHub — Skill Registry"));
                    println!("  Registry: {}", t::info(sm.registry_url()));
                    match sm.auth_status() {
                        Ok(status) => println!("  Auth: {}", status),
                        Err(_) => println!("  Auth: {}", t::muted("unknown")),
                    }
                    println!();
                    println!(
                        "  {} search <query>        Search for skills",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} trending [category]   Browse trending skills",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} categories            List skill categories",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} info <name>           Show skill details",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} browse                Open ClawHub in browser",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} auth login <token>    Authenticate",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} profile               Show your profile",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} starred                List starred skills",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} install <name>        Install a skill",
                        t::muted("rustyclaw clawhub")
                    );
                    println!(
                        "  {} publish <name>        Publish a skill",
                        t::muted("rustyclaw clawhub")
                    );
                }
                Some(ClawHubSub::Auth(auth_cmd)) => match auth_cmd {
                    ClawHubAuthCommands::Login { token } => match sm.auth_token(&token) {
                        Ok(resp) if resp.ok => {
                            config.clawhub_token = Some(token);
                            config.save(None)?;
                            let user = resp.username.unwrap_or_else(|| "unknown".into());
                            println!(
                                "{}",
                                t::icon_ok(&format!("Authenticated as '{}' on ClawHub.", user))
                            );
                        }
                        Ok(_) => {
                            println!("{}", t::icon_fail("Token is invalid."));
                            std::process::exit(1);
                        }
                        Err(e) => {
                            println!("{}", t::icon_fail(&format!("Auth failed: {}", e)));
                            std::process::exit(1);
                        }
                    },
                    ClawHubAuthCommands::Status => match sm.auth_status() {
                        Ok(msg) => println!("{}", msg),
                        Err(e) => println!(
                            "{}",
                            t::icon_fail(&format!("Auth status check failed: {}", e))
                        ),
                    },
                    ClawHubAuthCommands::Logout => {
                        config.clawhub_token = None;
                        config.save(None)?;
                        println!("{}", t::icon_ok("Logged out from ClawHub."));
                    }
                },
                Some(ClawHubSub::Search { query }) => {
                    let q = query.join(" ");
                    if q.is_empty() {
                        println!(
                            "{}",
                            t::icon_fail("Usage: rustyclaw clawhub search <query>")
                        );
                        std::process::exit(1);
                    }
                    match sm.search_registry(&q) {
                        Ok(results) => {
                            if results.is_empty() {
                                println!(
                                    "{}",
                                    t::muted(&format!("No skills found matching '{}'.", q))
                                );
                            } else {
                                println!("{} result(s) for '{}':", results.len(), q);
                                for r in &results {
                                    let dl = if r.downloads > 0 {
                                        format!(" (↓{})", r.downloads)
                                    } else {
                                        String::new()
                                    };
                                    println!(
                                        "  {} {} v{} by {} — {}{}",
                                        t::icon_ok(""),
                                        r.name,
                                        r.version,
                                        r.author,
                                        r.description,
                                        dl,
                                    );
                                }
                                println!();
                                println!(
                                    "Install with: {} install <name>",
                                    t::muted("rustyclaw clawhub")
                                );
                            }
                        }
                        Err(e) => {
                            println!("{}", t::icon_fail(&format!("Search failed: {}", e)));
                            std::process::exit(1);
                        }
                    }
                }
                Some(ClawHubSub::Trending { category, limit }) => {
                    match sm.trending(category.as_deref(), Some(limit)) {
                        Ok(entries) => {
                            if entries.is_empty() {
                                println!("{}", t::muted("No trending skills found."));
                            } else {
                                let header = match &category {
                                    Some(cat) => format!("Trending skills in '{}':", cat),
                                    None => "Trending skills on ClawHub:".into(),
                                };
                                println!("{}", t::accent_bright(&header));
                                for (i, e) in entries.iter().enumerate() {
                                    println!(
                                        "  {}. {} — {} (★{} ↓{})",
                                        i + 1,
                                        e.name,
                                        e.description,
                                        e.stars,
                                        e.downloads,
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            println!(
                                "{}",
                                t::icon_fail(&format!("Failed to fetch trending: {}", e))
                            );
                            std::process::exit(1);
                        }
                    }
                }
                Some(ClawHubSub::Categories) => match sm.categories() {
                    Ok(cats) => {
                        if cats.is_empty() {
                            println!("{}", t::muted("No categories found."));
                        } else {
                            println!("{}", t::accent_bright("ClawHub skill categories:"));
                            for c in &cats {
                                let count = if c.count > 0 {
                                    format!(" ({})", c.count)
                                } else {
                                    String::new()
                                };
                                println!("  • {}{} — {}", c.name, count, c.description);
                            }
                            println!();
                            println!(
                                "Browse by category: {} trending <category>",
                                t::muted("rustyclaw clawhub")
                            );
                        }
                    }
                    Err(e) => {
                        println!(
                            "{}",
                            t::icon_fail(&format!("Failed to fetch categories: {}", e))
                        );
                        std::process::exit(1);
                    }
                },
                Some(ClawHubSub::Info { name }) => match sm.registry_info(&name) {
                    Ok(detail) => {
                        println!(
                            "{}",
                            t::accent_bright(&format!("{}  v{}", detail.name, detail.version))
                        );
                        if !detail.description.is_empty() {
                            println!("  {}", detail.description);
                        }
                        if !detail.author.is_empty() {
                            println!("  Author: {}", detail.author);
                        }
                        if !detail.license.is_empty() {
                            println!("  License: {}", detail.license);
                        }
                        println!("  ★ {}  ↓ {}", detail.stars, detail.downloads);
                        if let Some(ref repo) = detail.repository {
                            println!("  Repo: {}", repo);
                        }
                        if !detail.categories.is_empty() {
                            println!("  Categories: {}", detail.categories.join(", "));
                        }
                        if !detail.required_secrets.is_empty() {
                            println!("  Requires secrets: {}", detail.required_secrets.join(", "));
                        }
                        if !detail.updated_at.is_empty() {
                            println!("  Updated: {}", detail.updated_at);
                        }
                    }
                    Err(e) => {
                        println!(
                            "{}",
                            t::icon_fail(&format!("Failed to fetch skill info: {}", e))
                        );
                        std::process::exit(1);
                    }
                },
                Some(ClawHubSub::Browse) => {
                    let url = sm.registry_url().to_string();
                    println!("Opening {} …", t::info(&url));
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open").arg(&url).spawn();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd")
                        .args(["/C", "start", &url])
                        .spawn();
                }
                Some(ClawHubSub::Profile) => match sm.profile() {
                    Ok(p) => {
                        println!(
                            "{}",
                            t::accent_bright(&format!("ClawHub profile: {}", p.username))
                        );
                        if !p.display_name.is_empty() {
                            println!("  Name: {}", p.display_name);
                        }
                        if !p.bio.is_empty() {
                            println!("  Bio: {}", p.bio);
                        }
                        println!(
                            "  Published: {}  Starred: {}",
                            p.published_count, p.starred_count
                        );
                        if !p.joined.is_empty() {
                            println!("  Joined: {}", p.joined);
                        }
                    }
                    Err(e) => {
                        println!(
                            "{}",
                            t::icon_fail(&format!("Failed to fetch profile: {}", e))
                        );
                        std::process::exit(1);
                    }
                },
                Some(ClawHubSub::Starred) => match sm.starred() {
                    Ok(entries) => {
                        if entries.is_empty() {
                            println!("{}", t::muted("No starred skills."));
                            println!(
                                "Star skills with: {} star <name>",
                                t::muted("rustyclaw clawhub")
                            );
                        } else {
                            println!("{} starred skill(s):", entries.len());
                            for e in &entries {
                                println!(
                                    "  ★ {} v{} by {} — {}",
                                    e.name, e.version, e.author, e.description,
                                );
                            }
                        }
                    }
                    Err(e) => {
                        println!(
                            "{}",
                            t::icon_fail(&format!("Failed to fetch starred: {}", e))
                        );
                        std::process::exit(1);
                    }
                },
                Some(ClawHubSub::Star { name }) => match sm.star(&name) {
                    Ok(msg) => println!("{}", t::icon_ok(&msg)),
                    Err(e) => {
                        println!("{}", t::icon_fail(&format!("Star failed: {}", e)));
                        std::process::exit(1);
                    }
                },
                Some(ClawHubSub::Unstar { name }) => match sm.unstar(&name) {
                    Ok(msg) => println!("{}", t::icon_ok(&msg)),
                    Err(e) => {
                        println!("{}", t::icon_fail(&format!("Unstar failed: {}", e)));
                        std::process::exit(1);
                    }
                },
                Some(ClawHubSub::Install { name, version }) => {
                    match sm.install_from_registry(&name, version.as_deref()) {
                        Ok(skill) => {
                            println!(
                                "{}",
                                t::icon_ok(&format!(
                                    "Skill '{}' installed from ClawHub.",
                                    skill.name
                                ))
                            );
                        }
                        Err(e) => {
                            println!("{}", t::icon_fail(&format!("Install failed: {}", e)));
                            std::process::exit(1);
                        }
                    }
                }
                Some(ClawHubSub::Publish { name }) => match sm.publish_to_registry(&name) {
                    Ok(msg) => println!("{}", t::icon_ok(&msg)),
                    Err(e) => {
                        println!("{}", t::icon_fail(&format!("Publish failed: {}", e)));
                        std::process::exit(1);
                    }
                },
            }
        }

        // ── Swarm sub-commands ──────────────────────────────────
        Commands::Swarm(sub) => {
            use rustyclaw_core::swarm::{builtin_templates, swarm_manager};
            use rustyclaw_core::theme as t;

            match sub {
                SwarmCommands::Create { template } => {
                    let templates = builtin_templates();
                    let cfg = templates
                        .into_iter()
                        .find(|t| t.name == template)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Unknown template '{}'. Use `rustyclaw swarm templates` to list.",
                                template
                            )
                        })?;
                    let name = cfg.name.clone();
                    let agent_count = cfg.agents.len();
                    let mgr = swarm_manager();
                    let mut m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
                    m.create(cfg).map_err(|e| anyhow::anyhow!(e))?;
                    m.start(&name).map_err(|e| anyhow::anyhow!(e))?;
                    println!(
                        "{}",
                        t::icon_ok(&format!(
                            "Swarm '{}' created and started with {} agents",
                            name, agent_count
                        ))
                    );

                    let inst = m.get(&name).expect("just created");
                    for agent in &inst.config.agents {
                        println!("  • {} ({})", t::accent_bright(&agent.name), agent.role);
                    }
                }
                SwarmCommands::List => {
                    let mgr = swarm_manager();
                    let m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
                    let swarms = m.list();
                    if swarms.is_empty() {
                        println!(
                            "{}",
                            t::muted(
                                "No swarms defined. Use `rustyclaw swarm create` to create one."
                            )
                        );
                    } else {
                        for inst in swarms {
                            let status = match inst.status {
                                rustyclaw_core::swarm::SwarmStatus::Running => {
                                    t::icon_ok("Running")
                                }
                                rustyclaw_core::swarm::SwarmStatus::Idle => t::info("Idle"),
                                rustyclaw_core::swarm::SwarmStatus::Paused => t::info("Paused"),
                                rustyclaw_core::swarm::SwarmStatus::Stopped => t::muted("Stopped"),
                                rustyclaw_core::swarm::SwarmStatus::Error => t::icon_fail("Error"),
                            };
                            println!(
                                "  {} {} — {} agents, {} tasks",
                                status,
                                t::accent_bright(&inst.config.name),
                                inst.config.agents.len(),
                                inst.tasks_routed,
                            );
                        }
                    }
                }
                SwarmCommands::Status { name } => {
                    let mgr = swarm_manager();
                    let m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
                    let inst = m
                        .get(&name)
                        .ok_or_else(|| anyhow::anyhow!("Swarm '{}' not found", name))?;
                    println!(
                        "{} — {} ({}s uptime, {} tasks)",
                        t::accent_bright(&inst.config.name),
                        inst.status,
                        inst.runtime_secs(),
                        inst.tasks_routed,
                    );
                    println!();
                    println!("{}", t::info("Agents:"));
                    for agent in &inst.config.agents {
                        let session = inst
                            .agent_sessions
                            .get(&agent.id)
                            .map(|s| format!(" [{}]", s))
                            .unwrap_or_default();
                        println!(
                            "  • {} ({}){}",
                            t::accent_bright(&agent.name),
                            agent.role,
                            session
                        );
                    }
                }
                SwarmCommands::Send {
                    swarm,
                    message,
                    agent,
                } => {
                    let msg = message.join(" ");
                    if msg.trim().is_empty() {
                        anyhow::bail!("Message cannot be empty");
                    }
                    let mgr = swarm_manager();
                    let target = agent.as_deref().unwrap_or("orchestrator");

                    // Phase 1: validate swarm/agent and extract info.
                    let (agent_name, agent_instructions, existing_session) = {
                        let mut m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
                        let inst = m
                            .get_mut(&swarm)
                            .ok_or_else(|| anyhow::anyhow!("Swarm '{}' not found", swarm))?;
                        if inst.status != rustyclaw_core::swarm::SwarmStatus::Running {
                            anyhow::bail!("Swarm '{}' is not running", swarm);
                        }
                        let a = inst
                            .config
                            .agents
                            .iter()
                            .find(|a| a.id == target)
                            .ok_or_else(|| {
                                let ids: Vec<&str> =
                                    inst.config.agents.iter().map(|a| a.id.as_str()).collect();
                                anyhow::anyhow!(
                                    "Agent '{}' not found in swarm '{}'. Available: {}",
                                    target,
                                    swarm,
                                    ids.join(", ")
                                )
                            })?;
                        let name = a.name.clone();
                        let instructions = a.instructions.clone();
                        let existing = inst.agent_sessions.get(target).cloned();
                        inst.record_task();
                        (name, instructions, existing)
                    };

                    // Phase 2: route via session manager (no swarm lock held).
                    let session_mgr = rustyclaw_core::sessions::session_manager();
                    let mut sess_mgr = session_mgr
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Session manager lock error"))?;

                    let session_key = if let Some(existing) = existing_session {
                        sess_mgr
                            .send_message(&existing, &msg)
                            .map_err(|e| anyhow::anyhow!(e))?;
                        existing
                    } else {
                        let label = format!("swarm:{}:{}", swarm, target);
                        let task = format!(
                            "[Swarm: {} | Agent: {}]\n\n{}\n\nSystem Instructions:\n{}",
                            swarm, agent_name, msg, agent_instructions
                        );
                        let key = sess_mgr.spawn_subagent(target, &task, Some(label), None);
                        drop(sess_mgr);

                        // Phase 3: store session key back.
                        let mut m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
                        if let Some(inst) = m.get_mut(&swarm) {
                            inst.agent_sessions.insert(target.to_string(), key.clone());
                        }
                        key
                    };

                    println!(
                        "{}",
                        t::icon_ok(&format!(
                            "Task routed to {} ({}) in swarm '{}' — session: {}",
                            agent_name, target, swarm, session_key
                        ))
                    );
                    println!("  Message: {}", t::muted(&msg));
                }
                SwarmCommands::Stop { name } => {
                    let mgr = swarm_manager();
                    let mut m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
                    m.stop(&name).map_err(|e| anyhow::anyhow!(e))?;
                    println!("{}", t::icon_ok(&format!("Swarm '{}' stopped", name)));
                }
                SwarmCommands::Templates => {
                    let templates = builtin_templates();
                    println!("{}", t::accent_bright("Available swarm templates:"));
                    println!();
                    for t_cfg in &templates {
                        println!(
                            "  {} — {} agents",
                            t::accent_bright(&t_cfg.name),
                            t_cfg.agents.len()
                        );
                        println!("    {}", t::muted(&t_cfg.description));
                        for agent in &t_cfg.agents {
                            println!("      • {} ({})", agent.name, agent.role);
                        }
                        println!();
                    }
                }
            }
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════════

// ── Local (offline) command execution ───────────────────────────────────────

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
        loop {
            let msg = match reader.next().await {
                Some(m) => m,
                None => anyhow::bail!("Connection closed"),
            };
            let msg = msg.context("Gateway read error")?;
            // Handle both binary and text frames (for backwards compat during transition)
            match msg {
                Message::Binary(data) => {
                    if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                        match frame.frame_type {
                            ServerFrameType::AuthChallenge => {
                                if let ServerPayload::AuthChallenge { method: _ } = frame.payload {
                                    let code = rpassword::prompt_password(format!(
                                        "{} 2FA code: ",
                                        rustyclaw_core::theme::info("🔑")
                                    ))
                                    .unwrap_or_default();
                                    let auth_frame = ClientFrame {
                                        frame_type: ClientFrameType::AuthResponse,
                                        payload: ClientPayload::AuthResponse {
                                            code: code.trim().to_string(),
                                        },
                                    };
                                    let bytes = serialize_frame(&auth_frame)
                                        .map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
                                    writer.send(Message::Binary(bytes.into())).await?;
                                }
                            }
                            ServerFrameType::AuthResult => {
                                if let ServerPayload::AuthResult {
                                    ok,
                                    message,
                                    retry: _,
                                } = frame.payload
                                {
                                    if !ok {
                                        let msg = message.as_deref().unwrap_or("Auth failed");
                                        anyhow::bail!("{}", msg);
                                    }
                                    break; // Auth succeeded
                                }
                            }
                            ServerFrameType::Hello => {
                                break; // No auth needed
                            }
                            _ => {}
                        }
                    }
                }
                Message::Text(text) => {
                    // Also handle text frames for backwards compat
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                        let frame_type = val.get("type").and_then(|t| t.as_str());
                        if frame_type == Some("auth_challenge") {
                            let code = rpassword::prompt_password(format!(
                                "{} 2FA code: ",
                                rustyclaw_core::theme::info("🔑")
                            ))
                            .unwrap_or_default();
                            let auth_frame = ClientFrame {
                                frame_type: ClientFrameType::AuthResponse,
                                payload: ClientPayload::AuthResponse {
                                    code: code.trim().to_string(),
                                },
                            };
                            let bytes = serialize_frame(&auth_frame)
                                .map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
                            writer.send(Message::Binary(bytes.into())).await?;
                            continue;
                        }
                        if frame_type == Some("auth_result") {
                            let ok = val.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                            if !ok {
                                let msg = val
                                    .get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Auth failed");
                                anyhow::bail!("{}", msg);
                            }
                            break;
                        }
                        if frame_type == Some("hello") {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Wait for hello frame
    let mut _result_provider = String::new();
    let mut _result_model = String::new();
    loop {
        match reader.next().await {
            Some(Ok(Message::Binary(data))) => {
                if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                    match frame.frame_type {
                        ServerFrameType::Hello => {
                            if let ServerPayload::Hello {
                                provider, model, ..
                            } = frame.payload
                            {
                                _result_provider = provider.unwrap_or_default();
                                _result_model = model.unwrap_or_default();
                                break;
                            }
                        }
                        ServerFrameType::AuthChallenge if totp_enabled => {
                            // Prompt the user for their TOTP 2FA code and reply
                            // with an AuthResponse frame.
                            let code = rpassword::prompt_password(format!(
                                "{} 2FA code: ",
                                rustyclaw_core::theme::info("🔑")
                            ))
                            .unwrap_or_default();
                            let auth_frame = ClientFrame {
                                frame_type: ClientFrameType::AuthResponse,
                                payload: ClientPayload::AuthResponse {
                                    code: code.trim().to_string(),
                                },
                            };
                            let bytes = serialize_frame(&auth_frame)
                                .map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
                            writer.send(Message::Binary(bytes.into())).await?;
                        }
                        _ => {}
                    }
                }
            }
            Some(Ok(Message::Text(text))) => {
                // Also handle text frames for backwards compat
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                    let frame_type = val.get("type").and_then(|t| t.as_str());
                    if frame_type == Some("hello") || frame_type == Some("auth_challenge") {
                        if frame_type == Some("auth_challenge") && !totp_enabled {
                            let code = rpassword::prompt_password(format!(
                                "{} 2FA code: ",
                                rustyclaw_core::theme::info("🔑")
                            ))
                            .unwrap_or_default();
                            let auth_frame = ClientFrame {
                                frame_type: ClientFrameType::AuthResponse,
                                payload: ClientPayload::AuthResponse {
                                    code: code.trim().to_string(),
                                },
                            };
                            let bytes = serialize_frame(&auth_frame)
                                .map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
                            writer.send(Message::Binary(bytes.into())).await?;
                            continue;
                        }
                        let provider = val
                            .get("provider")
                            .and_then(|p| p.as_str())
                            .unwrap_or("")
                            .to_string();
                        let model = val
                            .get("model")
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .to_string();
                        _result_provider = provider;
                        _result_model = model;
                        break;
                    }
                }
            }
            Some(Ok(Message::Close(_))) => {
                anyhow::bail!("Gateway closed connection");
            }
            None => {
                anyhow::bail!("Gateway disconnected");
            }
            _ => {}
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

    // Send reload command using binary frame
    let reload_frame = ClientFrame {
        frame_type: ClientFrameType::Reload,
        payload: ClientPayload::Reload,
    };
    let bytes =
        serialize_frame(&reload_frame).map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
    writer
        .send(Message::Binary(bytes.into()))
        .await
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
                    Some(Ok(Message::Binary(data))) => {
                        if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                            match frame.frame_type {
                                ServerFrameType::ReloadResult => {
                                    if let ServerPayload::ReloadResult { ok, provider, model, message } = frame.payload {
                                        if ok {
                                            // Close cleanly
                                            let _ = writer.send(Message::Close(None)).await;
                                            return Ok((provider, model));
                                        } else {
                                            let msg = message.as_deref().unwrap_or("Unknown error");
                                            anyhow::bail!("{}", msg);
                                        }
                                    }
                                }
                                _ => continue,
                            }
                        }
                    }
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

/// Handle the `ask` command — headless model interaction.
async fn handle_ask(config: &Config, args: AskArgs) -> Result<()> {
    use rustyclaw_core::gateway::protocol::types::ChatMessage;
    use rustyclaw_core::gateway::protocol::{
        ClientFrame, ClientFrameType, ClientPayload, ServerFrame, ServerFrameType, ServerPayload,
        deserialize_frame, serialize_frame,
    };
    use std::io::{self, Read};

    // Gather the prompt
    let prompt = if args.stdin {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        if !args.prompt.is_empty() {
            // Prepend CLI args to stdin content
            format!("{}\n\n{}", args.prompt.join(" "), buf)
        } else {
            buf
        }
    } else {
        args.prompt.join(" ")
    };

    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        anyhow::bail!("No prompt provided. Use `rustyclaw ask 'your prompt'` or `--stdin`.");
    }

    // Determine gateway URL
    let gateway_url = args
        .gateway
        .or_else(|| config.gateway_url.clone())
        .unwrap_or_else(|| "ws://127.0.0.1:9001".to_string());

    // Connect to gateway
    let url = Url::parse(&gateway_url).context("Invalid gateway URL")?;
    let (ws_stream, _) = tokio_tungstenite::connect_async(url.to_string())
        .await
        .context("Failed to connect to gateway. Is it running? Try `rustyclaw gateway start`")?;
    let (mut writer, mut reader) = ws_stream.split();

    // Handle auth if needed (simplified — skip TOTP for now)
    // TODO: Add TOTP support for headless mode

    // Build the chat message
    let message = ChatMessage::text("user", &prompt);

    // Send as ClientFrame
    let frame = ClientFrame {
        frame_type: ClientFrameType::Chat,
        payload: ClientPayload::Chat {
            messages: vec![message],
        },
    };
    let bytes = serialize_frame(&frame).map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
    writer.send(Message::Binary(bytes.into())).await?;

    // Collect response
    let mut response_text = String::new();
    let mut tool_outputs: Vec<String> = Vec::new();

    while let Some(message) = reader.next().await {
        let message = message.context("Gateway read error")?;

        match message {
            Message::Binary(data) => {
                if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                    match frame.frame_type {
                        ServerFrameType::Chunk => {
                            if let ServerPayload::Chunk { delta } = frame.payload {
                                if !args.json {
                                    // Stream text to stdout
                                    print!("{}", delta);
                                    io::Write::flush(&mut io::stdout())?;
                                }
                                response_text.push_str(&delta);
                            }
                        }
                        ServerFrameType::ResponseDone => {
                            // Model finished
                            if !args.json {
                                println!(); // Final newline
                            }
                            break;
                        }
                        ServerFrameType::ToolCall => {
                            if let ServerPayload::ToolCall { id: _, name, .. } = frame.payload {
                                if !args.json {
                                    eprintln!("  → {}", name);
                                }
                            }
                        }
                        ServerFrameType::ToolResult => {
                            if let ServerPayload::ToolResult {
                                id: _,
                                name,
                                result,
                                ..
                            } = frame.payload
                            {
                                tool_outputs.push(format!("{}: {}", name, result));
                            }
                        }
                        ServerFrameType::Error => {
                            if let ServerPayload::Error { message, .. } = frame.payload {
                                anyhow::bail!("Gateway error: {}", message);
                            }
                        }
                        ServerFrameType::Info => {
                            if let ServerPayload::Info { message } = frame.payload {
                                if !args.json {
                                    eprintln!("  ℹ {}", message);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::Text(text) => {
                // Legacy text frame — just print it
                if !args.json {
                    print!("{}", text);
                }
                response_text.push_str(&text);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // JSON output if requested
    if args.json {
        let output = serde_json::json!({
            "response": response_text,
            "tool_calls": tool_outputs,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

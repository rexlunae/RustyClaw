use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use rustyclaw::app::App;
use rustyclaw::args::CommonArgs;
use rustyclaw::commands::{handle_command, CommandAction, CommandContext};
use rustyclaw::config::Config;
use rustyclaw::onboard::run_onboard_wizard;
use rustyclaw::secrets::SecretsManager;
use rustyclaw::skills::SkillManager;
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
                run_onboard_wizard(&mut config, &mut secrets, false)?;
            } else {
                // Minimal setup: ensure directory skeleton + default config.
                if let Some(ws) = args.workspace {
                    config.workspace_dir = Some(ws.into());
                }
                config.ensure_dirs()?;
                config.save(None)?;
                println!("✓ Initialised config + workspace at {}", config.settings_dir.display());
            }
        }

        // ── Onboard ─────────────────────────────────────────────
        Commands::Onboard(args) => {
            let mut secrets = open_secrets(&config)?;
            run_onboard_wizard(&mut config, &mut secrets, args.reset)?;
        }

        // ── Configure ───────────────────────────────────────────
        Commands::Configure => {
            let mut secrets = open_secrets(&config)?;
            run_onboard_wizard(&mut config, &mut secrets, false)?;
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
                    println!("✓ Set {} = {}", path, value);
                }
                ConfigCommands::Unset { path } => {
                    config_unset(&mut config, &path)?;
                    config.save(None)?;
                    println!("✓ Unset {}", path);
                }
            }
        }

        // ── Doctor ──────────────────────────────────────────────
        Commands::Doctor(_args) => {
            // Basic health checks — will be expanded.
            println!("Running health checks…\n");

            let checks = vec![
                ("Config file", config.settings_dir.join("config.toml").exists()),
                ("Workspace dir", config.workspace_dir().exists()),
                ("Credentials dir", config.credentials_dir().exists()),
                ("SOUL.md", config.soul_path().exists()),
                ("Skills dir", config.skills_dir().exists()),
            ];

            let mut ok = true;
            for (label, passed) in &checks {
                let icon = if *passed { "✓" } else { "✗" };
                println!("  {} {}", icon, label);
                if !passed { ok = false; }
            }
            println!();
            if ok {
                println!("All checks passed.");
            } else {
                println!("Some checks failed.  Run `rustyclaw setup` or `rustyclaw onboard` to fix.");
            }
        }

        // ── TUI ─────────────────────────────────────────────────
        Commands::Tui(args) => {
            // Apply TUI-specific overrides.
            if let Some(url) = &args.url {
                config.gateway_url = Some(url.clone());
            }
            let mut app = if config.secrets_password_protected {
                let pw = if let Some(pw) = args.password {
                    pw
                } else {
                    prompt_password("Enter secrets vault password: ")?
                };
                App::with_password(config, pw)?
            } else {
                App::new(config)?
            };
            app.run().await?;
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
                run_local_command(&config, &input)?;
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
                    println!("Starting gateway…  (use `rustyclaw-gateway` for foreground mode)");
                    // TODO: launch daemon
                    println!("Gateway daemon start is not yet implemented. Use `rustyclaw gateway run` or `rustyclaw-gateway` instead.");
                }
                GatewayCommands::Stop => {
                    println!("Stopping gateway…");
                    // TODO: stop daemon
                    println!("Gateway daemon stop is not yet implemented.");
                }
                GatewayCommands::Restart => {
                    println!("Restarting gateway…");
                    // TODO: restart daemon
                    println!("Gateway daemon restart is not yet implemented.");
                }
                GatewayCommands::Status { json: _ } => {
                    let url = config.gateway_url.as_deref().unwrap_or("ws://127.0.0.1:9001");
                    println!("Gateway URL: {}", url);
                    println!("(detailed status probe not yet implemented)");
                }
                GatewayCommands::Run(args) => {
                    use rustyclaw::gateway::{run_gateway, GatewayOptions};
                    use tokio_util::sync::CancellationToken;

                    let host = match args.bind {
                        GatewayBind::Loopback => "127.0.0.1",
                        GatewayBind::Lan => "0.0.0.0",
                        _ => "127.0.0.1",
                    };
                    let listen = format!("{}:{}", host, args.port);
                    println!("RustyClaw gateway listening on ws://{}", listen);

                    let cancel = CancellationToken::new();
                    run_gateway(config, GatewayOptions { listen }, cancel).await?;
                }
            }
        }

        // ── Skills sub-commands ─────────────────────────────────
        Commands::Skills(sub) => {
            let skills_dir = config.skills_dir();
            let mut sm = SkillManager::new(skills_dir);
            sm.load_skills()?;

            match sub {
                SkillsCommands::List => {
                    let skills = sm.get_skills();
                    if skills.is_empty() {
                        println!("No skills installed.");
                    } else {
                        for s in skills {
                            let status = if s.enabled { "✓" } else { "✗" };
                            println!("  {} {}", status, s.name);
                        }
                    }
                }
                SkillsCommands::Info { name } => {
                    println!("Skill info for '{}' is not yet implemented.", name);
                }
                SkillsCommands::Check => {
                    println!("Skill check is not yet implemented.");
                }
            }
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Open the secrets vault, prompting for a password if required.
fn open_secrets(config: &Config) -> Result<SecretsManager> {
    if config.secrets_password_protected {
        let pw = prompt_password("Enter secrets vault password: ")?;
        Ok(SecretsManager::with_password(config.credentials_dir(), pw))
    } else {
        Ok(SecretsManager::new(config.credentials_dir()))
    }
}

fn prompt_password(prompt: &str) -> Result<String> {
    use std::io::{self, Write};
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
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
        println!("RustyClaw status\n");
        println!("  Settings dir : {}", config.settings_dir.display());
        println!("  Workspace    : {}", config.workspace_dir().display());
        if let Some(m) = &config.model {
            println!("  Provider     : {}", m.provider);
            if let Some(model) = &m.model {
                println!("  Model        : {}", model);
            }
        } else {
            println!("  Provider     : (not configured — run `rustyclaw onboard`)");
        }
        if let Some(gw) = &config.gateway_url {
            println!("  Gateway URL  : {}", gw);
        }
        if args.verbose || args.all {
            println!("  SOUL.md      : {}", config.soul_path().display());
            println!("  Skills dir   : {}", config.skills_dir().display());
            println!("  Credentials  : {}", config.credentials_dir().display());
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

fn run_local_command(config: &Config, input: &str) -> Result<()> {
    let mut secrets_manager = if config.secrets_password_protected {
        let pw = prompt_password("Enter secrets vault password: ")?;
        SecretsManager::with_password(config.credentials_dir(), pw)
    } else {
        SecretsManager::new(config.credentials_dir())
    };
    let skills_dir = config.skills_dir();
    let mut skill_manager = SkillManager::new(skills_dir);
    skill_manager.load_skills()?;

    let mut context = CommandContext {
        secrets_manager: &mut secrets_manager,
        skill_manager: &mut skill_manager,
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

async fn send_command_via_gateway(gateway_url: &str, command: &str) -> Result<String> {
    let url = Url::parse(gateway_url).context("Invalid gateway URL")?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .context("Failed to connect to gateway")?;
    let (mut writer, mut reader) = ws_stream.split();
    writer
        .send(Message::Text(command.to_string()))
        .await
        .context("Failed to send command")?;

    while let Some(message) = reader.next().await {
        let message = message.context("Gateway read error")?;
        if let Message::Text(text) = message {
            return Ok(text);
        }
    }

    anyhow::bail!("Gateway closed without responding")
}

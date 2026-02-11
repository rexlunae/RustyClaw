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
                let mut secrets = open_secrets(&config)?;
                run_onboard_wizard(&mut config, &mut secrets, false)?;
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
                    use rustyclaw::daemon;
                    use rustyclaw::theme as t;

                    let sp = t::spinner("Starting gateway…");

                    let (port, bind) = parse_gateway_defaults(&config);

                    match daemon::start(
                        &config.settings_dir,
                        port,
                        bind,
                        &[],
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
                GatewayCommands::Run(args) => {
                    use rustyclaw::gateway::{run_gateway, GatewayOptions};
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

/// Open the secrets vault, prompting for a password and TOTP if required.
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

fn run_local_command(config: &Config, input: &str) -> Result<()> {
    let mut secrets_manager = open_secrets(config)?;
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

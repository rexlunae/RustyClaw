use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use rustyclaw::args::CommonArgs;
use rustyclaw::config::Config;
use rustyclaw::gateway::{run_gateway, GatewayOptions};
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
                println!("Gateway URL: {}", url);
                println!("(detailed status probe not yet implemented)");
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

    println!("RustyClaw gateway listening on ws://{}", listen);

    let cancel = CancellationToken::new();
    run_gateway(config, GatewayOptions { listen }, cancel).await
}

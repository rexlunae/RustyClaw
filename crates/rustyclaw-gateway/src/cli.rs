//! Command-line interface for the `rustyclaw-gateway` binary: argument
//! definitions (clap) and the `pair` subcommand handler.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use rustyclaw_core::args::CommonArgs;
use rustyclaw_core::theme as t;

// ── Gateway bind modes ──────────────────────────────────────────────────────

#[derive(Debug, Clone, ValueEnum)]
pub(crate) enum GatewayBind {
    Loopback,
    Lan,
    Tailnet,
    Auto,
    Custom,
}

// ── Gateway auth modes ──────────────────────────────────────────────────────

#[derive(Debug, Clone, ValueEnum)]
pub(crate) enum GatewayAuth {
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
pub(crate) struct GatewayCli {
    #[command(flatten)]
    pub(crate) common: CommonArgs,
    #[command(subcommand)]
    pub(crate) command: Option<GatewayCommands>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum GatewayCommands {
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
pub(crate) enum PairCommands {
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
pub(crate) struct RunArgs {
    /// Gateway port
    #[arg(long, value_name = "PORT", default_value_t = 9001)]
    pub(crate) port: u16,
    /// Bind mode (loopback, lan, tailnet, auto, custom)
    #[arg(long, value_enum, default_value_t = GatewayBind::Loopback)]
    pub(crate) bind: GatewayBind,
    /// Auth token
    #[arg(long, value_name = "TOKEN")]
    pub(crate) token: Option<String>,
    /// Auth mode
    #[arg(long, value_enum)]
    pub(crate) auth: Option<GatewayAuth>,
    /// Auth password
    #[arg(long, value_name = "PASSWORD")]
    pub(crate) password: Option<String>,
    /// Overwrite existing configuration
    #[arg(long)]
    pub(crate) force: bool,
    /// Verbose logging
    #[arg(long, short)]
    pub(crate) verbose: bool,
    /// WebSocket listen URL (ws://host:port) — overrides --bind/--port
    #[arg(long = "listen", alias = "url", alias = "ws", value_name = "WS_URL")]
    pub(crate) listen: Option<String>,
    /// Path to TLS certificate file (PEM) for WSS connections
    #[arg(long, value_name = "PATH")]
    pub(crate) tls_cert: Option<std::path::PathBuf>,
    /// Path to TLS private key file (PEM) for WSS connections
    #[arg(long, value_name = "PATH")]
    pub(crate) tls_key: Option<std::path::PathBuf>,
    /// SSH server listen address (e.g., 0.0.0.0:2222)
    #[arg(long, value_name = "ADDR")]
    pub(crate) ssh_listen: Option<String>,
    /// Run as SSH subsystem (stdio mode for OpenSSH integration)
    #[arg(long)]
    pub(crate) ssh_stdio: bool,
    /// Path to SSH host key (default: ~/.rustyclaw/ssh_host_key)
    #[arg(long, value_name = "PATH")]
    pub(crate) ssh_host_key: Option<std::path::PathBuf>,
    /// Path to authorized_clients file (default: ~/.rustyclaw/authorized_clients)
    #[arg(long, value_name = "PATH")]
    pub(crate) ssh_authorized_clients: Option<std::path::PathBuf>,
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

/// Handle pairing subcommands.
pub(crate) async fn handle_pair_command(cmd: PairCommands) -> Result<()> {
    use rustyclaw_core::pairing::{
        add_authorized_client, default_authorized_clients_path, load_authorized_clients,
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
                println!(
                    "  {} pair add <PUBLIC_KEY> --name <NAME>",
                    t::info("rustyclaw-gateway")
                );
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
            println!("{} {}", t::muted("File:"), auth_path.display());
        }

        PairCommands::Add { key, name } => {
            match add_authorized_client(&auth_path, &key, name.as_deref()) {
                Ok(client) => {
                    println!(
                        "{} Added client: {}",
                        t::icon_ok(""),
                        t::info(client.comment.as_deref().unwrap_or("(unnamed)"))
                    );
                    println!("  {} {}", t::muted("Fingerprint:"), client.fingerprint);
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

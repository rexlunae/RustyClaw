//! `rustyclaw-tui` — terminal UI client for RustyClaw, built on
//! [`iocraft`](https://crates.io/crates/iocraft).
//!
//! Standalone binary: connects to a local or remote `rustyclaw-gateway` over
//! WebSocket and renders the conversation in the terminal. Launched directly or
//! spawned by the `rustyclaw` CLI's `tui` subcommand.

mod action;
mod app;
mod components;
mod connection_dialog;
mod gateway_client;
mod markdown;
mod pairing;
mod theme;
mod types;

use std::path::PathBuf;

use clap::Parser;
use rustyclaw_view::anyhow::Result;
use rustyclaw_view::{dirs, tokio};

use rustyclaw_core::args::CommonArgs;
use rustyclaw_core::config::Config;

use app::App;

#[derive(Debug, Parser)]
#[command(
    name = "rustyclaw-tui",
    version,
    about = "RustyClaw terminal UI client"
)]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,
    /// Gateway WebSocket URL (overrides config)
    #[arg(long = "url", value_name = "URL")]
    url: Option<String>,
    /// Vault password (forwarded to the gateway after connect if the vault is locked)
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,
    /// Skip the interactive connection dialog and use the saved/default gateway URL.
    #[arg(long = "no-dialog", alias = "auto-connect")]
    no_dialog: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Redirect logs to a file so they don't corrupt the terminal UI.
    let log_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".rustyclaw")
        .join("tui.log");
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rustyclaw_core::logging::init_for_tui(&log_path);

    // Initialise colour output (respects --no-color / NO_COLOR).
    rustyclaw_core::theme::init_color(cli.common.no_color);

    let config_path = cli.common.config_path();
    let mut config = Config::load(config_path)?;
    cli.common.apply_overrides(&mut config);

    if let Some(url) = &cli.url {
        config.gateway_url = Some(url.clone());
    }

    // The gateway owns the secrets vault. The TUI fetches secrets via gateway
    // messages; a --password is forwarded to the gateway after connect if the
    // vault is locked.
    let mut app = App::new(config)?;
    if let Some(pw) = cli.password {
        app.set_deferred_vault_password(pw);
    }
    app.set_skip_connection_dialog(cli.no_dialog);
    app.run().await?;

    Ok(())
}

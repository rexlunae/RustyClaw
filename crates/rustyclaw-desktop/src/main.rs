//! `rustyclaw-desktop` — desktop GUI client for RustyClaw (Dioxus).
//!
//! Standalone binary: connects to a `rustyclaw-gateway` over SSH and renders
//! the conversation in a native window. Launched directly or spawned by the
//! `rustyclaw` CLI's `desktop` subcommand.

use std::sync::OnceLock;

use clap::Parser;
use dioxus::desktop::tao::window::Icon;
use dioxus::desktop::{Config as DesktopConfig, LogicalSize, WindowBuilder};
use rustyclaw_view::anyhow::Result;
use rustyclaw_view::url::Url;
use rustyclaw_view::tracing;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use rustyclaw_core::args::CommonArgs;
use rustyclaw_core::config::Config;

mod app;
mod app_support;
mod chat_transcript;
mod components;
mod menu;
mod state;

// Shared client-preference helpers from `rustyclaw-core`, surfaced at the crate
// root so the desktop modules can reach them as `crate::…` (kept in lock-step
// with the TUI client).
use rustyclaw_core::client_prefs::{
    DEFAULT_GATEWAY_URL, load_auto_connect_gateway_urls, load_saved_gateway_url, save_gateway_url,
    should_bypass_connection_dialog,
};

static GATEWAY_URL: OnceLock<Option<String>> = OnceLock::new();
static SKIP_DIALOG: OnceLock<bool> = OnceLock::new();
static FORCE_DIALOG: OnceLock<bool> = OnceLock::new();

#[derive(Debug, Parser)]
#[command(
    name = "rustyclaw-desktop",
    version,
    about = "RustyClaw desktop GUI client"
)]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,
    /// Gateway URL (overrides config)
    #[arg(long = "url", value_name = "URL")]
    url: Option<String>,
    /// Skip the connection dialog on startup and connect to the saved or
    /// default URL automatically.
    #[arg(long = "no-dialog", alias = "auto-connect")]
    no_dialog: bool,
    /// Always show the connection dialog on startup, even when auto-connect
    /// is configured. Used by the "New Connection Window" menu entry.
    #[arg(long = "pick-connection", conflicts_with = "no_dialog")]
    pick_connection: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut config = Config::load(cli.common.config_path())?;
    cli.common.apply_overrides(&mut config);

    // Only forward an explicit URL (from --url or config). When neither is set,
    // leave it None so the desktop client shows its connection dialog with the
    // default pre-filled. --pick-connection forces the dialog regardless.
    let gateway_url = if cli.pick_connection {
        None
    } else {
        cli.url.or_else(|| config.gateway_url.clone())
    };

    run(gateway_url, cli.no_dialog, cli.pick_connection);
    Ok(())
}

fn run(gateway_url: Option<String>, no_dialog: bool, pick_connection: bool) {
    let normalized_gateway_url = normalize_gateway_url(gateway_url);
    let _ = GATEWAY_URL.set(normalized_gateway_url);
    let _ = SKIP_DIALOG.set(no_dialog);
    let _ = FORCE_DIALOG.set(pick_connection);

    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .try_init();

    tracing::info!("Starting RustyClaw Desktop");

    let window = WindowBuilder::new()
        .with_title("RustyClaw")
        .with_inner_size(LogicalSize::new(1180.0, 760.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .with_window_icon(app_icon());

    // Match the dark-theme background so there's no white flash on startup.
    let cfg = DesktopConfig::new()
        .with_window(window)
        .with_background_color((15, 17, 21, 0xFF))
        .with_menu(menu::build_app_menu());

    dioxus::LaunchBuilder::desktop()
        .with_cfg(cfg)
        .launch(app::App);
}

/// 256×256 application icon, rendered from the project logo at build time
/// (`logo.svg` → `$OUT_DIR/icon-256.png`; see `build.rs` for the full set).
const ICON_PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon-256.png"));

/// Decode the embedded icon for the window/taskbar. Used on Windows and
/// Linux; macOS takes the Dock icon from the app bundle's `icon.icns`.
fn app_icon() -> Option<Icon> {
    let img = image::load_from_memory(ICON_PNG).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Icon::from_rgba(img.into_raw(), width, height).ok()
}

pub(crate) fn configured_gateway_url() -> Option<String> {
    GATEWAY_URL.get().cloned().flatten()
}

pub(crate) fn skip_connection_dialog() -> bool {
    SKIP_DIALOG.get().copied().unwrap_or(false)
}

pub(crate) fn force_connection_dialog() -> bool {
    FORCE_DIALOG.get().copied().unwrap_or(false)
}

/// Spawn a second desktop window as a separate process, opening on the
/// connection dialog so a new gateway connection can be established
/// without disturbing this one.
pub(crate) fn spawn_connection_window() {
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe)
                .arg("--pick-connection")
                .spawn()
            {
                tracing::error!("failed to spawn new connection window: {e}");
            }
        }
        Err(e) => tracing::error!("failed to resolve current executable: {e}"),
    }
}

fn normalize_gateway_url(gateway_url: Option<String>) -> Option<String> {
    let url = gateway_url?;

    let parsed = match Url::parse(&url) {
        Ok(parsed) => parsed,
        Err(_) => return Some(url),
    };

    if !matches!(parsed.scheme(), "ws" | "wss") {
        return Some(url);
    }

    let host = parsed.host_str().unwrap_or("127.0.0.1");
    let port = match parsed.port() {
        Some(9001) | None => 2222,
        Some(port) => port,
    };

    let normalized = if parsed.username().is_empty() {
        format!("ssh://{}:{}", host, port)
    } else {
        format!("ssh://{}@{}:{}", parsed.username(), host, port)
    };

    tracing::warn!(
        old_url = %url,
        new_url = %normalized,
        "Converting legacy WebSocket desktop gateway URL to SSH"
    );

    Some(normalized)
}

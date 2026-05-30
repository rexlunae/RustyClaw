//! RustyClaw Desktop Client library entrypoint.

use std::sync::OnceLock;

use dioxus::desktop::{Config as DesktopConfig, LogicalSize, WindowBuilder};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use url::Url;

pub mod app;
mod components;
mod markdown;
pub(crate) mod menu;
mod state;

static GATEWAY_URL: OnceLock<Option<String>> = OnceLock::new();
static SKIP_DIALOG: OnceLock<bool> = OnceLock::new();

pub fn run(gateway_url: Option<String>, no_dialog: bool) {
    let normalized_gateway_url = normalize_gateway_url(gateway_url);
    let _ = GATEWAY_URL.set(normalized_gateway_url);
    let _ = SKIP_DIALOG.set(no_dialog);

    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .try_init();

    tracing::info!("Starting RustyClaw Desktop");

    let window = WindowBuilder::new()
        .with_title("RustyClaw")
        .with_inner_size(LogicalSize::new(1180.0, 760.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0));

    // Match the dark-theme background so there's no white flash on startup.
    let cfg = DesktopConfig::new()
        .with_window(window)
        .with_background_color((15, 17, 21, 0xFF))
        .with_menu(menu::build_app_menu());

    dioxus::LaunchBuilder::desktop()
        .with_cfg(cfg)
        .launch(app::App);
}

pub(crate) fn configured_gateway_url() -> Option<String> {
    GATEWAY_URL.get().cloned().flatten()
}

pub(crate) fn skip_connection_dialog() -> bool {
    SKIP_DIALOG.get().copied().unwrap_or(false)
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

/// Re-export shared client preference helpers from `rustyclaw-core` so the
/// desktop and TUI clients stay in lock-step.
pub use rustyclaw_core::client_prefs::{
    DEFAULT_GATEWAY_URL, load_auto_connect_gateway_urls, load_saved_gateway_url, save_gateway_url,
    should_bypass_connection_dialog,
};

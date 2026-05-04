//! RustyClaw Desktop Client library entrypoint.

use std::sync::OnceLock;

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub mod app;
mod components;
mod gateway;
mod state;

static GATEWAY_URL: OnceLock<Option<String>> = OnceLock::new();

pub fn run(gateway_url: Option<String>) {
    let _ = GATEWAY_URL.set(gateway_url);

    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .try_init();

    tracing::info!("Starting RustyClaw Desktop");
    dioxus::launch(app::App);
}

pub(crate) fn configured_gateway_url() -> Option<String> {
    GATEWAY_URL.get().cloned().flatten()
}
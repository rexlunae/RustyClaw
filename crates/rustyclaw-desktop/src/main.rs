//! RustyClaw Desktop Client
//!
//! A GUI alternative to the TUI client, built with Dioxus and dioxus-bulma.

use dioxus::prelude::*;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod app;
mod components;
mod gateway;
mod state;

fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting RustyClaw Desktop");

    // Launch the Dioxus app
    dioxus::launch(app::App);
}

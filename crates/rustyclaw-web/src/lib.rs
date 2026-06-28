//! `rustyclaw-web` — WebSocket/WASM web client for RustyClaw.
//!
//! This crate provides a WebSocket-based transport layer for connecting to
//! the RustyClaw gateway from a browser via WASM. It cfg-gates approximately
//! 39 desktop-only APIs that are not available in the browser environment.
//!
//! ## Architecture
//!
//! ```text
//! Browser ─── WebSocket ──→ Gateway (ws:// endpoint)
//!   │                           │
//!   └── rustyclaw-view ←────────┘
//!       (shared rendering)
//! ```
//!
//! The web client uses the same view data types from `rustyclaw-view` as the
//! TUI and desktop clients, but renders them via DOM manipulation or a
//! framework like Dioxus web.

pub mod transport;

use wasm_bindgen::prelude::*;

/// Initialize the web client WASM module.
#[wasm_bindgen(start)]
pub fn start() {
    // Set up panic hook for better error messages in the browser console.
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();

    web_sys::console::log_1(&"RustyClaw web client initialised".into());
}

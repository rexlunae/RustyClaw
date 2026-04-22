//! `rustyclaw-tui` — terminal UI client for RustyClaw, built on
//! [`iocraft`](https://crates.io/crates/iocraft).
//!
//! This crate provides the interactive TUI that connects to a local or remote
//! `rustyclaw-gateway` over WebSocket and renders the conversation in the
//! terminal.

pub mod action;
pub mod app;
pub mod components;
pub mod gateway_client;
pub mod markdown;
pub mod onboard;
pub mod pairing;
pub mod theme;
pub mod types;

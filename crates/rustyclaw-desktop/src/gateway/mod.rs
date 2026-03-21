//! Gateway WebSocket client.

mod client;
mod protocol;

pub use client::GatewayClient;
pub use protocol::GatewayEvent;

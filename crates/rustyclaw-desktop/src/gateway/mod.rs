//! Gateway client transport.

mod client;
mod protocol;

pub use client::GatewayClient;
pub use protocol::{GatewayCommand, GatewayEvent};

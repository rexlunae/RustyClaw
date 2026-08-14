//! Messenger construction from config.
//!
//! The construction logic itself lives in core beside the messenger-kind
//! registry (`rustyclaw_core::messengers::registry`), where in-tree kinds
//! and plugin-registered kinds build through the same path. What stays here
//! is the gateway's half of the contract: construction is synchronous in the
//! registry, and the gateway owns the async `initialize()`.

use anyhow::Result;
use rustyclaw_core::config::MessengerConfig;
use rustyclaw_core::messengers::{Messenger, messenger_registry};

/// Create and initialize a single messenger from config.
pub(crate) async fn create_messenger(config: &MessengerConfig) -> Result<Box<dyn Messenger>> {
    let mut messenger = messenger_registry().create(config)?;
    messenger.initialize().await?;
    Ok(messenger)
}

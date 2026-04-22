//! SSH pairing connection logic for TUI.
//!
//! This module handles the actual SSH connection to a gateway for pairing.

#[cfg(feature = "ssh")]
use anyhow::{Context, Result};

/// Connect to a gateway via SSH and register this client's public key.
///
/// Returns the gateway name on success.
#[cfg(feature = "ssh")]
pub async fn connect_and_pair(host: &str, port: u16, public_key: &str) -> Result<String> {
    use russh::client;
    use russh_keys::key::PublicKey;
    use std::sync::Arc;

    // Load the client keypair
    let keypair = rustyclaw_core::pairing::ClientKeyPair::load_or_generate(None)
        .context("Failed to load client keypair")?;

    // Create SSH client config
    let config = Arc::new(client::Config::default());

    // Connect to the gateway
    let addr = format!("{}:{}", host, port);
    tracing::info!("Connecting to gateway at {}", addr);

    // Create a client handler
    struct PairingHandler {
        gateway_name: Option<String>,
    }

    #[async_trait::async_trait]
    impl client::Handler for PairingHandler {
        type Error = anyhow::Error;

        async fn check_server_key(
            &mut self,
            _server_public_key: &PublicKey,
        ) -> Result<bool, Self::Error> {
            // For pairing, we accept any server key
            // In production, we'd want to verify/store the server fingerprint
            Ok(true)
        }
    }

    let handler = PairingHandler { gateway_name: None };

    // Attempt connection
    let mut session = client::connect(config, &addr, handler)
        .await
        .context("Failed to connect to gateway")?;

    // Authenticate with our keypair
    let key = keypair.load_private_key()?;
    let authenticated = session
        .authenticate_publickey("rustyclaw", Arc::new(key))
        .await
        .context("Failed to authenticate")?;

    if !authenticated {
        anyhow::bail!("Authentication failed - key not authorized on gateway");
    }

    tracing::info!("Successfully paired with gateway at {}", host);

    // For now, use the host as the gateway name
    // In the future, we could query the gateway for its actual name
    Ok(host.to_string())
}

/// Stub for when SSH feature is disabled.
#[cfg(not(feature = "ssh"))]
pub async fn connect_and_pair(
    _host: &str,
    _port: u16,
    _public_key: &str,
) -> anyhow::Result<String> {
    anyhow::bail!("SSH feature not enabled")
}

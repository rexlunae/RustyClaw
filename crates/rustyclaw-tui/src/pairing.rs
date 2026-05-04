//! SSH pairing connection logic for TUI.
//!
//! This module handles the actual SSH connection to a gateway for pairing.

use anyhow::{Context, Result};

/// Connect to a gateway via SSH and register this client's public key.
///
/// Returns the gateway name on success.
pub async fn connect_and_pair(host: &str, port: u16, public_key: &str) -> Result<String> {
    use russh::client;
    use russh::client::AuthResult;
    use russh::keys::{PrivateKeyWithHashAlg, PublicKey};
    use std::sync::Arc;

    let _ = public_key;

    // Load the client keypair
    let keypair = rustyclaw_core::pairing::ClientKeyPair::load_or_generate(None)
        .context("Failed to load client keypair")?;

    // Create SSH client config
    let config = Arc::new(client::Config::default());

    // Connect to the gateway
    let addr = format!("{}:{}", host, port);
    tracing::info!("Connecting to gateway at {}", addr);

    // Create a client handler
    struct PairingHandler;

    impl client::Handler for PairingHandler {
        type Error = anyhow::Error;

        async fn check_server_key(
            &mut self,
            _server_public_key: &PublicKey,
        ) -> std::result::Result<bool, Self::Error> {
            // For pairing, we accept any server key.
            // In production we'd want to verify/store the server fingerprint.
            Ok(true)
        }
    }

    let handler = PairingHandler;

    // Attempt connection
    let mut session = client::connect(config, &addr, handler)
        .await
        .context("Failed to connect to gateway")?;

    // Authenticate with our keypair
    let auth_result = session
        .authenticate_publickey(
            "rustyclaw",
            PrivateKeyWithHashAlg::new(Arc::new(keypair.private_key.clone()), None),
        )
        .await
        .context("Failed to authenticate")?;

    if !matches!(auth_result, AuthResult::Success) {
        anyhow::bail!("Authentication failed - key not authorized on gateway");
    }

    tracing::info!("Successfully paired with gateway at {}", host);

    // For now, use the host as the gateway name
    // In the future, we could query the gateway for its actual name
    Ok(host.to_string())
}

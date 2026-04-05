//! SSH pairing flow for RustyClaw clients and gateways.
//!
//! This module provides the infrastructure for pairing clients with gateways
//! using Ed25519 keypairs. The pairing flow works as follows:
//!
//! ## Client Side
//!
//! 1. Generate an Ed25519 keypair (stored in `~/.rustyclaw/client_ed25519_key`)
//! 2. Display the public key for copy/paste or as a QR code
//! 3. Connect to the gateway once the key has been authorized
//!
//! ## Gateway Side
//!
//! 1. Receive the client's public key (via QR scan, paste, or protocol)
//! 2. Display the key fingerprint for verification
//! 3. Add approved keys to `~/.rustyclaw/authorized_clients`
//!
//! ## File Formats
//!
//! ### Client Private Key (`~/.rustyclaw/client_ed25519_key`)
//!
//! Standard OpenSSH private key format (PEM-encoded).
//!
//! ### Authorized Clients (`~/.rustyclaw/authorized_clients`)
//!
//! Same format as OpenSSH's `authorized_keys`:
//! ```text
//! # RustyClaw authorized clients
//! ssh-ed25519 AAAAC3NzaC1lZDI1NTE5... laptop@user
//! ssh-ed25519 AAAAC3NzaC1lZDI1NTE5... phone@user
//! ```

mod client_keys;
mod authorized;
mod qr;
mod fingerprint;

pub use client_keys::{
    ClientKeyPair,
    generate_client_keypair,
    load_client_keypair,
    save_client_keypair,
    default_client_key_path,
};

pub use authorized::{
    AuthorizedClient,
    AuthorizedClients,
    load_authorized_clients,
    add_authorized_client,
    remove_authorized_client,
    default_authorized_clients_path,
};

pub use qr::{
    generate_pairing_qr,
    generate_pairing_qr_ascii,
    parse_pairing_qr,
    PairingData,
};

pub use fingerprint::{
    key_fingerprint,
    key_fingerprint_short,
    format_fingerprint_art,
};

/// Default directory for RustyClaw configuration and keys.
pub fn rustyclaw_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".rustyclaw"))
        .unwrap_or_else(|| std::path::PathBuf::from(".rustyclaw"))
}

/// Ensure the RustyClaw directory exists with proper permissions.
pub fn ensure_rustyclaw_dir() -> std::io::Result<()> {
    let dir = rustyclaw_dir();
    std::fs::create_dir_all(&dir)?;
    
    // Set directory permissions to 700 on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    
    Ok(())
}

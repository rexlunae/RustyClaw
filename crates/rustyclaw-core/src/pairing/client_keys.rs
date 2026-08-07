//! Client keypair generation and management.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// An Ed25519 keypair for client authentication.
#[derive(Clone)]
pub struct ClientKeyPair {
    /// The private key (kept secret).
    pub private_key: russh::keys::PrivateKey,

    /// The public key (shared with gateway).
    pub public_key: russh::keys::PublicKey,

    /// Optional comment (e.g., "user@hostname").
    pub comment: Option<String>,
}

impl ClientKeyPair {
    /// Load or generate a client keypair at the default location.
    ///
    /// If `comment` is None, generates "rustyclaw@client".
    pub fn load_or_generate(comment: Option<String>) -> Result<Self> {
        let path = default_client_key_path();
        let comment = comment.or_else(|| Some("rustyclaw@client".to_string()));
        load_or_generate_client_keypair(&path, comment)
    }

    /// Load the private key for SSH authentication.
    pub fn load_private_key(&self) -> Result<russh::keys::PrivateKey> {
        let path = default_client_key_path();
        let key_data = std::fs::read_to_string(&path).context("Failed to read private key")?;
        russh::keys::decode_secret_key(&key_data, None).context("Failed to decode private key")
    }

    /// Get the public key in OpenSSH format (for display/copy).
    pub fn public_key_openssh(&self) -> String {
        {
            let key_str = self
                .public_key
                .to_openssh()
                .unwrap_or_else(|_| format!("{:?}", self.public_key));
            if let Some(ref comment) = self.comment {
                format!("{} {}", key_str.trim(), comment)
            } else {
                key_str.trim().to_string()
            }
        }
    }

    /// Get the key fingerprint (SHA256).
    pub fn fingerprint(&self) -> String {
        super::key_fingerprint(self)
    }

    /// Get a short fingerprint (last 8 characters).
    pub fn fingerprint_short(&self) -> String {
        super::key_fingerprint_short(self)
    }
}

/// Default path for the client private key.
pub fn default_client_key_path() -> PathBuf {
    super::rustyclaw_dir().join("client_ed25519_key")
}

/// Generate a new Ed25519 keypair for client authentication.
///
/// The `comment` is typically "user@hostname" and will be appended to
/// the public key when displayed.
pub fn generate_client_keypair(comment: Option<String>) -> Result<ClientKeyPair> {
    use russh::keys::{Algorithm, PrivateKey};

    // Generate Ed25519 key. russh 0.61 / ssh-key 0.7 take a CryptoRng; use
    // rand's thread RNG (matches russh's own key-gen usage).
    let private_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
        .context("Failed to generate Ed25519 keypair")?;

    let public_key = private_key.public_key().clone();

    Ok(ClientKeyPair {
        private_key,
        public_key,
        comment,
    })
}

/// Load an existing client keypair from disk.
pub fn load_client_keypair(path: &Path) -> Result<ClientKeyPair> {
    let key_data = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read key file: {}", path.display()))?;

    let private_key = russh::keys::PrivateKey::from_openssh(&key_data)
        .with_context(|| format!("Failed to parse key: {}", path.display()))?;

    let public_key = private_key.public_key().clone();
    let comment = {
        let c = public_key.comment();
        if c.is_empty() {
            None
        } else {
            Some(c.to_string())
        }
    };

    Ok(ClientKeyPair {
        private_key,
        public_key,
        comment,
    })
}

/// Save a client keypair to disk.
///
/// The private key is saved with restrictive permissions (600 on Unix).
pub fn save_client_keypair(keypair: &ClientKeyPair, path: &Path) -> Result<()> {
    use russh::keys::ssh_key::LineEnding;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    // Encode private key in OpenSSH format
    let key_data = keypair
        .private_key
        .to_openssh(LineEnding::LF)
        .context("Failed to encode private key")?;

    // Atomic and 0o600 from the first byte: a write-then-chmod leaves the
    // key world-readable for the umask window, and a torn write leaves a
    // half-key that `path.exists()` used to mistake for a working one.
    crate::persist::write_atomically_private(path, key_data.as_bytes())
        .with_context(|| format!("Failed to write key: {}", path.display()))?;

    Ok(())
}

/// Load or generate a client keypair.
///
/// If the keypair exists at `path`, loads it. Otherwise, generates a new
/// keypair and saves it.
pub fn load_or_generate_client_keypair(
    path: &Path,
    comment: Option<String>,
) -> Result<ClientKeyPair> {
    if path.exists() {
        match load_client_keypair(path) {
            Ok(keypair) => Ok(keypair),
            // A key file that exists but does not parse used to fail every
            // gateway connection until the user deleted it by hand. It is
            // not a working identity — quarantine it and mint a fresh one.
            // The gateway will see an unknown key, which is the pairing
            // flow, not an outage.
            Err(e) => {
                crate::persist::quarantine(path, &format!("{e:#}"));
                let keypair = generate_client_keypair(comment)?;
                save_client_keypair(&keypair, path)?;
                Ok(keypair)
            }
        }
    } else {
        let keypair = generate_client_keypair(comment)?;
        save_client_keypair(&keypair, path)?;
        Ok(keypair)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_client_key_path() {
        let path = default_client_key_path();
        assert!(path.to_string_lossy().contains("client_ed25519_key"));
    }

    /// A key file that exists but does not parse used to fail every
    /// connection until the user found and deleted it by hand. It must be
    /// quarantined and replaced instead — re-pairing is the recovery flow,
    /// a client that cannot connect at all is not.
    #[test]
    fn a_corrupt_key_file_is_replaced_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("client_key");
        std::fs::write(&path, b"-----BEGIN NONSENSE-----").unwrap();

        let keypair = load_or_generate_client_keypair(&path, None).expect("a fresh key is minted");
        // The new key on disk parses and matches what was returned.
        let reloaded = load_client_keypair(&path).unwrap();
        assert_eq!(
            keypair.public_key.to_openssh().unwrap(),
            reloaded.public_key.to_openssh().unwrap()
        );

        // The old bytes were moved aside, not destroyed.
        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert_eq!(quarantined.len(), 1);
    }

    /// The private key must never exist on disk with permissions wider
    /// than the owner — not even between write and a later chmod.
    #[cfg(unix)]
    #[test]
    fn a_saved_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("client_key");
        load_or_generate_client_keypair(&path, None).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "key mode was {mode:o}");
    }
}

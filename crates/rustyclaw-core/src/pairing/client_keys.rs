//! Client keypair generation and management.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// An Ed25519 keypair for client authentication.
#[derive(Clone)]
pub struct ClientKeyPair {
    /// The private key (kept secret).
    #[cfg(feature = "ssh")]
    pub private_key: russh::keys::PrivateKey,
    #[cfg(not(feature = "ssh"))]
    pub private_key_pem: String,
    
    /// The public key (shared with gateway).
    #[cfg(feature = "ssh")]
    pub public_key: russh::keys::PublicKey,
    #[cfg(not(feature = "ssh"))]
    pub public_key_openssh: String,
    
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
    #[cfg(feature = "ssh")]
    pub fn load_private_key(&self) -> Result<russh_keys::key::KeyPair> {
        let path = default_client_key_path();
        let key_data = std::fs::read_to_string(&path)
            .context("Failed to read private key")?;
        russh_keys::decode_secret_key(&key_data, None)
            .context("Failed to decode private key")
    }
    
    /// Get the public key in OpenSSH format (for display/copy).
    pub fn public_key_openssh(&self) -> String {
        #[cfg(feature = "ssh")]
        {
            let key_str = self.public_key
                .to_openssh()
                .unwrap_or_else(|_| format!("{:?}", self.public_key));
            if let Some(ref comment) = self.comment {
                format!("{} {}", key_str.trim(), comment)
            } else {
                key_str.trim().to_string()
            }
        }
        #[cfg(not(feature = "ssh"))]
        {
            if let Some(ref comment) = self.comment {
                format!("{} {}", self.public_key_openssh.trim(), comment)
            } else {
                self.public_key_openssh.clone()
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
#[cfg(feature = "ssh")]
pub fn generate_client_keypair(comment: Option<String>) -> Result<ClientKeyPair> {
    use russh::keys::{Algorithm, PrivateKey};
    
    // Generate Ed25519 key
    let private_key = PrivateKey::random(&mut rand_core::OsRng, Algorithm::Ed25519)
        .context("Failed to generate Ed25519 keypair")?;
    
    let public_key = private_key.public_key().clone();
    
    Ok(ClientKeyPair {
        private_key,
        public_key,
        comment,
    })
}

#[cfg(not(feature = "ssh"))]
pub fn generate_client_keypair(_comment: Option<String>) -> Result<ClientKeyPair> {
    anyhow::bail!("SSH feature not enabled; cannot generate keypair")
}

/// Load an existing client keypair from disk.
#[cfg(feature = "ssh")]
pub fn load_client_keypair(path: &Path) -> Result<ClientKeyPair> {
    let key_data = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read key file: {}", path.display()))?;
    
    let private_key = russh::keys::PrivateKey::from_openssh(&key_data)
        .with_context(|| format!("Failed to parse key: {}", path.display()))?;
    
    let public_key = private_key.public_key().clone();
    let comment = {
        let c = public_key.comment();
        if c.is_empty() { None } else { Some(c.to_string()) }
    };
    
    Ok(ClientKeyPair {
        private_key,
        public_key,
        comment,
    })
}

#[cfg(not(feature = "ssh"))]
pub fn load_client_keypair(path: &Path) -> Result<ClientKeyPair> {
    let _key_data = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read key file: {}", path.display()))?;
    
    // Parse just enough to extract the public key line
    // This is a simplified fallback when SSH feature is disabled
    anyhow::bail!("SSH feature not enabled; cannot load keypair from {}", path.display())
}

/// Save a client keypair to disk.
///
/// The private key is saved with restrictive permissions (600 on Unix).
#[cfg(feature = "ssh")]
pub fn save_client_keypair(keypair: &ClientKeyPair, path: &Path) -> Result<()> {
    use russh::keys::ssh_key::LineEnding;
    
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }
    
    // Encode private key in OpenSSH format
    let key_data = keypair.private_key
        .to_openssh(LineEnding::LF)
        .context("Failed to encode private key")?;
    
    // Write the key
    std::fs::write(path, key_data.as_bytes())
        .with_context(|| format!("Failed to write key: {}", path.display()))?;
    
    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions: {}", path.display()))?;
    }
    
    Ok(())
}

#[cfg(not(feature = "ssh"))]
pub fn save_client_keypair(_keypair: &ClientKeyPair, _path: &Path) -> Result<()> {
    anyhow::bail!("SSH feature not enabled; cannot save keypair")
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
        load_client_keypair(path)
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
    
    #[test]
    #[cfg(feature = "ssh")]
    fn test_generate_keypair() {
        let keypair = generate_client_keypair(Some("test@localhost".to_string()))
            .expect("Should generate keypair");
        
        let openssh = keypair.public_key_openssh();
        assert!(openssh.starts_with("ssh-ed25519 "));
        assert!(openssh.contains("test@localhost"));
    }
}

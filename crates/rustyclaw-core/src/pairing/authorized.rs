//! Authorized clients management (gateway side).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// An authorized client entry.
#[derive(Debug, Clone)]
pub struct AuthorizedClient {
    /// The public key in OpenSSH format.
    pub public_key_openssh: String,

    /// The key fingerprint (SHA256).
    pub fingerprint: String,

    /// Optional comment (usually client-name@host).
    pub comment: Option<String>,

    /// When the key was authorized (Unix timestamp).
    pub authorized_at: Option<u64>,
}

impl AuthorizedClient {
    /// Parse from an OpenSSH authorized_keys line.
    pub fn from_line(line: &str) -> Option<Self> {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            return None;
        }

        // Parse: "ssh-ed25519 AAAA... comment"
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() < 2 {
            return None;
        }

        let key_type = parts[0];
        let key_data = parts[1];
        let comment = parts.get(2).map(|s| s.to_string());

        // Reconstruct the key for fingerprinting
        let public_key_openssh = if let Some(ref c) = comment {
            format!("{} {} {}", key_type, key_data, c)
        } else {
            format!("{} {}", key_type, key_data)
        };

        // Calculate fingerprint
        let fingerprint = calculate_fingerprint(&public_key_openssh);

        Some(AuthorizedClient {
            public_key_openssh,
            fingerprint,
            comment,
            authorized_at: None,
        })
    }

    /// Format as an authorized_keys line.
    pub fn to_line(&self) -> String {
        self.public_key_openssh.clone()
    }
}

/// Collection of authorized clients.
#[derive(Debug, Clone, Default)]
pub struct AuthorizedClients {
    /// List of authorized clients.
    pub clients: Vec<AuthorizedClient>,

    /// Path to the authorized_clients file.
    pub path: PathBuf,
}

impl AuthorizedClients {
    /// Create an empty authorized clients list.
    pub fn new(path: PathBuf) -> Self {
        Self {
            clients: Vec::new(),
            path,
        }
    }

    /// Check if a public key is authorized.
    pub fn is_authorized(&self, public_key_openssh: &str) -> bool {
        // Normalize the key for comparison (strip comment, whitespace)
        let normalized = normalize_key(public_key_openssh);

        self.clients
            .iter()
            .any(|c| normalize_key(&c.public_key_openssh) == normalized)
    }

    /// Find a client by fingerprint.
    pub fn find_by_fingerprint(&self, fingerprint: &str) -> Option<&AuthorizedClient> {
        self.clients.iter().find(|c| c.fingerprint == fingerprint)
    }

    /// Add a new client.
    pub fn add(&mut self, client: AuthorizedClient) {
        // Don't add duplicates
        if !self.is_authorized(&client.public_key_openssh) {
            self.clients.push(client);
        }
    }

    /// Remove a client by fingerprint.
    pub fn remove_by_fingerprint(&mut self, fingerprint: &str) -> bool {
        let original_len = self.clients.len();
        self.clients.retain(|c| c.fingerprint != fingerprint);
        self.clients.len() < original_len
    }
}

/// Default path for the authorized_clients file.
pub fn default_authorized_clients_path() -> PathBuf {
    super::rustyclaw_dir().join("authorized_clients")
}

/// Load authorized clients from a file.
pub fn load_authorized_clients(path: &Path) -> Result<AuthorizedClients> {
    let mut clients = AuthorizedClients::new(path.to_path_buf());

    if !path.exists() {
        // Return empty list if file doesn't exist
        return Ok(clients);
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read authorized_clients: {}", path.display()))?;

    for (line_num, line) in content.lines().enumerate() {
        match AuthorizedClient::from_line(line) {
            Some(client) => clients.clients.push(client),
            None if line.trim().is_empty() || line.trim().starts_with('#') => {
                // Ignore empty lines and comments
            }
            None => {
                warn!(
                    line = line_num + 1,
                    content = line,
                    "Failed to parse authorized_clients line"
                );
            }
        }
    }

    info!(
        path = %path.display(),
        count = clients.clients.len(),
        "Loaded authorized clients"
    );

    Ok(clients)
}

/// Add a client to the authorized_clients file.
pub fn add_authorized_client(
    path: &Path,
    public_key_openssh: &str,
    comment: Option<&str>,
) -> Result<AuthorizedClient> {
    use std::io::Write;

    // Build the key line
    let key_line = if let Some(c) = comment {
        // If key already has a comment, replace it
        let parts: Vec<&str> = public_key_openssh.splitn(3, ' ').collect();
        if parts.len() >= 2 {
            format!("{} {} {}", parts[0], parts[1], c)
        } else {
            format!("{} {}", public_key_openssh.trim(), c)
        }
    } else {
        public_key_openssh.trim().to_string()
    };

    // Parse into AuthorizedClient
    let client = AuthorizedClient::from_line(&key_line).context("Invalid public key format")?;

    // Check if already authorized
    let existing = load_authorized_clients(path)?;
    if existing.is_authorized(&client.public_key_openssh) {
        anyhow::bail!("Key is already authorized");
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Append to file
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open authorized_clients: {}", path.display()))?;

    // Add header comment if file is new/empty
    let metadata = file.metadata()?;
    if metadata.len() == 0 {
        writeln!(file, "# RustyClaw authorized clients")?;
        writeln!(file, "# Format: ssh-ed25519 <key> <comment>")?;
        writeln!(file)?;
    }

    writeln!(file, "{}", key_line)?;

    info!(
        path = %path.display(),
        fingerprint = %client.fingerprint,
        comment = ?client.comment,
        "Added authorized client"
    );

    Ok(client)
}

/// Remove a client from the authorized_clients file by fingerprint.
pub fn remove_authorized_client(path: &Path, fingerprint: &str) -> Result<bool> {
    let mut clients = load_authorized_clients(path)?;

    if !clients.remove_by_fingerprint(fingerprint) {
        return Ok(false);
    }

    // Rewrite the file
    let mut content = String::from("# RustyClaw authorized clients\n");
    content.push_str("# Format: ssh-ed25519 <key> <comment>\n\n");

    for client in &clients.clients {
        content.push_str(&client.to_line());
        content.push('\n');
    }

    std::fs::write(path, content)
        .with_context(|| format!("Failed to write authorized_clients: {}", path.display()))?;

    info!(
        path = %path.display(),
        fingerprint = fingerprint,
        "Removed authorized client"
    );

    Ok(true)
}

/// Normalize a public key for comparison.
///
/// Strips comments and extra whitespace, keeping only "type base64key".
fn normalize_key(key: &str) -> String {
    let parts: Vec<&str> = key.split_whitespace().collect();
    if parts.len() >= 2 {
        format!("{} {}", parts[0], parts[1])
    } else {
        key.trim().to_string()
    }
}

/// Calculate the SHA256 fingerprint of a public key.
#[cfg(feature = "ssh")]
fn calculate_fingerprint(public_key_openssh: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    // Parse the key to get the base64 data
    let parts: Vec<&str> = public_key_openssh.split_whitespace().collect();
    if parts.len() < 2 {
        return "SHA256:invalid".to_string();
    }

    // Decode the base64 key data
    let key_data = match base64::engine::general_purpose::STANDARD.decode(parts[1]) {
        Ok(data) => data,
        Err(_) => return "SHA256:invalid".to_string(),
    };

    // Calculate SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(&key_data);
    let hash = hasher.finalize();

    // Encode as base64 (without padding, to match ssh-keygen format)
    let fingerprint = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&hash);

    format!("SHA256:{}", fingerprint)
}

/// Calculate fingerprint stub when ssh feature is disabled.
#[cfg(not(feature = "ssh"))]
fn calculate_fingerprint(_public_key_openssh: &str) -> String {
    "SHA256:unavailable".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_authorized_line() {
        let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKtJvJZDLNbPkTYf4ZbXaBeCq3I9sEG9qS9XvGBFMT4C test@localhost";
        let client = AuthorizedClient::from_line(line).expect("Should parse");

        assert!(client.public_key_openssh.starts_with("ssh-ed25519"));
        assert_eq!(client.comment, Some("test@localhost".to_string()));
        assert!(client.fingerprint.starts_with("SHA256:"));
    }

    #[test]
    fn test_normalize_key() {
        let key1 = "ssh-ed25519 AAAA... user@host";
        let key2 = "ssh-ed25519   AAAA...   different@comment";

        assert_eq!(normalize_key(key1), "ssh-ed25519 AAAA...");
        assert_eq!(normalize_key(key2), "ssh-ed25519 AAAA...");
    }

    #[test]
    fn test_skip_comments() {
        assert!(AuthorizedClient::from_line("# This is a comment").is_none());
        assert!(AuthorizedClient::from_line("").is_none());
        assert!(AuthorizedClient::from_line("   ").is_none());
    }
}

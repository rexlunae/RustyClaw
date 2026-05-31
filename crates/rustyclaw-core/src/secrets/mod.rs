//! Secrets manager backed by an encrypted SecureStore vault.
//!
//! The vault is stored at `{credentials_dir}/secrets.json`.  Encryption uses
//! either a CSPRNG-generated key file (`{credentials_dir}/secrets.key`) or a
//! user-supplied password — never both.
//!
//! ## Storage layout
//!
//! | Key pattern            | Content                                          |
//! |------------------------|--------------------------------------------------|
//! | `cred:<name>`          | JSON-serialized [`SecretEntry`] metadata          |
//! | `val:<name>`           | Primary secret value (or private key PEM / note)  |
//! | `val:<name>:user`      | Username (for `UsernamePassword` kind)             |
//! | `val:<name>:pub`       | Public key string (for `SshKey` kind)              |
//! | `val:<name>:fields`    | JSON map of form-field key/value pairs             |
//! | `val:<name>:card`      | JSON `{cardholder,number,expiry,cvv}`              |
//! | `val:<name>:card_extra`| JSON map of additional payment card fields         |
//! | `<bare key>`           | Legacy / raw secrets (API keys, TOTP, etc.)        |

mod types;
mod vault;

use std::path::PathBuf;

pub use types::{
    AccessContext, AccessPolicy, BrowserStore, Cookie, CredentialValue, Secret, SecretEntry,
    SecretKind, SecretString, WebStorage,
};

/// Secrets manager backed by an encrypted SecureStore vault.
pub struct SecretsManager {
    /// Path to the vault JSON file
    pub(crate) vault_path: PathBuf,
    /// Path to the key file (only used when no password is set)
    pub(crate) key_path: PathBuf,
    /// Optional user-supplied password (used instead of the key file)
    pub(crate) password: Option<String>,
    /// In-memory vault handle (loaded lazily)
    pub(crate) vault: Option<securestore::SecretsManager>,
    /// Whether the agent can access secrets without prompting
    pub(crate) agent_access_enabled: bool,
}

impl SecretsManager {
    /// Create a new `SecretsManager` rooted in `credentials_dir`.
    ///
    /// The vault and key files are created on-demand the first time a
    /// mutating operation is performed.
    pub fn new(credentials_dir: impl Into<PathBuf>) -> Self {
        let dir: PathBuf = credentials_dir.into();
        Self {
            vault_path: dir.join("secrets.json"),
            key_path: dir.join("secrets.key"),
            password: None,
            vault: None,
            agent_access_enabled: false,
        }
    }

    /// Create a `SecretsManager` that uses a password for encryption
    /// instead of a key file.
    pub fn with_password(credentials_dir: impl Into<PathBuf>, password: String) -> Self {
        let dir: PathBuf = credentials_dir.into();
        Self {
            vault_path: dir.join("secrets.json"),
            key_path: dir.join("secrets.key"),
            password: Some(password),
            vault: None,
            agent_access_enabled: false,
        }
    }

    /// Set the password after construction (e.g. after prompting the user).
    ///
    /// **Note:** This only affects how the vault is opened on next access.
    /// If the vault already exists on disk with a different key source, you
    /// must call [`change_password`](Self::change_password) instead.
    pub fn set_password(&mut self, password: String) {
        self.password = Some(password);
        // Invalidate any previously loaded vault so it reloads with the
        // new key source.
        self.vault = None;
    }

    /// Remove the password and invalidate the loaded vault, returning the
    /// manager to a locked state.
    pub fn clear_password(&mut self) {
        self.password = None;
        self.vault = None;
    }

    /// Create a `SecretsManager` in a locked state.
    ///
    /// The vault file path is known but no password or key file has been
    /// provided yet.  The vault cannot be accessed until
    /// [`set_password`](Self::set_password) is called.
    pub fn locked(credentials_dir: impl Into<PathBuf>) -> Self {
        let dir: PathBuf = credentials_dir.into();
        Self {
            vault_path: dir.join("secrets.json"),
            key_path: dir.join("secrets.key"),
            password: None,
            vault: None,
            agent_access_enabled: false,
        }
    }

    /// Check whether the vault is in a locked state (password-protected
    /// vault with no password provided yet).
    ///
    /// Returns `true` if the vault file exists on disk, no key file is
    /// present, and no password has been set — meaning the vault cannot
    /// be decrypted without a password.
    pub fn is_locked(&self) -> bool {
        self.vault.is_none()
            && self.password.is_none()
            && !self.key_path.exists()
            && self.vault_path.exists()
    }

    /// Return the current password, if one has been set.
    ///
    /// Used by the TUI to forward the vault password to the gateway
    /// daemon so it can open the vault without prompting.
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    // ── Access control ──────────────────────────────────────────────

    /// Enable or disable automatic agent access to secrets
    pub fn set_agent_access(&mut self, enabled: bool) {
        self.agent_access_enabled = enabled;
    }

    /// Check if agent has access to secrets
    pub fn has_agent_access(&self) -> bool {
        self.agent_access_enabled
    }
}

#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use securestore::KeySource;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use totp_rs::{Algorithm, TOTP, Secret as TotpSecret};

/// Secrets manager backed by an encrypted SecureStore vault.
///
/// The vault is stored at `{settings_dir}/secrets.json`.  Encryption uses
/// either a CSPRNG-generated key file (`{settings_dir}/secrets.key`) or a
/// user-supplied password — never both.
pub struct SecretsManager {
    /// Path to the vault JSON file
    vault_path: PathBuf,
    /// Path to the key file (only used when no password is set)
    key_path: PathBuf,
    /// Optional user-supplied password (used instead of the key file)
    password: Option<String>,
    /// In-memory vault handle (loaded lazily)
    vault: Option<securestore::SecretsManager>,
    /// Whether the agent can access secrets without prompting
    agent_access_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    pub key: String,
    pub description: Option<String>,
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
    pub fn set_password(&mut self, password: String) {
        self.password = Some(password);
        // Invalidate any previously loaded vault so it reloads with the
        // new key source.
        self.vault = None;
    }

    /// Ensure the vault is loaded (or created if it doesn't exist yet).
    fn ensure_vault(&mut self) -> Result<&mut securestore::SecretsManager> {
        if self.vault.is_none() {
            let vault = if self.vault_path.exists() {
                // Existing vault — load with password or key file.
                if let Some(ref pw) = self.password {
                    securestore::SecretsManager::load(
                        &self.vault_path,
                        KeySource::Password(pw),
                    )
                    .context("Failed to load secrets vault (wrong password?)")?
                } else if self.key_path.exists() {
                    securestore::SecretsManager::load(
                        &self.vault_path,
                        KeySource::from_file(&self.key_path),
                    )
                    .context("Failed to load secrets vault")?
                } else {
                    anyhow::bail!(
                        "Secrets vault exists but no key file or password provided. \
                         Run `rustyclaw onboard` to configure."
                    );
                }
            } else {
                // First run: create a brand-new vault.
                if let Some(parent) = self.vault_path.parent() {
                    std::fs::create_dir_all(parent)
                        .context("Failed to create secrets directory")?;
                }
                if let Some(ref pw) = self.password {
                    // Password-based vault — no key file needed.
                    let sman = securestore::SecretsManager::new(KeySource::Password(pw))
                        .context("Failed to create new secrets vault")?;
                    sman.save_as(&self.vault_path)
                        .context("Failed to save new secrets vault")?;
                    securestore::SecretsManager::load(
                        &self.vault_path,
                        KeySource::Password(pw),
                    )
                    .context("Failed to reload newly-created secrets vault")?
                } else {
                    // Key-file-based vault.
                    let sman = securestore::SecretsManager::new(KeySource::Csprng)
                        .context("Failed to create new secrets vault")?;
                    sman.export_key(&self.key_path)
                        .context("Failed to export secrets key")?;
                    sman.save_as(&self.vault_path)
                        .context("Failed to save new secrets vault")?;
                    securestore::SecretsManager::load(
                        &self.vault_path,
                        KeySource::from_file(&self.key_path),
                    )
                    .context("Failed to reload newly-created secrets vault")?
                }
            };
            self.vault = Some(vault);
        }
        // SAFETY: we just ensured `self.vault` is `Some`.
        Ok(self.vault.as_mut().unwrap())
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

    // ── CRUD operations ─────────────────────────────────────────────

    /// Store (or overwrite) a secret in the vault and persist to disk.
    pub fn store_secret(&mut self, key: &str, value: &str) -> Result<()> {
        let vault = self.ensure_vault()?;
        vault.set(key, value);
        vault.save().context("Failed to save secrets vault")?;
        Ok(())
    }

    /// Retrieve a secret from the vault.
    ///
    /// Returns `None` if the secret does not exist **or** if agent
    /// access is disabled and the caller has not provided explicit
    /// user approval.
    pub fn get_secret(&mut self, key: &str, user_approved: bool) -> Result<Option<String>> {
        if !self.agent_access_enabled && !user_approved {
            return Ok(None);
        }

        let vault = self.ensure_vault()?;
        match vault.get(key) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.kind() == securestore::ErrorKind::SecretNotFound => Ok(None),
            Err(e) => Err(anyhow::anyhow!("Failed to get secret: {}", e)),
        }
    }

    /// Delete a secret from the vault and persist to disk.
    pub fn delete_secret(&mut self, key: &str) -> Result<()> {
        let vault = self.ensure_vault()?;
        vault.remove(key).context("Failed to remove secret")?;
        vault.save().context("Failed to save secrets vault")?;
        Ok(())
    }

    /// List all stored secret keys (not values).
    pub fn list_secrets(&mut self) -> Vec<String> {
        match self.ensure_vault() {
            Ok(vault) => vault.keys().map(|s| s.to_string()).collect(),
            Err(_) => Vec::new(),
        }
    }

    // ── TOTP two-factor authentication ──────────────────────────────

    /// The vault key used to store the TOTP shared secret.
    const TOTP_SECRET_KEY: &'static str = "__rustyclaw_totp_secret";

    /// Generate a fresh TOTP secret, store it in the vault, and return
    /// the `otpauth://` URI (suitable for QR codes / manual entry in an
    /// authenticator app).
    pub fn setup_totp(&mut self, account_name: &str) -> Result<String> {
        let secret = TotpSecret::generate_secret();
        let secret_bytes = secret.to_bytes()
            .map_err(|e| anyhow::anyhow!("Failed to generate TOTP secret bytes: {:?}", e))?;

        let totp = TOTP::new(
            Algorithm::SHA1,
            6,  // digits
            1,  // skew (allow ±1 step)
            30, // step (seconds)
            secret_bytes,
            Some("RustyClaw".to_string()),
            account_name.to_string(),
        )
        .map_err(|e| anyhow::anyhow!("Failed to create TOTP: {:?}", e))?;

        // Store the base32-encoded secret in the vault.
        let encoded = secret.to_encoded().to_string();
        self.store_secret(Self::TOTP_SECRET_KEY, &encoded)?;

        Ok(totp.get_url())
    }

    /// Verify a 6-digit TOTP code against the stored secret.
    /// Returns `Ok(true)` if the code is valid, `Ok(false)` if invalid,
    /// or an error if no TOTP secret is configured.
    pub fn verify_totp(&mut self, code: &str) -> Result<bool> {
        let encoded = self.get_secret(Self::TOTP_SECRET_KEY, true)?
            .ok_or_else(|| anyhow::anyhow!("No TOTP secret configured"))?;

        let secret = TotpSecret::Encoded(encoded);
        let secret_bytes = secret.to_bytes()
            .map_err(|e| anyhow::anyhow!("Corrupted TOTP secret: {:?}", e))?;

        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret_bytes,
            Some("RustyClaw".to_string()),
            String::new(),
        )
        .map_err(|e| anyhow::anyhow!("Failed to create TOTP: {:?}", e))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("System time error")?
            .as_secs();

        Ok(totp.check(code, now))
    }

    /// Check whether a TOTP secret is stored in the vault.
    pub fn has_totp(&mut self) -> bool {
        self.get_secret(Self::TOTP_SECRET_KEY, true)
            .ok()
            .flatten()
            .is_some()
    }

    /// Remove the stored TOTP secret (disables 2FA).
    pub fn remove_totp(&mut self) -> Result<()> {
        if self.has_totp() {
            self.delete_secret(Self::TOTP_SECRET_KEY)?;
        }
        Ok(())
    }

    /// No-op kept for API compatibility.  The securestore crate
    /// decrypts on-demand so there is no separate cache to clear.
    pub fn clear_cache(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw_test_{}_{}", std::process::id(), id
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_secrets_manager_creation() {
        let dir = temp_dir();
        let manager = SecretsManager::new(&dir);
        assert!(!manager.has_agent_access());

        // Vault files should not exist yet (lazy creation)
        assert!(!dir.join("secrets.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_agent_access_control() {
        let dir = temp_dir();
        let mut manager = SecretsManager::new(&dir);
        assert!(!manager.has_agent_access());

        manager.set_agent_access(true);
        assert!(manager.has_agent_access());

        manager.set_agent_access(false);
        assert!(!manager.has_agent_access());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_store_and_retrieve() {
        let dir = temp_dir();
        let mut manager = SecretsManager::new(&dir);
        manager.set_agent_access(true);

        manager.store_secret("api_key", "hunter2").unwrap();
        assert!(Path::new(&dir.join("secrets.json")).exists());
        assert!(Path::new(&dir.join("secrets.key")).exists());

        let val = manager.get_secret("api_key", false).unwrap();
        assert_eq!(val, Some("hunter2".to_string()));

        // Non-existent key
        let missing = manager.get_secret("nope", true).unwrap();
        assert_eq!(missing, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_list_and_delete() {
        let dir = temp_dir();
        let mut manager = SecretsManager::new(&dir);
        manager.set_agent_access(true);

        manager.store_secret("a", "1").unwrap();
        manager.store_secret("b", "2").unwrap();

        let mut keys = manager.list_secrets();
        keys.sort();
        assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);

        manager.delete_secret("a").unwrap();
        let keys = manager.list_secrets();
        assert_eq!(keys, vec!["b".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_access_denied_without_approval() {
        let dir = temp_dir();
        let mut manager = SecretsManager::new(&dir);
        manager.store_secret("secret", "value").unwrap();

        // Agent access off + no user approval → None
        let val = manager.get_secret("secret", false).unwrap();
        assert_eq!(val, None);

        // With user approval → Some
        let val = manager.get_secret("secret", true).unwrap();
        assert_eq!(val, Some("value".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reload_from_disk() {
        let dir = temp_dir();

        // Create and populate
        {
            let mut m = SecretsManager::new(&dir);
            m.store_secret("persist", "yes").unwrap();
        }

        // Load fresh and read back
        {
            let mut m = SecretsManager::new(&dir);
            m.set_agent_access(true);
            let val = m.get_secret("persist", false).unwrap();
            assert_eq!(val, Some("yes".to_string()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_password_based_vault() {
        let dir = temp_dir();

        // Create a password-protected vault and store a secret.
        {
            let mut m = SecretsManager::with_password(&dir, "s3cret".to_string());
            m.store_secret("token", "abc123").unwrap();
        }

        // Vault file should exist, but key file should NOT.
        assert!(dir.join("secrets.json").exists());
        assert!(!dir.join("secrets.key").exists());

        // Reload with the correct password.
        {
            let mut m = SecretsManager::with_password(&dir, "s3cret".to_string());
            m.set_agent_access(true);
            let val = m.get_secret("token", false).unwrap();
            assert_eq!(val, Some("abc123".to_string()));
        }

        // Wrong password should fail to load.
        {
            let mut m = SecretsManager::with_password(&dir, "wrong".to_string());
            assert!(m.get_secret("token", true).is_err());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_totp_setup_and_verify() {
        let dir = temp_dir();
        let mut manager = SecretsManager::new(&dir);
        manager.set_agent_access(true);

        // No TOTP secret initially.
        assert!(!manager.has_totp());

        // Set up TOTP and get the otpauth:// URL.
        let url = manager.setup_totp("testuser").unwrap();
        assert!(url.starts_with("otpauth://totp/"));
        assert!(url.contains("RustyClaw"));
        assert!(manager.has_totp());

        // Generate a valid code from the stored secret and verify it.
        let encoded = manager.get_secret(SecretsManager::TOTP_SECRET_KEY, true)
            .unwrap().unwrap();
        let secret = TotpSecret::Encoded(encoded);
        let secret_bytes = secret.to_bytes().unwrap();
        let totp = TOTP::new(
            Algorithm::SHA1, 6, 1, 30, secret_bytes,
            Some("RustyClaw".to_string()), "testuser".to_string(),
        ).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let code = totp.generate(now);

        assert!(manager.verify_totp(&code).unwrap());

        // Wrong code should fail.
        assert!(!manager.verify_totp("000000").unwrap());

        // Remove TOTP.
        manager.remove_totp().unwrap();
        assert!(!manager.has_totp());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

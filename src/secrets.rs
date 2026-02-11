use anyhow::{Context, Result};
use securestore::KeySource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use totp_rs::{Algorithm, TOTP, Secret as TotpSecret};

// ── Credential types ────────────────────────────────────────────────────────

/// What kind of secret a credential entry holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    /// Bearer / API token (single opaque string).
    ApiKey,
    /// HTTP passkey (WebAuthn-style credential id + secret).
    HttpPasskey,
    /// Username + password pair.
    UsernamePassword,
    /// SSH keypair (Ed25519).  The private key is stored in the vault;
    /// the public key is also written to `<credentials_dir>/rustyclaw_agent.pub`.
    SshKey,
    /// Generic single-value token (OAuth tokens, bot tokens, etc.).
    Token,
    /// Form autofill data — arbitrary key/value pairs for filling web
    /// forms (name, address, email, phone, etc.).
    FormAutofill,
    /// Payment method — credit/debit card details.
    PaymentMethod,
    /// Free-form encrypted note (recovery codes, license keys,
    /// security questions, PIN codes, etc.).
    SecureNote,
    /// Catch-all for anything that doesn't fit the above.
    Other,
}

/// Controls *when* the agent is allowed to read a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessPolicy {
    /// The agent may read this secret at any time without prompting.
    Always,
    /// The agent may read this secret only with explicit per-use user
    /// approval (e.g. a "yes/no" confirmation in the TUI).
    WithApproval,
    /// The agent must re-authenticate (vault password and/or TOTP)
    /// before each access.
    WithAuth,
    /// The secret is only available when the agent is executing one of
    /// the named skills.  An empty list means "no skill may access it"
    /// (effectively locked).
    SkillOnly(Vec<String>),
}

impl Default for AccessPolicy {
    fn default() -> Self {
        Self::WithApproval
    }
}

/// Metadata envelope stored alongside the secret value(s) in the vault.
///
/// This is JSON-serialized and stored under the key `cred:<name>`.
/// The actual sensitive values live under `val:<name>` (and for
/// `UsernamePassword`, also `val:<name>:user`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    /// Human-readable label (e.g. "Anthropic API key").
    pub label: String,
    /// What kind of credential this is.
    pub kind: SecretKind,
    /// Who (or what) is allowed to read the secret.
    pub policy: AccessPolicy,
    /// Optional free-form description / notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The result of reading a credential — includes the metadata envelope
/// plus the decrypted value(s).
#[derive(Debug, Clone)]
pub enum CredentialValue {
    /// A single opaque string (ApiKey, Token, HttpPasskey, Other).
    Single(String),
    /// Username + password pair.
    UserPass { username: String, password: String },
    /// SSH keypair — private key in OpenSSH PEM format, public key in
    /// `ssh-ed25519 AAAA…` format.
    SshKeyPair { private_key: String, public_key: String },
    /// Arbitrary key/value pairs (form autofill fields).
    FormFields(BTreeMap<String, String>),
    /// Payment card details.
    PaymentCard {
        cardholder: String,
        number: String,
        expiry: String,
        cvv: String,
        /// Optional billing-address / notes fields.
        extra: BTreeMap<String, String>,
    },
}

/// Context supplied by the caller when requesting access to a
/// credential.  The [`SecretsManager`] evaluates this against the
/// credential's [`AccessPolicy`].
#[derive(Debug, Clone, Default)]
pub struct AccessContext {
    /// The user explicitly approved this specific access.
    pub user_approved: bool,
    /// The caller has re-verified the vault password and/or TOTP
    /// within this request.
    pub authenticated: bool,
    /// The name of the skill currently being executed, if any.
    pub active_skill: Option<String>,
}

/// Secrets manager backed by an encrypted SecureStore vault.
///
/// The vault is stored at `{credentials_dir}/secrets.json`.  Encryption uses
/// either a CSPRNG-generated key file (`{credentials_dir}/secrets.key`) or a
/// user-supplied password — never both.
///
/// ## Storage layout
///
/// | Key pattern            | Content                                          |
/// |------------------------|--------------------------------------------------|
/// | `cred:<name>`          | JSON-serialized [`SecretEntry`] metadata          |
/// | `val:<name>`           | Primary secret value (or private key PEM / note)  |
/// | `val:<name>:user`      | Username (for `UsernamePassword` kind)             |
/// | `val:<name>:pub`       | Public key string (for `SshKey` kind)              |
/// | `val:<name>:fields`    | JSON map of form-field key/value pairs             |
/// | `val:<name>:card`      | JSON `{cardholder,number,expiry,cvv}`              |
/// | `val:<name>:card_extra`| JSON map of additional payment card fields         |
/// | `<bare key>`           | Legacy / raw secrets (API keys, TOTP, etc.)        |
pub struct SecretsManager {
    /// Root credentials directory (for writing SSH pubkey file, etc.)
    credentials_dir: PathBuf,
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

/// Kept for backward compatibility with older code that references this type.
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
            credentials_dir: dir.clone(),
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
            credentials_dir: dir.clone(),
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

    // ── Typed credential API ────────────────────────────────────────

    /// Store a typed credential in the vault.
    ///
    /// For `UsernamePassword`, supply the password as `value` and the
    /// username as `username`.  For all other kinds, `username` is
    /// ignored and `value` holds the single secret string.
    ///
    /// For `SshKey`, prefer [`generate_ssh_key`] which creates the
    /// keypair automatically.
    pub fn store_credential(
        &mut self,
        name: &str,
        entry: &SecretEntry,
        value: &str,
        username: Option<&str>,
    ) -> Result<()> {
        let meta_key = format!("cred:{}", name);
        let val_key = format!("val:{}", name);

        let meta_json = serde_json::to_string(entry)
            .context("Failed to serialize credential metadata")?;
        self.store_secret(&meta_key, &meta_json)?;
        self.store_secret(&val_key, value)?;

        if entry.kind == SecretKind::UsernamePassword {
            let user_key = format!("val:{}:user", name);
            self.store_secret(&user_key, username.unwrap_or(""))?;
        }

        Ok(())
    }

    /// Store a form-autofill credential (arbitrary key/value fields).
    ///
    /// `fields` maps field names (e.g. "email", "phone", "address")
    /// to their values.  The `description` on the entry is a good
    /// place to record the site URL or form name.
    pub fn store_form_autofill(
        &mut self,
        name: &str,
        entry: &SecretEntry,
        fields: &BTreeMap<String, String>,
    ) -> Result<()> {
        debug_assert_eq!(entry.kind, SecretKind::FormAutofill);

        let meta_key = format!("cred:{}", name);
        let fields_key = format!("val:{}:fields", name);

        let meta_json = serde_json::to_string(entry)
            .context("Failed to serialize credential metadata")?;
        let fields_json = serde_json::to_string(fields)
            .context("Failed to serialize form fields")?;

        self.store_secret(&meta_key, &meta_json)?;
        self.store_secret(&fields_key, &fields_json)?;
        Ok(())
    }

    /// Store a payment-method credential.
    pub fn store_payment_method(
        &mut self,
        name: &str,
        entry: &SecretEntry,
        cardholder: &str,
        number: &str,
        expiry: &str,
        cvv: &str,
        extra: &BTreeMap<String, String>,
    ) -> Result<()> {
        debug_assert_eq!(entry.kind, SecretKind::PaymentMethod);

        let meta_key = format!("cred:{}", name);
        let card_key = format!("val:{}:card", name);
        let extra_key = format!("val:{}:card_extra", name);

        let meta_json = serde_json::to_string(entry)
            .context("Failed to serialize credential metadata")?;

        #[derive(Serialize)]
        struct Card<'a> {
            cardholder: &'a str,
            number: &'a str,
            expiry: &'a str,
            cvv: &'a str,
        }
        let card_json = serde_json::to_string(&Card {
            cardholder, number, expiry, cvv,
        }).context("Failed to serialize card details")?;

        self.store_secret(&meta_key, &meta_json)?;
        self.store_secret(&card_key, &card_json)?;

        if !extra.is_empty() {
            let extra_json = serde_json::to_string(extra)
                .context("Failed to serialize card extras")?;
            self.store_secret(&extra_key, &extra_json)?;
        }

        Ok(())
    }

    /// Retrieve a typed credential from the vault.
    ///
    /// `context` drives the permission check:
    /// - `user_approved`: the user has explicitly said "yes" for this
    ///   access (satisfies `WithApproval`).
    /// - `authenticated`: the caller has already re-verified the vault
    ///   password / TOTP (satisfies `WithAuth`).
    /// - `active_skill`: if the agent is currently executing a skill,
    ///   pass its name here (satisfies `SkillOnly` when listed).
    pub fn get_credential(
        &mut self,
        name: &str,
        ctx: &AccessContext,
    ) -> Result<Option<(SecretEntry, CredentialValue)>> {
        let meta_key = format!("cred:{}", name);
        let val_key = format!("val:{}", name);

        // Load metadata.
        let meta_json = match self.get_secret(&meta_key, true)? {
            Some(j) => j,
            None => return Ok(None),
        };
        let entry: SecretEntry = serde_json::from_str(&meta_json)
            .context("Corrupted credential metadata")?;

        // ── Policy check ───────────────────────────────────────────
        if !self.check_access(&entry.policy, ctx) {
            anyhow::bail!(
                "Access denied for credential '{}' (policy: {:?})",
                name,
                entry.policy,
            );
        }

        // ── Load value(s) ──────────────────────────────────────────
        let value = match entry.kind {
            SecretKind::UsernamePassword => {
                let password = self.get_secret(&val_key, true)?
                    .unwrap_or_default();
                let user_key = format!("val:{}:user", name);
                let username = self.get_secret(&user_key, true)?
                    .unwrap_or_default();
                CredentialValue::UserPass { username, password }
            }
            SecretKind::SshKey => {
                let private_key = self.get_secret(&val_key, true)?
                    .unwrap_or_default();
                let pub_key = format!("val:{}:pub", name);
                let public_key = self.get_secret(&pub_key, true)?
                    .unwrap_or_default();
                CredentialValue::SshKeyPair { private_key, public_key }
            }
            SecretKind::FormAutofill => {
                let fields_key = format!("val:{}:fields", name);
                let fields_json = self.get_secret(&fields_key, true)?
                    .unwrap_or_else(|| "{}".to_string());
                let fields: BTreeMap<String, String> = serde_json::from_str(&fields_json)
                    .context("Corrupted form-autofill fields")?;
                CredentialValue::FormFields(fields)
            }
            SecretKind::PaymentMethod => {
                let card_key = format!("val:{}:card", name);
                let extra_key = format!("val:{}:card_extra", name);

                let card_json = self.get_secret(&card_key, true)?
                    .unwrap_or_else(|| "{}".to_string());

                #[derive(Deserialize)]
                struct Card {
                    #[serde(default)] cardholder: String,
                    #[serde(default)] number: String,
                    #[serde(default)] expiry: String,
                    #[serde(default)] cvv: String,
                }
                let card: Card = serde_json::from_str(&card_json)
                    .context("Corrupted payment card data")?;

                let extra: BTreeMap<String, String> = match self.get_secret(&extra_key, true)? {
                    Some(j) => serde_json::from_str(&j)
                        .context("Corrupted card extras")?,
                    None => BTreeMap::new(),
                };

                CredentialValue::PaymentCard {
                    cardholder: card.cardholder,
                    number: card.number,
                    expiry: card.expiry,
                    cvv: card.cvv,
                    extra,
                }
            }
            _ => {
                let v = self.get_secret(&val_key, true)?
                    .unwrap_or_default();
                CredentialValue::Single(v)
            }
        };

        Ok(Some((entry, value)))
    }

    /// List all typed credential names (not raw / legacy keys).
    pub fn list_credentials(&mut self) -> Vec<(String, SecretEntry)> {
        let keys = self.list_secrets();
        let mut result = Vec::new();
        for key in &keys {
            if let Some(name) = key.strip_prefix("cred:") {
                if let Ok(Some(json)) = self.get_secret(key, true) {
                    if let Ok(entry) = serde_json::from_str::<SecretEntry>(&json) {
                        result.push((name.to_string(), entry));
                    }
                }
            }
        }
        result
    }

    /// Delete a typed credential and all its associated vault keys.
    pub fn delete_credential(&mut self, name: &str) -> Result<()> {
        // Every possible sub-key pattern — best-effort removal.
        let sub_keys = [
            format!("cred:{}", name),
            format!("val:{}", name),
            format!("val:{}:user", name),
            format!("val:{}:pub", name),
            format!("val:{}:fields", name),
            format!("val:{}:card", name),
            format!("val:{}:card_extra", name),
        ];
        for key in &sub_keys {
            let _ = self.delete_secret(key);
        }

        // Also remove the pubkey file if it exists.
        let pubkey_path = self.credentials_dir.join(format!("{}.pub", name));
        let _ = std::fs::remove_file(pubkey_path);

        Ok(())
    }

    // ── SSH key generation ──────────────────────────────────────────

    /// Generate a new Ed25519 SSH keypair, store it in the vault as
    /// an `SshKey` credential, and write the public key to
    /// `<credentials_dir>/<name>.pub`.
    ///
    /// Returns the public key string (`ssh-ed25519 AAAA… <comment>`).
    pub fn generate_ssh_key(
        &mut self,
        name: &str,
        comment: &str,
        policy: AccessPolicy,
    ) -> Result<String> {
        use ssh_key::private::PrivateKey;

        // Generate keypair.
        let private = PrivateKey::random(
            &mut ssh_key::rand_core::OsRng,
            ssh_key::Algorithm::Ed25519,
        )
        .map_err(|e| anyhow::anyhow!("Failed to generate SSH key: {}", e))?;

        let private_pem = private
            .to_openssh(ssh_key::LineEnding::LF)
            .map_err(|e| anyhow::anyhow!("Failed to encode private key: {}", e))?;

        let public = private.public_key();
        let public_openssh = public.to_openssh()
            .map_err(|e| anyhow::anyhow!("Failed to encode public key: {}", e))?;

        let public_str = if comment.is_empty() {
            public_openssh.to_string()
        } else {
            format!("{} {}", public_openssh, comment)
        };

        // Store in vault.
        let entry = SecretEntry {
            label: format!("SSH key ({})", name),
            kind: SecretKind::SshKey,
            policy,
            description: Some(format!("Ed25519 keypair — {}", comment)),
        };

        let meta_key = format!("cred:{}", name);
        let val_key = format!("val:{}", name);
        let pub_vault_key = format!("val:{}:pub", name);

        let meta_json = serde_json::to_string(&entry)
            .context("Failed to serialize credential metadata")?;
        self.store_secret(&meta_key, &meta_json)?;
        self.store_secret(&val_key, private_pem.to_string().as_str())?;
        self.store_secret(&pub_vault_key, &public_str)?;

        // Write public key file.
        let pubkey_path = self.credentials_dir.join(format!("{}.pub", name));
        std::fs::write(&pubkey_path, &public_str)
            .context("Failed to write SSH public key file")?;

        Ok(public_str)
    }

    /// Path to the SSH public key file for a given credential name.
    pub fn ssh_pubkey_path(&self, name: &str) -> PathBuf {
        self.credentials_dir.join(format!("{}.pub", name))
    }

    // ── Access policy enforcement ───────────────────────────────────

    /// Evaluate whether the given [`AccessContext`] satisfies a
    /// credential's [`AccessPolicy`].
    fn check_access(&self, policy: &AccessPolicy, ctx: &AccessContext) -> bool {
        match policy {
            AccessPolicy::Always => true,
            AccessPolicy::WithApproval => {
                ctx.user_approved || self.agent_access_enabled
            }
            AccessPolicy::WithAuth => ctx.authenticated,
            AccessPolicy::SkillOnly(allowed) => {
                if let Some(ref skill) = ctx.active_skill {
                    allowed.iter().any(|s| s == skill)
                } else {
                    false
                }
            }
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

    // ── Typed credential tests ──────────────────────────────────────

    #[test]
    fn test_store_and_retrieve_api_key() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "Anthropic".to_string(),
            kind: SecretKind::ApiKey,
            policy: AccessPolicy::WithApproval,
            description: None,
        };
        m.store_credential("anthropic_key", &entry, "sk-ant-12345", None).unwrap();

        let ctx = AccessContext { user_approved: true, ..Default::default() };
        let (meta, val) = m.get_credential("anthropic_key", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::ApiKey);
        assert_eq!(meta.label, "Anthropic");
        match val {
            CredentialValue::Single(v) => assert_eq!(v, "sk-ant-12345"),
            _ => panic!("Expected Single"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_store_and_retrieve_username_password() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "Registry".to_string(),
            kind: SecretKind::UsernamePassword,
            policy: AccessPolicy::Always,
            description: None,
        };
        m.store_credential("registry", &entry, "s3cret", Some("admin")).unwrap();

        let ctx = AccessContext::default();
        let (_, val) = m.get_credential("registry", &ctx).unwrap().unwrap();
        match val {
            CredentialValue::UserPass { username, password } => {
                assert_eq!(username, "admin");
                assert_eq!(password, "s3cret");
            }
            _ => panic!("Expected UserPass"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_store_http_passkey() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "WebAuthn passkey".to_string(),
            kind: SecretKind::HttpPasskey,
            policy: AccessPolicy::WithAuth,
            description: Some("FIDO2 credential".to_string()),
        };
        m.store_credential("passkey1", &entry, "cred-id-base64", None).unwrap();

        // Access without authentication should be denied.
        let ctx = AccessContext { user_approved: true, ..Default::default() };
        assert!(m.get_credential("passkey1", &ctx).is_err());

        // Access with authentication should succeed.
        let ctx = AccessContext { authenticated: true, ..Default::default() };
        let (meta, val) = m.get_credential("passkey1", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::HttpPasskey);
        match val {
            CredentialValue::Single(v) => assert_eq!(v, "cred-id-base64"),
            _ => panic!("Expected Single"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_ssh_key() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let pubkey = m.generate_ssh_key(
            "rustyclaw_agent", "rustyclaw@agent", AccessPolicy::WithApproval,
        ).unwrap();

        assert!(pubkey.starts_with("ssh-ed25519 "));
        assert!(pubkey.contains("rustyclaw@agent"));

        // Public key file should exist on disk.
        let pubkey_path = dir.join("rustyclaw_agent.pub");
        assert!(pubkey_path.exists());
        let on_disk = std::fs::read_to_string(&pubkey_path).unwrap();
        assert_eq!(on_disk, pubkey);

        // Retrieve via typed API.
        let ctx = AccessContext { user_approved: true, ..Default::default() };
        let (meta, val) = m.get_credential("rustyclaw_agent", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::SshKey);
        match val {
            CredentialValue::SshKeyPair { private_key, public_key } => {
                assert!(private_key.contains("BEGIN OPENSSH PRIVATE KEY"));
                assert!(public_key.starts_with("ssh-ed25519 "));
            }
            _ => panic!("Expected SshKeyPair"),
        }

        // Delete should clean up the pubkey file too.
        m.delete_credential("rustyclaw_agent").unwrap();
        assert!(!pubkey_path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_list_credentials() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let e1 = SecretEntry {
            label: "Key A".to_string(),
            kind: SecretKind::ApiKey,
            policy: AccessPolicy::Always,
            description: None,
        };
        let e2 = SecretEntry {
            label: "Key B".to_string(),
            kind: SecretKind::Token,
            policy: AccessPolicy::WithApproval,
            description: None,
        };
        m.store_credential("a", &e1, "val_a", None).unwrap();
        m.store_credential("b", &e2, "val_b", None).unwrap();

        // Also store a raw legacy secret — should NOT appear in list_credentials.
        m.store_secret("legacy_key", "legacy_val").unwrap();

        let creds = m.list_credentials();
        let names: Vec<&str> = creds.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        assert!(!names.contains(&"legacy_key"));
        assert_eq!(creds.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Access policy tests ─────────────────────────────────────────

    #[test]
    fn test_policy_always() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);
        let entry = SecretEntry {
            label: "open".to_string(),
            kind: SecretKind::Token,
            policy: AccessPolicy::Always,
            description: None,
        };
        m.store_credential("open_tok", &entry, "val", None).unwrap();

        // Should succeed with an empty context.
        let ctx = AccessContext::default();
        assert!(m.get_credential("open_tok", &ctx).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_policy_with_approval_denied() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);
        let entry = SecretEntry {
            label: "guarded".to_string(),
            kind: SecretKind::ApiKey,
            policy: AccessPolicy::WithApproval,
            description: None,
        };
        m.store_credential("guarded", &entry, "val", None).unwrap();

        // No approval, no agent_access → denied.
        let ctx = AccessContext::default();
        assert!(m.get_credential("guarded", &ctx).is_err());

        // With approval → ok.
        let ctx = AccessContext { user_approved: true, ..Default::default() };
        assert!(m.get_credential("guarded", &ctx).unwrap().is_some());

        // With agent_access enabled → also ok.
        m.set_agent_access(true);
        let ctx = AccessContext::default();
        assert!(m.get_credential("guarded", &ctx).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_policy_with_auth() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);
        let entry = SecretEntry {
            label: "high-sec".to_string(),
            kind: SecretKind::ApiKey,
            policy: AccessPolicy::WithAuth,
            description: None,
        };
        m.store_credential("hs", &entry, "val", None).unwrap();

        // Even with user_approved, needs authenticated.
        let ctx = AccessContext { user_approved: true, ..Default::default() };
        assert!(m.get_credential("hs", &ctx).is_err());

        let ctx = AccessContext { authenticated: true, ..Default::default() };
        assert!(m.get_credential("hs", &ctx).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_policy_skill_only() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);
        let entry = SecretEntry {
            label: "deploy-key".to_string(),
            kind: SecretKind::Token,
            policy: AccessPolicy::SkillOnly(vec!["deploy".to_string(), "ci".to_string()]),
            description: None,
        };
        m.store_credential("dk", &entry, "val", None).unwrap();

        // No skill → denied.
        let ctx = AccessContext { user_approved: true, ..Default::default() };
        assert!(m.get_credential("dk", &ctx).is_err());

        // Wrong skill → denied.
        let ctx = AccessContext {
            active_skill: Some("build".to_string()),
            ..Default::default()
        };
        assert!(m.get_credential("dk", &ctx).is_err());

        // Correct skill → ok.
        let ctx = AccessContext {
            active_skill: Some("deploy".to_string()),
            ..Default::default()
        };
        assert!(m.get_credential("dk", &ctx).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_credential() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);
        let entry = SecretEntry {
            label: "tmp".to_string(),
            kind: SecretKind::Token,
            policy: AccessPolicy::Always,
            description: None,
        };
        m.store_credential("tmp", &entry, "val", None).unwrap();
        assert_eq!(m.list_credentials().len(), 1);

        m.delete_credential("tmp").unwrap();
        assert_eq!(m.list_credentials().len(), 0);

        // get_credential should return None now.
        let ctx = AccessContext::default();
        assert!(m.get_credential("tmp", &ctx).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Web-navigation credential tests ─────────────────────────────

    #[test]
    fn test_store_and_retrieve_form_autofill() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "Shipping address".to_string(),
            kind: SecretKind::FormAutofill,
            policy: AccessPolicy::WithApproval,
            description: Some("https://example.com/checkout".to_string()),
        };
        let mut fields = BTreeMap::new();
        fields.insert("name".to_string(), "Ada Lovelace".to_string());
        fields.insert("email".to_string(), "ada@example.com".to_string());
        fields.insert("phone".to_string(), "+1-555-0100".to_string());
        fields.insert("address".to_string(), "1 Infinite Loop".to_string());

        m.store_form_autofill("shipping", &entry, &fields).unwrap();

        let ctx = AccessContext { user_approved: true, ..Default::default() };
        let (meta, val) = m.get_credential("shipping", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::FormAutofill);
        assert_eq!(meta.label, "Shipping address");
        match val {
            CredentialValue::FormFields(f) => {
                assert_eq!(f.len(), 4);
                assert_eq!(f["name"], "Ada Lovelace");
                assert_eq!(f["email"], "ada@example.com");
            }
            _ => panic!("Expected FormFields"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_store_and_retrieve_payment_method() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "Visa ending 4242".to_string(),
            kind: SecretKind::PaymentMethod,
            policy: AccessPolicy::WithAuth,
            description: None,
        };
        let mut extra = BTreeMap::new();
        extra.insert("billing_zip".to_string(), "94025".to_string());

        m.store_payment_method(
            "visa_4242", &entry,
            "A. Lovelace", "4242424242424242", "12/28", "123",
            &extra,
        ).unwrap();

        // Needs authentication.
        let ctx = AccessContext { user_approved: true, ..Default::default() };
        assert!(m.get_credential("visa_4242", &ctx).is_err());

        let ctx = AccessContext { authenticated: true, ..Default::default() };
        let (meta, val) = m.get_credential("visa_4242", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::PaymentMethod);
        match val {
            CredentialValue::PaymentCard { cardholder, number, expiry, cvv, extra } => {
                assert_eq!(cardholder, "A. Lovelace");
                assert_eq!(number, "4242424242424242");
                assert_eq!(expiry, "12/28");
                assert_eq!(cvv, "123");
                assert_eq!(extra["billing_zip"], "94025");
            }
            _ => panic!("Expected PaymentCard"),
        }

        // Delete should clean everything up.
        m.delete_credential("visa_4242").unwrap();
        assert_eq!(m.list_credentials().len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_store_and_retrieve_secure_note() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "Recovery codes".to_string(),
            kind: SecretKind::SecureNote,
            policy: AccessPolicy::WithAuth,
            description: Some("GitHub 2FA backup codes".to_string()),
        };
        let note = "abcde-12345\nfghij-67890\nklmno-13579";
        m.store_credential("gh_recovery", &entry, note, None).unwrap();

        let ctx = AccessContext { authenticated: true, ..Default::default() };
        let (meta, val) = m.get_credential("gh_recovery", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::SecureNote);
        assert_eq!(meta.description, Some("GitHub 2FA backup codes".to_string()));
        match val {
            CredentialValue::Single(v) => assert_eq!(v, note),
            _ => panic!("Expected Single"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_form_autofill_delete_cleans_fields() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "Login form".to_string(),
            kind: SecretKind::FormAutofill,
            policy: AccessPolicy::Always,
            description: None,
        };
        let mut fields = BTreeMap::new();
        fields.insert("user".to_string(), "alice".to_string());
        m.store_form_autofill("login", &entry, &fields).unwrap();
        assert_eq!(m.list_credentials().len(), 1);

        m.delete_credential("login").unwrap();
        assert_eq!(m.list_credentials().len(), 0);

        // The :fields sub-key should also be gone.
        m.set_agent_access(true);
        let raw = m.get_secret("val:login:fields", false).unwrap();
        assert_eq!(raw, None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

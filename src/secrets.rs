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
    /// SSH keypair (Ed25519).  Both keys are stored encrypted in the vault.
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

impl std::fmt::Display for SecretKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey => write!(f, "API Key"),
            Self::HttpPasskey => write!(f, "HTTP Passkey"),
            Self::UsernamePassword => write!(f, "Login"),
            Self::SshKey => write!(f, "SSH Key"),
            Self::Token => write!(f, "Token"),
            Self::FormAutofill => write!(f, "Form"),
            Self::PaymentMethod => write!(f, "Payment"),
            Self::SecureNote => write!(f, "Note"),
            Self::Other => write!(f, "Other"),
        }
    }
}

impl SecretKind {
    /// A single-character icon suitable for the TUI list.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::ApiKey => "🔑",
            Self::HttpPasskey => "🌐",
            Self::UsernamePassword => "👤",
            Self::SshKey => "🔐",
            Self::Token => "🎫",
            Self::FormAutofill => "📋",
            Self::PaymentMethod => "💳",
            Self::SecureNote => "📝",
            Self::Other => "🔒",
        }
    }
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

impl std::fmt::Display for AccessPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Always => write!(f, "always"),
            Self::WithApproval => write!(f, "approval"),
            Self::WithAuth => write!(f, "auth"),
            Self::SkillOnly(skills) => {
                if skills.is_empty() {
                    write!(f, "locked")
                } else {
                    write!(f, "skills: {}", skills.join(", "))
                }
            }
        }
    }
}

impl AccessPolicy {
    /// Short badge-style label for the TUI.
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Always => "OPEN",
            Self::WithApproval => "ASK",
            Self::WithAuth => "AUTH",
            Self::SkillOnly(_) => "SKILL",
        }
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
    /// When true, the credential is listed but the agent cannot read
    /// its value.  The user can re-enable it from the TUI.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
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

    /// Re-encrypt an existing vault with a new password.
    ///
    /// Loads the vault with the current key source, reads every secret,
    /// creates a brand-new vault encrypted with `new_password`, writes
    /// back all the secrets, and saves.  On success the in-memory state
    /// is updated to use the new password.
    pub fn change_password(&mut self, new_password: String) -> Result<()> {
        // 1. Make sure the vault is loaded with the *current* credentials.
        let old_vault = self.ensure_vault()?;

        // 2. Read out every key → value pair.
        let keys: Vec<String> = old_vault.keys().map(|s| s.to_string()).collect();
        let mut entries: Vec<(String, String)> = Vec::new();
        for key in &keys {
            match old_vault.get(key) {
                Ok(value) => entries.push((key.clone(), value)),
                // Skip entries we can't decrypt (shouldn't happen, but be safe).
                Err(_) => {}
            }
        }

        // 3. Drop the old vault and create a new one with the new password.
        self.vault = None;

        let new_vault = securestore::SecretsManager::new(
            KeySource::Password(&new_password),
        )
        .context("Failed to create vault with new password")?;
        new_vault
            .save_as(&self.vault_path)
            .context("Failed to save re-encrypted vault")?;

        // 4. Reload so we can write to it.
        let mut reloaded = securestore::SecretsManager::load(
            &self.vault_path,
            KeySource::Password(&new_password),
        )
        .context("Failed to reload vault with new password")?;

        // 5. Write all secrets back.
        for (key, value) in entries {
            reloaded.set(&key, value);
        }
        reloaded.save().context("Failed to save re-keyed vault")?;

        // 6. Update in-memory state.
        self.password = Some(new_password);
        self.vault = Some(reloaded);

        // 7. Remove the old key file if it exists — no longer needed.
        if self.key_path.exists() {
            let _ = std::fs::remove_file(&self.key_path);
        }

        Ok(())
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

        // ── Disabled check ─────────────────────────────────────────
        if entry.disabled {
            anyhow::bail!(
                "Credential '{}' is disabled",
                name,
            );
        }

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

    /// List *all* credentials — both typed (`cred:*`) and legacy bare-key
    /// secrets (e.g. `ANTHROPIC_API_KEY`).
    ///
    /// Legacy keys that match a known provider secret name get a
    /// synthesised [`SecretEntry`] with `kind = ApiKey` or `Token`.
    /// Internal keys (TOTP secret, `__init`, `cred:*`, `val:*`) are
    /// excluded.
    pub fn list_all_entries(&mut self) -> Vec<(String, SecretEntry)> {
        let all_keys = self.list_secrets();

        let mut result = Vec::new();
        let mut typed_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 1. Typed credentials (cred:* prefix)
        for key in &all_keys {
            if let Some(name) = key.strip_prefix("cred:") {
                if let Ok(Some(json)) = self.get_secret(key, true) {
                    if let Ok(entry) = serde_json::from_str::<SecretEntry>(&json) {
                        typed_names.insert(name.to_string());
                        result.push((name.to_string(), entry));
                    }
                }
            }
        }

        // 2. Legacy / bare keys — skip internal bookkeeping keys.
        for key in &all_keys {
            // Skip typed credential sub-keys and internal keys.
            if key.starts_with("cred:")
                || key.starts_with("val:")
                || key == Self::TOTP_SECRET_KEY
                || key == "__init"
            {
                continue;
            }
            // Skip if already covered by a typed credential.
            if typed_names.contains(key.as_str()) {
                continue;
            }

            // Try to match against a known provider secret key.
            let (label, kind) = Self::label_for_legacy_key(key);
            result.push((key.clone(), SecretEntry {
                label,
                kind,
                policy: AccessPolicy::WithApproval,
                description: None,
                disabled: false,
            }));
        }

        result
    }

    /// Produce a human-readable label and [`SecretKind`] for a legacy
    /// bare vault key.
    fn label_for_legacy_key(key: &str) -> (String, SecretKind) {
        use crate::providers::PROVIDERS;
        // Check known providers first.
        for p in PROVIDERS {
            if p.secret_key == Some(key) {
                let kind = match p.auth_method {
                    crate::providers::AuthMethod::DeviceFlow => SecretKind::Token,
                    _ => SecretKind::ApiKey,
                };
                return (format!("{}", p.display), kind);
            }
        }
        // Fallback: humanise the key name.
        let label = key
            .replace('_', " ")
            .to_lowercase()
            .split(' ')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(first) => {
                        let upper: String = first.to_uppercase().collect();
                        format!("{}{}", upper, c.as_str())
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        (label, SecretKind::Other)
    }

    /// Retrieve a credential's value(s) as displayable `(label, value)` pairs
    /// for the TUI secret viewer.
    ///
    /// This bypasses the disabled check and the access-policy check because
    /// the *user* is physically present and explicitly asked to view the
    /// secret.  For legacy bare-key secrets (no `cred:` metadata) the raw
    /// value is returned directly.
    pub fn peek_credential_display(&mut self, name: &str) -> Result<Vec<(String, String)>> {
        let meta_key = format!("cred:{}", name);
        let val_key = format!("val:{}", name);

        // Check if this is a typed credential.
        if let Some(json) = self.get_secret(&meta_key, true)? {
            let entry: SecretEntry = serde_json::from_str(&json)
                .context("Corrupted credential metadata")?;

            let pairs = match entry.kind {
                SecretKind::UsernamePassword => {
                    let password = self.get_secret(&val_key, true)?.unwrap_or_default();
                    let user_key = format!("val:{}:user", name);
                    let username = self.get_secret(&user_key, true)?.unwrap_or_default();
                    vec![
                        ("Username".to_string(), username),
                        ("Password".to_string(), password),
                    ]
                }
                SecretKind::SshKey => {
                    let private_key = self.get_secret(&val_key, true)?.unwrap_or_default();
                    let pub_key = format!("val:{}:pub", name);
                    let public_key = self.get_secret(&pub_key, true)?.unwrap_or_default();
                    vec![
                        ("Public Key".to_string(), public_key),
                        ("Private Key".to_string(), private_key),
                    ]
                }
                SecretKind::FormAutofill => {
                    let fields_key = format!("val:{}:fields", name);
                    let fields_json = self.get_secret(&fields_key, true)?
                        .unwrap_or_else(|| "{}".to_string());
                    let fields: BTreeMap<String, String> = serde_json::from_str(&fields_json)
                        .unwrap_or_default();
                    fields.into_iter().collect()
                }
                SecretKind::PaymentMethod => {
                    let card_key = format!("val:{}:card", name);
                    let card_json = self.get_secret(&card_key, true)?
                        .unwrap_or_else(|| "{}".to_string());

                    #[derive(Deserialize)]
                    struct Card {
                        #[serde(default)] cardholder: String,
                        #[serde(default)] number: String,
                        #[serde(default)] expiry: String,
                        #[serde(default)] cvv: String,
                    }
                    let card: Card = serde_json::from_str(&card_json).unwrap_or(Card {
                        cardholder: String::new(),
                        number: String::new(),
                        expiry: String::new(),
                        cvv: String::new(),
                    });

                    let mut pairs = vec![
                        ("Cardholder".to_string(), card.cardholder),
                        ("Number".to_string(), card.number),
                        ("Expiry".to_string(), card.expiry),
                        ("CVV".to_string(), card.cvv),
                    ];

                    let extra_key = format!("val:{}:card_extra", name);
                    if let Some(j) = self.get_secret(&extra_key, true)? {
                        let extra: BTreeMap<String, String> =
                            serde_json::from_str(&j).unwrap_or_default();
                        for (k, v) in extra {
                            pairs.push((k, v));
                        }
                    }
                    pairs
                }
                _ => {
                    let v = self.get_secret(&val_key, true)?.unwrap_or_default();
                    vec![("Value".to_string(), v)]
                }
            };
            return Ok(pairs);
        }

        // Legacy bare-key secret — return the raw value.
        match self.get_secret(name, true)? {
            Some(v) => Ok(vec![("Value".to_string(), v)]),
            None => anyhow::bail!("Secret '{}' not found", name),
        }
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

        Ok(())
    }

    /// Enable or disable a credential.
    ///
    /// For typed credentials (`cred:<name>` exists) the `disabled`
    /// flag is updated in the metadata envelope.  For legacy bare-key
    /// secrets a typed envelope is created in-place so the flag can
    /// be persisted.
    pub fn set_credential_disabled(&mut self, name: &str, disabled: bool) -> Result<()> {
        let meta_key = format!("cred:{}", name);

        let mut entry: SecretEntry = match self.get_secret(&meta_key, true)? {
            Some(json) => serde_json::from_str(&json)
                .context("Corrupted credential metadata")?,
            None => {
                // Legacy bare key — promote to typed entry.
                let (label, kind) = Self::label_for_legacy_key(name);
                SecretEntry {
                    label,
                    kind,
                    policy: AccessPolicy::WithApproval,
                    description: None,
                    disabled: false,
                }
            }
        };

        entry.disabled = disabled;

        let meta_json = serde_json::to_string(&entry)
            .context("Failed to serialize credential metadata")?;
        self.store_secret(&meta_key, &meta_json)?;
        Ok(())
    }

    /// Change the access policy of a credential.
    pub fn set_credential_policy(&mut self, name: &str, policy: AccessPolicy) -> Result<()> {
        let meta_key = format!("cred:{}", name);

        let mut entry: SecretEntry = match self.get_secret(&meta_key, true)? {
            Some(json) => serde_json::from_str(&json)
                .context("Corrupted credential metadata")?,
            None => {
                // Legacy bare key — promote to typed entry.
                let (label, kind) = Self::label_for_legacy_key(name);
                SecretEntry {
                    label,
                    kind,
                    policy: AccessPolicy::WithApproval,
                    description: None,
                    disabled: false,
                }
            }
        };

        entry.policy = policy;

        let meta_json = serde_json::to_string(&entry)
            .context("Failed to serialize credential metadata")?;
        self.store_secret(&meta_key, &meta_json)?;
        Ok(())
    }

    // ── SSH key generation ──────────────────────────────────────────

    /// Generate a new Ed25519 SSH keypair and store it in the vault
    /// as an `SshKey` credential.
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
            disabled: false,
        };

        let meta_key = format!("cred:{}", name);
        let val_key = format!("val:{}", name);
        let pub_vault_key = format!("val:{}:pub", name);

        let meta_json = serde_json::to_string(&entry)
            .context("Failed to serialize credential metadata")?;
        self.store_secret(&meta_key, &meta_json)?;
        self.store_secret(&val_key, private_pem.to_string().as_str())?;
        self.store_secret(&pub_vault_key, &public_str)?;

        Ok(public_str)
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
        self.setup_totp_with_issuer(account_name, "RustyClaw")
    }

    /// Like [`setup_totp`](Self::setup_totp) but with a custom issuer name
    /// (shown as the app/service label in authenticator apps).
    pub fn setup_totp_with_issuer(&mut self, account_name: &str, issuer: &str) -> Result<String> {
        let secret = TotpSecret::generate_secret();
        let secret_bytes = secret.to_bytes()
            .map_err(|e| anyhow::anyhow!("Failed to generate TOTP secret bytes: {:?}", e))?;

        let totp = TOTP::new(
            Algorithm::SHA1,
            6,  // digits
            1,  // skew (allow ±1 step)
            30, // step (seconds)
            secret_bytes,
            Some(issuer.to_string()),
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
    fn test_change_password() {
        let dir = temp_dir();

        // Create a key-file vault and store some secrets.
        {
            let mut m = SecretsManager::new(&dir);
            m.store_secret("api_key", "sk-abc").unwrap();
            m.store_secret("token", "tok-xyz").unwrap();
        }
        assert!(dir.join("secrets.json").exists());
        assert!(dir.join("secrets.key").exists());

        // Re-open with key-file and change to a password.
        {
            let mut m = SecretsManager::new(&dir);
            m.change_password("newpass".to_string()).unwrap();
        }

        // Key file should be removed after password migration.
        assert!(!dir.join("secrets.key").exists());

        // Reload with the new password — secrets should still be there.
        {
            let mut m = SecretsManager::with_password(&dir, "newpass".to_string());
            m.set_agent_access(true);
            assert_eq!(m.get_secret("api_key", false).unwrap(), Some("sk-abc".to_string()));
            assert_eq!(m.get_secret("token", false).unwrap(), Some("tok-xyz".to_string()));
        }

        // Old key file should no longer work (it's deleted).
        // Wrong password should fail.
        {
            let mut m = SecretsManager::with_password(&dir, "wrong".to_string());
            assert!(m.get_secret("api_key", true).is_err());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_change_password_between_passwords() {
        let dir = temp_dir();

        // Create a password-protected vault.
        {
            let mut m = SecretsManager::with_password(&dir, "old_pw".to_string());
            m.store_secret("secret", "value123").unwrap();
        }

        // Change the password.
        {
            let mut m = SecretsManager::with_password(&dir, "old_pw".to_string());
            m.change_password("new_pw".to_string()).unwrap();
        }

        // New password should work.
        {
            let mut m = SecretsManager::with_password(&dir, "new_pw".to_string());
            m.set_agent_access(true);
            assert_eq!(m.get_secret("secret", false).unwrap(), Some("value123".to_string()));
        }

        // Old password should fail.
        {
            let mut m = SecretsManager::with_password(&dir, "old_pw".to_string());
            assert!(m.get_secret("secret", true).is_err());
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
            disabled: false,
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
            disabled: false,
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
            disabled: false,
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

        // Delete should clean up vault entries.
        m.delete_credential("rustyclaw_agent").unwrap();

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
            disabled: false,
        };
        let e2 = SecretEntry {
            label: "Key B".to_string(),
            kind: SecretKind::Token,
            policy: AccessPolicy::WithApproval,
            description: None,
            disabled: false,
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

    #[test]
    fn test_list_all_entries_includes_legacy_keys() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        // Store a typed credential.
        let entry = SecretEntry {
            label: "Typed".to_string(),
            kind: SecretKind::ApiKey,
            policy: AccessPolicy::Always,
            description: None,
            disabled: false,
        };
        m.store_credential("typed_one", &entry, "val", None).unwrap();

        // Store legacy bare-key secrets (one known provider, one unknown).
        m.store_secret("ANTHROPIC_API_KEY", "sk-ant-xxx").unwrap();
        m.store_secret("MY_CUSTOM_SECRET", "custom-val").unwrap();

        // Store internal keys that should NOT appear.
        m.store_secret("__init", "").unwrap();

        let all = m.list_all_entries();
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();

        // Typed credential appears.
        assert!(names.contains(&"typed_one"));
        // Known provider legacy key appears with correct label.
        assert!(names.contains(&"ANTHROPIC_API_KEY"));
        let anth = all.iter().find(|(n, _)| n == "ANTHROPIC_API_KEY").unwrap();
        assert_eq!(anth.1.kind, SecretKind::ApiKey);
        assert!(anth.1.label.contains("Anthropic"));

        // Unknown legacy key appears with humanised label.
        assert!(names.contains(&"MY_CUSTOM_SECRET"));
        let custom = all.iter().find(|(n, _)| n == "MY_CUSTOM_SECRET").unwrap();
        assert_eq!(custom.1.kind, SecretKind::Other);

        // Internal keys excluded.
        assert!(!names.contains(&"__init"));

        // Sub-keys (cred:*, val:*) excluded.
        assert!(!names.iter().any(|n| n.starts_with("cred:") || n.starts_with("val:")));

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
            disabled: false,
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
            disabled: false,
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
            disabled: false,
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
            disabled: false,
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
            disabled: false,
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
            disabled: false,
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
            disabled: false,
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
            disabled: false,
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
            disabled: false,
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

    #[test]
    fn test_disable_and_reenable_credential() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "my key".to_string(),
            kind: SecretKind::ApiKey,
            policy: AccessPolicy::Always,
            description: None,
            disabled: false,
        };
        m.store_credential("k", &entry, "secret", None).unwrap();

        // Initially accessible.
        let ctx = AccessContext::default();
        assert!(m.get_credential("k", &ctx).unwrap().is_some());

        // Disable it — access should fail.
        m.set_credential_disabled("k", true).unwrap();
        assert!(m.get_credential("k", &ctx).is_err());

        // Still listed.
        let creds = m.list_credentials();
        assert_eq!(creds.len(), 1);
        assert!(creds[0].1.disabled);

        // Re-enable — access should work again.
        m.set_credential_disabled("k", false).unwrap();
        assert!(m.get_credential("k", &ctx).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_disable_legacy_key_promotes_to_typed() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        // Store a bare-key secret (no cred: metadata).
        m.store_secret("MY_BARE_KEY", "bare_val").unwrap();

        // Disabling it should create a cred: entry.
        m.set_credential_disabled("MY_BARE_KEY", true).unwrap();

        let all = m.list_all_entries();
        let bare = all.iter().find(|(n, _)| n == "MY_BARE_KEY").unwrap();
        assert!(bare.1.disabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_set_credential_policy() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        let entry = SecretEntry {
            label: "my key".to_string(),
            kind: SecretKind::ApiKey,
            policy: AccessPolicy::WithApproval,
            description: None,
            disabled: false,
        };
        m.store_credential("k", &entry, "secret", None).unwrap();

        // Default policy is ASK (WithApproval).
        let creds = m.list_credentials();
        assert_eq!(creds[0].1.policy, AccessPolicy::WithApproval);

        // Change to OPEN.
        m.set_credential_policy("k", AccessPolicy::Always).unwrap();
        let creds = m.list_credentials();
        assert_eq!(creds[0].1.policy, AccessPolicy::Always);

        // Change to AUTH.
        m.set_credential_policy("k", AccessPolicy::WithAuth).unwrap();
        let creds = m.list_credentials();
        assert_eq!(creds[0].1.policy, AccessPolicy::WithAuth);

        // Change to SKILL.
        m.set_credential_policy("k", AccessPolicy::SkillOnly(vec!["web".to_string()])).unwrap();
        let creds = m.list_credentials();
        assert_eq!(creds[0].1.policy, AccessPolicy::SkillOnly(vec!["web".to_string()]));

        // Change back to ASK.
        m.set_credential_policy("k", AccessPolicy::WithApproval).unwrap();
        let creds = m.list_credentials();
        assert_eq!(creds[0].1.policy, AccessPolicy::WithApproval);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_set_policy_legacy_key_promotes_to_typed() {
        let dir = temp_dir();
        let mut m = SecretsManager::new(&dir);

        // Store a bare-key secret (no cred: metadata).
        m.store_secret("LEGACY_KEY", "legacy_val").unwrap();

        // Setting policy should create a cred: entry.
        m.set_credential_policy("LEGACY_KEY", AccessPolicy::Always).unwrap();

        let all = m.list_all_entries();
        let entry = all.iter().find(|(n, _)| n == "LEGACY_KEY").unwrap();
        assert_eq!(entry.1.policy, AccessPolicy::Always);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

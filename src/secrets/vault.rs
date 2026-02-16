//! Encryption/decryption logic, file I/O for the vault, and TOTP functionality.

use anyhow::{Context, Result};
use securestore::KeySource;
use totp_rs::{Algorithm, Secret as TotpSecret, TOTP};
use zeroize::Zeroize;

use crate::secret::{ExposeSecret, SecretString};
use super::types::{
    AccessContext, AccessPolicy, CredentialValue, SecretEntry, SecretKind,
};
use super::SecretsManager;

impl SecretsManager {
    /// Ensure the vault is loaded (or created if it doesn't exist yet).
    pub(super) fn ensure_vault(&mut self) -> Result<&mut securestore::SecretsManager> {
        if self.vault.is_none() {
            let vault = if self.vault_path.exists() {
                // Existing vault — load with password or key file.
                if let Some(ref pw) = self.password {
                    securestore::SecretsManager::load(
                        &self.vault_path,
                        KeySource::Password(pw.expose_secret()),
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
                    let sman = securestore::SecretsManager::new(KeySource::Password(pw.expose_secret()))
                        .context("Failed to create new secrets vault")?;
                    sman.save_as(&self.vault_path)
                        .context("Failed to save new secrets vault")?;
                    securestore::SecretsManager::load(
                        &self.vault_path,
                        KeySource::Password(pw.expose_secret()),
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

    /// Re-encrypt an existing vault with a new password.
    ///
    /// Loads the vault with the current key source, reads every secret,
    /// creates a brand-new vault encrypted with `new_password`, writes
    /// back all the secrets, and saves.  On success the in-memory state
    /// is updated to use the new password.
    pub fn change_password(&mut self, new_password: String) -> Result<()> {
        let new_password = SecretString::new(new_password);

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

        let new_vault =
            securestore::SecretsManager::new(KeySource::Password(new_password.expose_secret()))
                .context("Failed to create vault with new password")?;
        new_vault
            .save_as(&self.vault_path)
            .context("Failed to save re-encrypted vault")?;

        // 4. Reload so we can write to it.
        let mut reloaded = securestore::SecretsManager::load(
            &self.vault_path,
            KeySource::Password(new_password.expose_secret()),
        )
        .context("Failed to reload vault with new password")?;

        // 5. Write all secrets back.
        for (key, mut value) in entries {
            reloaded.set(&key, value.clone());
            value.zeroize();
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

        let meta_json =
            serde_json::to_string(entry).context("Failed to serialize credential metadata")?;
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
        fields: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        debug_assert_eq!(entry.kind, SecretKind::FormAutofill);

        let meta_key = format!("cred:{}", name);
        let fields_key = format!("val:{}:fields", name);

        let meta_json =
            serde_json::to_string(entry).context("Failed to serialize credential metadata")?;
        let fields_json =
            serde_json::to_string(fields).context("Failed to serialize form fields")?;

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
        extra: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        debug_assert_eq!(entry.kind, SecretKind::PaymentMethod);

        let meta_key = format!("cred:{}", name);
        let card_key = format!("val:{}:card", name);
        let extra_key = format!("val:{}:card_extra", name);

        let meta_json =
            serde_json::to_string(entry).context("Failed to serialize credential metadata")?;

        #[derive(serde::Serialize)]
        struct Card<'a> {
            cardholder: &'a str,
            number: &'a str,
            expiry: &'a str,
            cvv: &'a str,
        }
        let card_json = serde_json::to_string(&Card {
            cardholder,
            number,
            expiry,
            cvv,
        })
        .context("Failed to serialize card details")?;

        self.store_secret(&meta_key, &meta_json)?;
        self.store_secret(&card_key, &card_json)?;

        if !extra.is_empty() {
            let extra_json =
                serde_json::to_string(extra).context("Failed to serialize card extras")?;
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
        let entry: SecretEntry =
            serde_json::from_str(&meta_json).context("Corrupted credential metadata")?;

        // ── Disabled check ─────────────────────────────────────────
        if entry.disabled {
            anyhow::bail!("Credential '{}' is disabled", name,);
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
                let password = self.get_secret(&val_key, true)?.unwrap_or_default();
                let user_key = format!("val:{}:user", name);
                let username = self.get_secret(&user_key, true)?.unwrap_or_default();
                CredentialValue::UserPass { username, password }
            }
            SecretKind::SshKey => {
                let private_key = self.get_secret(&val_key, true)?.unwrap_or_default();
                let pub_key = format!("val:{}:pub", name);
                let public_key = self.get_secret(&pub_key, true)?.unwrap_or_default();
                CredentialValue::SshKeyPair {
                    private_key,
                    public_key,
                }
            }
            SecretKind::FormAutofill => {
                let fields_key = format!("val:{}:fields", name);
                let fields_json = self
                    .get_secret(&fields_key, true)?
                    .unwrap_or_else(|| "{}".to_string());
                let fields: std::collections::BTreeMap<String, String> =
                    serde_json::from_str(&fields_json).context("Corrupted form-autofill fields")?;
                CredentialValue::FormFields(fields)
            }
            SecretKind::PaymentMethod => {
                let card_key = format!("val:{}:card", name);
                let extra_key = format!("val:{}:card_extra", name);

                let card_json = self
                    .get_secret(&card_key, true)?
                    .unwrap_or_else(|| "{}".to_string());

                #[derive(serde::Deserialize)]
                struct Card {
                    #[serde(default)]
                    cardholder: String,
                    #[serde(default)]
                    number: String,
                    #[serde(default)]
                    expiry: String,
                    #[serde(default)]
                    cvv: String,
                }
                let card: Card =
                    serde_json::from_str(&card_json).context("Corrupted payment card data")?;

                let extra: std::collections::BTreeMap<String, String> =
                    match self.get_secret(&extra_key, true)? {
                        Some(j) => serde_json::from_str(&j).context("Corrupted card extras")?,
                        None => std::collections::BTreeMap::new(),
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
                let v = self.get_secret(&val_key, true)?.unwrap_or_default();
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
            result.push((
                key.clone(),
                SecretEntry {
                    label,
                    kind,
                    policy: AccessPolicy::WithApproval,
                    description: None,
                    disabled: false,
                },
            ));
        }

        result
    }

    /// Produce a human-readable label and [`SecretKind`] for a legacy
    /// bare vault key.
    pub(super) fn label_for_legacy_key(key: &str) -> (String, SecretKind) {
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
            let entry: SecretEntry =
                serde_json::from_str(&json).context("Corrupted credential metadata")?;

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
                    let fields_json = self
                        .get_secret(&fields_key, true)?
                        .unwrap_or_else(|| "{}".to_string());
                    let fields: std::collections::BTreeMap<String, String> =
                        serde_json::from_str(&fields_json).unwrap_or_default();
                    fields.into_iter().collect()
                }
                SecretKind::PaymentMethod => {
                    let card_key = format!("val:{}:card", name);
                    let card_json = self
                        .get_secret(&card_key, true)?
                        .unwrap_or_else(|| "{}".to_string());

                    #[derive(serde::Deserialize)]
                    struct Card {
                        #[serde(default)]
                        cardholder: String,
                        #[serde(default)]
                        number: String,
                        #[serde(default)]
                        expiry: String,
                        #[serde(default)]
                        cvv: String,
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
                        let extra: std::collections::BTreeMap<String, String> =
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
            Some(json) => {
                serde_json::from_str(&json).context("Corrupted credential metadata")?
            }
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

        let meta_json =
            serde_json::to_string(&entry).context("Failed to serialize credential metadata")?;
        self.store_secret(&meta_key, &meta_json)?;
        Ok(())
    }

    /// Change the access policy of a credential.
    pub fn set_credential_policy(&mut self, name: &str, policy: AccessPolicy) -> Result<()> {
        let meta_key = format!("cred:{}", name);

        let mut entry: SecretEntry = match self.get_secret(&meta_key, true)? {
            Some(json) => {
                serde_json::from_str(&json).context("Corrupted credential metadata")?
            }
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

        let meta_json =
            serde_json::to_string(&entry).context("Failed to serialize credential metadata")?;
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
        let private = PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .map_err(|e| anyhow::anyhow!("Failed to generate SSH key: {}", e))?;

        let private_pem = private
            .to_openssh(ssh_key::LineEnding::LF)
            .map_err(|e| anyhow::anyhow!("Failed to encode private key: {}", e))?;

        let public = private.public_key();
        let public_openssh = public
            .to_openssh()
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

        let meta_json =
            serde_json::to_string(&entry).context("Failed to serialize credential metadata")?;
        self.store_secret(&meta_key, &meta_json)?;
        self.store_secret(&val_key, private_pem.to_string().as_str())?;
        self.store_secret(&pub_vault_key, &public_str)?;

        Ok(public_str)
    }

    // ── Access policy enforcement ───────────────────────────────────

    /// Evaluate whether the given [`AccessContext`] satisfies a
    /// credential's [`AccessPolicy`].
    pub(super) fn check_access(&self, policy: &AccessPolicy, ctx: &AccessContext) -> bool {
        match policy {
            AccessPolicy::Always => true,
            AccessPolicy::WithApproval => ctx.user_approved || self.agent_access_enabled,
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
    pub(super) const TOTP_SECRET_KEY: &'static str = "__rustyclaw_totp_secret";

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
        let secret_bytes = secret
            .to_bytes()
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
        let encoded = self
            .get_secret(Self::TOTP_SECRET_KEY, true)?
            .ok_or_else(|| anyhow::anyhow!("No TOTP secret configured"))?;

        let secret = TotpSecret::Encoded(encoded);
        let secret_bytes = secret
            .to_bytes()
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

    // ── Browser-style credential storage ────────────────────────────

    /// The vault key used to store the browser credential store.
    const BROWSER_STORE_KEY: &'static str = "__rustyclaw_browser_store";

    /// Load the browser store from the vault, or create a new empty one.
    pub fn load_browser_store(&mut self) -> Result<super::types::BrowserStore> {
        match self.get_secret(Self::BROWSER_STORE_KEY, true)? {
            Some(json) => {
                let mut store: super::types::BrowserStore =
                    serde_json::from_str(&json).context("Corrupted browser store")?;
                // Purge expired cookies on load
                store.purge_expired();
                Ok(store)
            }
            None => Ok(super::types::BrowserStore::new()),
        }
    }

    /// Save the browser store to the vault.
    pub fn save_browser_store(&mut self, store: &super::types::BrowserStore) -> Result<()> {
        let json = serde_json::to_string(store).context("Failed to serialize browser store")?;
        self.store_secret(Self::BROWSER_STORE_KEY, &json)
    }

    /// Get cookies for a domain, respecting access policy.
    ///
    /// Returns cookies that match the domain (including subdomain matching).
    /// Access is controlled by the same agent_access / user_approved rules
    /// as regular secrets.
    pub fn get_cookies_for_domain(
        &mut self,
        domain: &str,
        path: &str,
        user_approved: bool,
    ) -> Result<Vec<super::types::Cookie>> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to browser cookies requires approval");
        }

        let store = self.load_browser_store()?;
        Ok(store
            .get_cookies(domain, path)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Set a cookie, respecting access policy.
    pub fn set_cookie(&mut self, cookie: super::types::Cookie, user_approved: bool) -> Result<()> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to browser cookies requires approval");
        }

        let mut store = self.load_browser_store()?;
        store.set_cookie(cookie);
        self.save_browser_store(&store)
    }

    /// Remove a cookie.
    pub fn remove_cookie(
        &mut self,
        domain: &str,
        name: &str,
        path: &str,
        user_approved: bool,
    ) -> Result<()> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to browser cookies requires approval");
        }

        let mut store = self.load_browser_store()?;
        store.remove_cookie(domain, name, path);
        self.save_browser_store(&store)
    }

    /// Clear all cookies for a domain.
    pub fn clear_domain_cookies(&mut self, domain: &str, user_approved: bool) -> Result<()> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to browser cookies requires approval");
        }

        let mut store = self.load_browser_store()?;
        store.clear_cookies(domain);
        self.save_browser_store(&store)
    }

    /// Build a Cookie header string for a request.
    ///
    /// This is the primary method used by web_fetch to attach cookies.
    /// Returns None if no cookies match or access is denied.
    pub fn cookie_header_for_request(
        &mut self,
        domain: &str,
        path: &str,
        is_secure: bool,
        user_approved: bool,
    ) -> Result<Option<String>> {
        if !self.agent_access_enabled && !user_approved {
            return Ok(None);
        }

        let store = self.load_browser_store()?;
        Ok(store.cookie_header(domain, path, is_secure))
    }

    /// Parse Set-Cookie headers from a response and store them.
    ///
    /// `response_domain` is the domain the response came from.
    /// Cookies with mismatched domains are rejected (browser security).
    pub fn store_cookies_from_response(
        &mut self,
        response_domain: &str,
        set_cookie_headers: &[String],
        user_approved: bool,
    ) -> Result<usize> {
        if !self.agent_access_enabled && !user_approved {
            return Ok(0);
        }

        let mut store = self.load_browser_store()?;
        let mut count = 0;

        for header in set_cookie_headers {
            if let Some(cookie) = Self::parse_set_cookie(header, response_domain) {
                // Security check: cookie domain must be valid for response domain
                if Self::is_valid_cookie_domain(&cookie.domain, response_domain) {
                    store.set_cookie(cookie);
                    count += 1;
                }
            }
        }

        if count > 0 {
            self.save_browser_store(&store)?;
        }

        Ok(count)
    }

    /// Parse a Set-Cookie header into a Cookie struct.
    fn parse_set_cookie(header: &str, default_domain: &str) -> Option<super::types::Cookie> {
        let parts: Vec<&str> = header.split(';').collect();
        if parts.is_empty() {
            return None;
        }

        // First part is name=value
        let name_value: Vec<&str> = parts[0].splitn(2, '=').collect();
        if name_value.len() != 2 {
            return None;
        }

        let name = name_value[0].trim().to_string();
        let value = name_value[1].trim().to_string();

        if name.is_empty() {
            return None;
        }

        let mut cookie = super::types::Cookie::new(name, value, default_domain);

        // Parse attributes
        for part in parts.iter().skip(1) {
            let attr: Vec<&str> = part.splitn(2, '=').collect();
            let attr_name = attr[0].trim().to_lowercase();
            let attr_value = attr.get(1).map(|v| v.trim()).unwrap_or("");

            match attr_name.as_str() {
                "domain" => {
                    let domain = attr_value.to_lowercase();
                    // Normalize: ensure leading dot for subdomain matching
                    cookie.domain = if domain.starts_with('.') {
                        domain
                    } else {
                        format!(".{}", domain)
                    };
                }
                "path" => {
                    cookie.path = attr_value.to_string();
                }
                "expires" => {
                    // Parse HTTP date — simplified, just use a long expiry
                    // Real implementation would parse the date properly
                    if let Ok(ts) = httpdate::parse_http_date(attr_value) {
                        if let Ok(duration) = ts.duration_since(std::time::UNIX_EPOCH) {
                            cookie.expires = Some(duration.as_secs() as i64);
                        }
                    }
                }
                "max-age" => {
                    if let Ok(secs) = attr_value.parse::<i64>() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        cookie.expires = Some(now + secs);
                    }
                }
                "secure" => {
                    cookie.secure = true;
                }
                "httponly" => {
                    cookie.http_only = true;
                }
                "samesite" => {
                    cookie.same_site = attr_value.to_lowercase();
                }
                _ => {}
            }
        }

        Some(cookie)
    }

    /// Check if a cookie domain is valid for the response domain.
    /// Implements browser security rules.
    fn is_valid_cookie_domain(cookie_domain: &str, response_domain: &str) -> bool {
        let cookie_domain = cookie_domain.to_lowercase();
        let response_domain = response_domain.to_lowercase();

        // Remove leading dot for comparison
        let cookie_base = cookie_domain.trim_start_matches('.');
        let response_base = response_domain.trim_start_matches('.');

        // Exact match
        if cookie_base == response_base {
            return true;
        }

        // Cookie domain must be a suffix of response domain
        // e.g., response "api.github.com" can set cookie for ".github.com"
        if response_base.ends_with(&format!(".{}", cookie_base)) {
            return true;
        }

        false
    }

    /// List all domains that have stored cookies.
    pub fn list_cookie_domains(&mut self) -> Result<Vec<String>> {
        let store = self.load_browser_store()?;
        Ok(store.cookie_domains().into_iter().cloned().collect())
    }

    // ── Web storage (localStorage equivalent) ───────────────────────

    /// Get a value from origin-scoped storage.
    pub fn storage_get(
        &mut self,
        origin: &str,
        key: &str,
        user_approved: bool,
    ) -> Result<Option<String>> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to web storage requires approval");
        }

        let store = self.load_browser_store()?;
        Ok(store.storage(origin).and_then(|s| s.get(key).cloned()))
    }

    /// Set a value in origin-scoped storage.
    pub fn storage_set(
        &mut self,
        origin: &str,
        key: &str,
        value: &str,
        user_approved: bool,
    ) -> Result<()> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to web storage requires approval");
        }

        let mut store = self.load_browser_store()?;
        store.storage_mut(origin).set(key, value);
        self.save_browser_store(&store)
    }

    /// Remove a value from origin-scoped storage.
    pub fn storage_remove(&mut self, origin: &str, key: &str, user_approved: bool) -> Result<()> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to web storage requires approval");
        }

        let mut store = self.load_browser_store()?;
        store.storage_mut(origin).remove(key);
        self.save_browser_store(&store)
    }

    /// Clear all storage for an origin.
    pub fn storage_clear(&mut self, origin: &str, user_approved: bool) -> Result<()> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to web storage requires approval");
        }

        let mut store = self.load_browser_store()?;
        store.clear_storage(origin);
        self.save_browser_store(&store)
    }

    /// List all origins that have stored data.
    pub fn list_storage_origins(&mut self) -> Result<Vec<String>> {
        let store = self.load_browser_store()?;
        Ok(store.storage_origins().into_iter().cloned().collect())
    }

    /// List all keys in storage for an origin.
    pub fn storage_keys(&mut self, origin: &str, user_approved: bool) -> Result<Vec<String>> {
        if !self.agent_access_enabled && !user_approved {
            anyhow::bail!("Access denied: agent access to web storage requires approval");
        }

        let store = self.load_browser_store()?;
        Ok(store
            .storage(origin)
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default())
    }
}

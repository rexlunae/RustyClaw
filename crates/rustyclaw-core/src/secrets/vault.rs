//! Encryption/decryption logic, file I/O for the vault, and TOTP functionality.

use anyhow::{Context, Result};
use securestore::KeySource;

use super::SecretsManager;
use super::types::{
    AccessContext, AccessPolicy, CredentialValue, SecretEntry, SecretKind, SecretString,
};

/// Restrict a file to owner read/write only (mode `0o600`) on Unix.
///
/// No-op on non-Unix platforms (Windows ACLs are inherited from the
/// parent directory, which the vault places under the user profile).
#[allow(unused_variables)]
fn set_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

impl SecretsManager {
    /// Ensure the vault is loaded (or created if it doesn't exist yet).
    pub(super) fn ensure_vault(&mut self) -> Result<&mut securestore::SecretsManager> {
        if self.vault.is_none() {
            let vault = if self.vault_path.exists() {
                // Existing vault — load with password or key file.
                if let Some(ref pw) = self.password {
                    securestore::SecretsManager::load(&self.vault_path, KeySource::Password(pw))
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
                    securestore::SecretsManager::load(&self.vault_path, KeySource::Password(pw))
                        .context("Failed to reload newly-created secrets vault")?
                } else {
                    // Key-file-based vault.
                    let sman = securestore::SecretsManager::new(KeySource::Csprng)
                        .context("Failed to create new secrets vault")?;
                    sman.export_key(&self.key_path)
                        .context("Failed to export secrets key")?;
                    // Restrict the master key to owner-only (0o600). securestore
                    // does not set permissions itself, so without this the key
                    // inherits the process umask and may be group/world-readable.
                    set_owner_only_permissions(&self.key_path)
                        .context("Failed to secure secrets key permissions")?;
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
        // 1. Make sure the vault is loaded with the *current* credentials.
        let old_vault = self.ensure_vault()?;

        // 2. Read out every key → value pair.
        let keys: Vec<String> = old_vault.keys().map(|s| s.to_string()).collect();
        let mut entries: Vec<(String, String)> = Vec::new();
        for key in &keys {
            if let Ok(value) = old_vault.get(key) {
                entries.push((key.clone(), value))
            }
        }

        // 3. Drop the old vault and create a new one with the new password.
        self.vault = None;

        let new_vault = securestore::SecretsManager::new(KeySource::Password(&new_password))
            .context("Failed to create vault with new password")?;
        new_vault
            .save_as(&self.vault_path)
            .context("Failed to save re-encrypted vault")?;

        // 4. Reload so we can write to it.
        let mut reloaded =
            securestore::SecretsManager::load(&self.vault_path, KeySource::Password(&new_password))
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
        //
        // NOTE: Values are wrapped in SecretString to zero memory on drop.
        // The earlier `get_secret` returns raw String for backward compatibility;
        // we intentionally convert to SecretString here as the single chokepoint
        // where credential values enter the process.
        let value = match entry.kind {
            SecretKind::UsernamePassword => {
                let password =
                    SecretString::new(self.get_secret(&val_key, true)?.unwrap_or_default());
                let user_key = format!("val:{}:user", name);
                let username =
                    SecretString::new(self.get_secret(&user_key, true)?.unwrap_or_default());
                CredentialValue::UserPass { username, password }
            }
            SecretKind::SshKey => {
                let private_key =
                    SecretString::new(self.get_secret(&val_key, true)?.unwrap_or_default());
                let pub_key = format!("val:{}:pub", name);
                let public_key =
                    SecretString::new(self.get_secret(&pub_key, true)?.unwrap_or_default());
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
                    cardholder: SecretString::new(card.cardholder),
                    number: SecretString::new(card.number),
                    expiry: SecretString::new(card.expiry),
                    cvv: SecretString::new(card.cvv),
                    extra,
                }
            }
            _ => {
                let v = self.get_secret(&val_key, true)?.unwrap_or_default();
                CredentialValue::Single(SecretString::new(v))
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
                return (p.display.to_string(), kind);
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
}

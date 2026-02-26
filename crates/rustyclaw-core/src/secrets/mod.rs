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
    SecretKind, WebStorage,
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
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};
    use totp_rs::{Algorithm, Secret as TotpSecret, TOTP};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("rustyclaw_test_{}_{}", std::process::id(), id));
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
            assert_eq!(
                m.get_secret("api_key", false).unwrap(),
                Some("sk-abc".to_string())
            );
            assert_eq!(
                m.get_secret("token", false).unwrap(),
                Some("tok-xyz".to_string())
            );
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
            assert_eq!(
                m.get_secret("secret", false).unwrap(),
                Some("value123".to_string())
            );
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
        let encoded = manager
            .get_secret(SecretsManager::TOTP_SECRET_KEY, true)
            .unwrap()
            .unwrap();
        let secret = TotpSecret::Encoded(encoded);
        let secret_bytes = secret.to_bytes().unwrap();
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret_bytes,
            Some("RustyClaw".to_string()),
            "testuser".to_string(),
        )
        .unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
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
        m.store_credential("anthropic_key", &entry, "sk-ant-12345", None)
            .unwrap();

        let ctx = AccessContext {
            user_approved: true,
            ..Default::default()
        };
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
        m.store_credential("registry", &entry, "s3cret", Some("admin"))
            .unwrap();

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
        m.store_credential("passkey1", &entry, "cred-id-base64", None)
            .unwrap();

        // Access without authentication should be denied.
        let ctx = AccessContext {
            user_approved: true,
            ..Default::default()
        };
        assert!(m.get_credential("passkey1", &ctx).is_err());

        // Access with authentication should succeed.
        let ctx = AccessContext {
            authenticated: true,
            ..Default::default()
        };
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

        let pubkey = m
            .generate_ssh_key(
                "rustyclaw_agent",
                "rustyclaw@agent",
                AccessPolicy::WithApproval,
            )
            .unwrap();

        assert!(pubkey.starts_with("ssh-ed25519 "));
        assert!(pubkey.contains("rustyclaw@agent"));

        // Retrieve via typed API.
        let ctx = AccessContext {
            user_approved: true,
            ..Default::default()
        };
        let (meta, val) = m.get_credential("rustyclaw_agent", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::SshKey);
        match val {
            CredentialValue::SshKeyPair {
                private_key,
                public_key,
            } => {
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
        m.store_credential("typed_one", &entry, "val", None)
            .unwrap();

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
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with("cred:") || n.starts_with("val:"))
        );

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
        let ctx = AccessContext {
            user_approved: true,
            ..Default::default()
        };
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
        let ctx = AccessContext {
            user_approved: true,
            ..Default::default()
        };
        assert!(m.get_credential("hs", &ctx).is_err());

        let ctx = AccessContext {
            authenticated: true,
            ..Default::default()
        };
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
        let ctx = AccessContext {
            user_approved: true,
            ..Default::default()
        };
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
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("name".to_string(), "Ada Lovelace".to_string());
        fields.insert("email".to_string(), "ada@example.com".to_string());
        fields.insert("phone".to_string(), "+1-555-0100".to_string());
        fields.insert("address".to_string(), "1 Infinite Loop".to_string());

        m.store_form_autofill("shipping", &entry, &fields).unwrap();

        let ctx = AccessContext {
            user_approved: true,
            ..Default::default()
        };
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
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("billing_zip".to_string(), "94025".to_string());

        m.store_payment_method(
            "visa_4242",
            &entry,
            "A. Lovelace",
            "4242424242424242",
            "12/28",
            "123",
            &extra,
        )
        .unwrap();

        // Needs authentication.
        let ctx = AccessContext {
            user_approved: true,
            ..Default::default()
        };
        assert!(m.get_credential("visa_4242", &ctx).is_err());

        let ctx = AccessContext {
            authenticated: true,
            ..Default::default()
        };
        let (meta, val) = m.get_credential("visa_4242", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::PaymentMethod);
        match val {
            CredentialValue::PaymentCard {
                cardholder,
                number,
                expiry,
                cvv,
                extra,
            } => {
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
        m.store_credential("gh_recovery", &entry, note, None)
            .unwrap();

        let ctx = AccessContext {
            authenticated: true,
            ..Default::default()
        };
        let (meta, val) = m.get_credential("gh_recovery", &ctx).unwrap().unwrap();
        assert_eq!(meta.kind, SecretKind::SecureNote);
        assert_eq!(
            meta.description,
            Some("GitHub 2FA backup codes".to_string())
        );
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
        let mut fields = std::collections::BTreeMap::new();
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
        m.set_credential_policy("k", AccessPolicy::WithAuth)
            .unwrap();
        let creds = m.list_credentials();
        assert_eq!(creds[0].1.policy, AccessPolicy::WithAuth);

        // Change to SKILL.
        m.set_credential_policy("k", AccessPolicy::SkillOnly(vec!["web".to_string()]))
            .unwrap();
        let creds = m.list_credentials();
        assert_eq!(
            creds[0].1.policy,
            AccessPolicy::SkillOnly(vec!["web".to_string()])
        );

        // Change back to ASK.
        m.set_credential_policy("k", AccessPolicy::WithApproval)
            .unwrap();
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
        m.set_credential_policy("LEGACY_KEY", AccessPolicy::Always)
            .unwrap();

        let all = m.list_all_entries();
        let entry = all.iter().find(|(n, _)| n == "LEGACY_KEY").unwrap();
        assert_eq!(entry.1.policy, AccessPolicy::Always);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

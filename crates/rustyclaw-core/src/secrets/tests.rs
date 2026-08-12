//! Tests for the secrets manager.

use super::*;
use crate::ignore::Ignore;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use totp_rs::{Algorithm, Secret as TotpSecret, TOTP};

static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A scratch directory that removes itself when the test ends.
///
/// This replaces a `remove_dir_all` as the last line of each test — which
/// meant a test that tripped an assertion never reached its own cleanup and
/// left the directory behind. A guard runs during unwind too, so the failing
/// tests clean up as reliably as the passing ones.
struct ScratchDir(PathBuf);

impl std::ops::Deref for ScratchDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for ScratchDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// `SecretsManager` takes `impl Into<PathBuf>`, which `Deref` does not reach.
impl From<&ScratchDir> for PathBuf {
    fn from(d: &ScratchDir) -> Self {
        d.0.clone()
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // The one place a discarded error is right: this runs during unwind
        // from a failing assertion, and panicking here would replace the real
        // failure with a cleanup error.
        std::fs::remove_dir_all(&self.0).ignore();
    }
}

fn temp_dir() -> ScratchDir {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rustyclaw_test_{}_{}", std::process::id(), id));
    // Not redundant with the guard: a run that crashed hard leaves its
    // directory behind, and pids get reused. Loud rather than ignored — a
    // stale directory this cannot clear would seed the test with another
    // run's vault files, and silently passing that on is how a test starts
    // proving the wrong thing.
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clearing a stale scratch directory");
    }
    std::fs::create_dir_all(&dir).unwrap();
    ScratchDir(dir)
}

#[test]
fn test_secrets_manager_creation() {
    let dir = temp_dir();
    let manager = SecretsManager::new(&dir);
    assert!(!manager.has_agent_access());

    // Vault files should not exist yet (lazy creation)
    assert!(!dir.join("secrets.json").exists());
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
}

#[cfg(unix)]
#[test]
fn test_key_file_permissions_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir();
    let mut manager = SecretsManager::new(&dir);
    manager.set_agent_access(true);

    // Storing a secret lazily creates the key-file-based vault.
    manager.store_secret("api_key", "hunter2").unwrap();

    let key_path = dir.join("secrets.key");
    assert!(key_path.exists(), "key file should exist");

    let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
    // Mask to the permission bits; must be exactly owner read/write (0o600),
    // i.e. no group/other access.
    assert_eq!(
        mode & 0o777,
        0o600,
        "secrets.key must be owner-only (0o600), got {:o}",
        mode & 0o777
    );
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
    assert!(
        manager
            .verify_totp(&format!("{} {}", &code[0..3], &code[3..6]))
            .unwrap()
    );
    assert!(
        manager
            .verify_totp(&format!("{}-{}", &code[0..3], &code[3..6]))
            .unwrap()
    );

    // Wrong code should fail.
    assert!(!manager.verify_totp("000000").unwrap());

    // Remove TOTP.
    manager.remove_totp().unwrap();
    assert!(!manager.has_totp());
}

#[test]
fn test_totp_clock_drift_detection() {
    use super::TotpOutcome;

    let dir = temp_dir();
    let mut manager = SecretsManager::new(&dir);
    manager.set_agent_access(true);
    manager.setup_totp("testuser").unwrap();

    let encoded = manager
        .get_secret(SecretsManager::TOTP_SECRET_KEY, true)
        .unwrap()
        .unwrap();
    let secret_bytes = TotpSecret::Encoded(encoded).to_bytes().unwrap();
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

    // A current code is Valid.
    assert_eq!(
        manager.verify_totp_detailed(&totp.generate(now)).unwrap(),
        TotpOutcome::Valid
    );

    // A code from a clock ~2.5 minutes ahead fails the real check but is
    // reported as drift, not a plain mismatch. (Step 5 is outside the ±1
    // accepted window and inside the diagnostic scan.)
    let outcome = manager
        .verify_totp_detailed(&totp.generate(now + 5 * 30))
        .unwrap();
    assert!(
        matches!(outcome, TotpOutcome::ClockDrift { steps } if steps.unsigned_abs() >= 2),
        "expected ClockDrift, got {outcome:?}"
    );
    // Drift is diagnostic only — verify_totp still rejects.
    assert!(!manager.verify_totp(&totp.generate(now + 5 * 30)).unwrap());
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
}

#[test]
fn test_trigger_scoped_secret() {
    let dir = temp_dir();
    let mut m = SecretsManager::new(&dir);
    let entry = SecretEntry {
        label: "stripe".to_string(),
        kind: SecretKind::ApiKey,
        policy: AccessPolicy::WithApproval,
        description: None,
        disabled: false,
    };
    m.store_credential("stripe", &entry, "sk_live_ABC", None)
        .unwrap();

    // Not linked to any trigger → denied.
    assert!(m.get_secret_for_trigger("stripe", "watcher").is_err());

    // Link to "watcher" → only that trigger can read it.
    m.set_credential_trigger_link("stripe", "watcher", true)
        .unwrap();
    assert_eq!(m.credential_triggers("stripe"), vec!["watcher".to_string()]);
    assert_eq!(
        m.get_secret_for_trigger("stripe", "watcher")
            .unwrap()
            .as_deref(),
        Some("sk_live_ABC")
    );
    assert!(m.get_secret_for_trigger("stripe", "other").is_err());

    // A missing secret returns Ok(None), not an error.
    assert!(
        m.get_secret_for_trigger("nope", "watcher")
            .unwrap()
            .is_none()
    );

    // Unlink → denied again.
    m.set_credential_trigger_link("stripe", "watcher", false)
        .unwrap();
    assert!(m.get_secret_for_trigger("stripe", "watcher").is_err());
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
}

/// A password change must never have a moment where the only copy of the
/// secrets is a file mid-write. After it succeeds, the previous state
/// survives as a pair — the old vault and the key that opens it — because a
/// backup whose key was deleted is ciphertext, not a backup.
#[test]
fn change_password_leaves_a_recoverable_backup() {
    let dir = temp_dir();
    {
        let mut m = SecretsManager::new(&dir);
        m.store_secret("api_key", "sk-123").unwrap();
        m.change_password("pw1".to_string()).unwrap();
    }

    // The re-keyed vault opens with the new password and holds the secret.
    let mut m = SecretsManager::with_password(&dir, "pw1".to_string());
    assert_eq!(
        m.get_secret("api_key", true).unwrap().as_deref(),
        Some("sk-123")
    );

    // The previous state is set aside, not destroyed — and the backup key
    // still opens the backup vault.
    let vault_bak = dir.join("secrets.json.bak");
    let key_bak = dir.join("secrets.key.bak");
    assert!(vault_bak.exists(), "old vault should be kept as .bak");
    assert!(key_bak.exists(), "old key must survive with the old vault");
    let backup =
        securestore::SecretsManager::load(&vault_bak, securestore::KeySource::from_file(&key_bak))
            .unwrap();
    assert_eq!(backup.get("api_key").unwrap(), "sk-123");

    // No staging leftovers.
    assert!(!dir.join("secrets.json.rekey").exists());
}

// ── `requires_password` ─────────────────────────────────────────────────
//
// The gateway asks this before deciding whether to prompt on startup, so a
// wrong answer here is a gateway that never asks for the passphrase to a
// vault it then cannot open.

#[test]
fn a_vault_with_no_key_file_requires_a_password() {
    let dir = temp_dir();
    std::fs::write(dir.join("secrets.json"), "{}").unwrap();
    assert!(SecretsManager::requires_password(&dir));
}

#[test]
fn a_vault_with_a_key_file_does_not_require_a_password() {
    let dir = temp_dir();
    std::fs::write(dir.join("secrets.json"), "{}").unwrap();
    std::fs::write(dir.join("secrets.key"), "key").unwrap();
    assert!(!SecretsManager::requires_password(&dir));
}

/// No vault yet is not "no password": nothing has chosen a key source, so
/// the caller's config decides. Answering `true` here would prompt every
/// first run before onboarding had created anything.
#[test]
fn no_vault_on_disk_does_not_require_a_password() {
    let dir = temp_dir();
    assert!(!SecretsManager::requires_password(&dir));
}

/// The rule `is_locked` applies to a live manager and the one
/// `requires_password` applies to a directory are the same rule, and must
/// not drift: a password vault opened with no password is locked.
#[test]
fn requires_password_agrees_with_is_locked() {
    let dir = temp_dir();
    std::fs::write(dir.join("secrets.json"), "{}").unwrap();

    assert!(SecretsManager::requires_password(&dir));
    assert!(SecretsManager::new(&dir).is_locked());
    assert!(!SecretsManager::with_password(&dir, "pw".to_string()).is_locked());
}

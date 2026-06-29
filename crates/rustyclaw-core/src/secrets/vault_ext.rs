//! Credential management methods for `SecretsManager` (continued from `vault`).

use anyhow::{Context, Result};
use totp_rs::{Algorithm, Secret as TotpSecret, TOTP};

use super::SecretsManager;
use super::types::{AccessContext, AccessPolicy, SecretEntry, SecretKind};

impl SecretsManager {
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
            Some(json) => serde_json::from_str(&json).context("Corrupted credential metadata")?,
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
            Some(json) => serde_json::from_str(&json).context("Corrupted credential metadata")?,
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
        let private = PrivateKey::random(&mut rand::rng(), ssh_key::Algorithm::Ed25519)
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

        // Users often paste codes formatted like "123 456" or "123-456".
        // Accept those by stripping non-digits, but still require a strict
        // 6-digit final code (our configured TOTP digit width).
        let digits_only: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
        let candidate = if digits_only.is_empty() {
            code.trim().to_string()
        } else {
            digits_only
        };
        if candidate.len() != 6 || !candidate.chars().all(|c| c.is_ascii_digit()) {
            return Ok(false);
        }

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

        Ok(totp.check(&candidate, now))
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

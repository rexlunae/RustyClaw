//! Type definitions for the secrets module.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// A [`String`] wrapper that zeroes its contents on drop.
///
/// Protects secret material (API keys, passwords, tokens) from being
/// recovered from process memory after the value is no longer needed.
///
/// This does **not** guarantee the memory is zeroed (the allocator may
/// leave copies, the OS may swap pages to disk, etc.), but it prevents
/// the easy case — a later allocation reading the same physical memory
/// and recovering the secret from the old `String`'s backing buffer.
///
/// Common patterns:
/// - [`SecretEntry`] values returned by vault reads
/// - Temporary credential values during agent tool execution
/// - Provider API keys held in memory during a request
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct SecretString(String);

impl SecretString {
    /// Create a new `SecretString` from a [`String`], consuming it.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Expose the inner value for read access.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the `SecretString` and return the inner [`String`].
    ///
    /// **Warning:** The returned [`String`] is NOT zeroed on drop.
    /// Only use this when the value will immediately be consumed
    /// (e.g. passed to a function that takes ownership).
    pub fn into_inner(self) -> String {
        let s = std::mem::ManuallyDrop::new(self);
        // SAFETY: We own `s` and prevent its Drop, so moving the inner String is sound.
        unsafe { std::ptr::read(&s.0) }
    }

    /// Convert to a `String` without consuming (clones the inner value).
    pub fn to_string_unsecured(&self) -> String {
        self.0.clone()
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SecretString").field(&"[REDACTED]").finish()
    }
}

impl std::fmt::Display for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<SecretString> for String {
    fn from(s: SecretString) -> Self {
        // Use ManuallyDrop to avoid running SecretString's Drop (which would zero the value)
        // at the same time we destructure to move the inner String out.
        let s = std::mem::ManuallyDrop::new(s);
        // SAFETY: We own `s` and prevent its Drop, so moving the inner String is sound.
        // The caller takes ownership of the String and is responsible for its lifecycle.
        unsafe { std::ptr::read(&s.0) }
    }
}

impl std::ops::Deref for SecretString {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

// ── Comparisons (needed by tests and display code) ─────────────────────────

impl PartialEq<&str> for SecretString {
    fn eq(&self, other: &&str) -> bool {
        self.0.as_str() == *other
    }
}

impl PartialEq<SecretString> for &str {
    fn eq(&self, other: &SecretString) -> bool {
        *self == other.0.as_str()
    }
}

impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for SecretString {}

use std::collections::BTreeMap;

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
#[derive(Default)]
pub enum AccessPolicy {
    /// The agent may read this secret at any time without prompting.
    Always,
    /// The agent may read this secret only with explicit per-use user
    /// approval (e.g. a "yes/no" confirmation in the TUI).
    #[default]
    WithApproval,
    /// The agent must re-authenticate (vault password and/or TOTP)
    /// before each access.
    WithAuth,
    /// The secret is only available when the agent is executing one of
    /// the named skills.  An empty list means "no skill may access it"
    /// (effectively locked).
    SkillOnly(Vec<String>),
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
///
/// Sensitive string fields use [`SecretString`] for automatic zeroing on drop.
#[derive(Clone)]
pub enum CredentialValue {
    /// A single opaque string (ApiKey, Token, HttpPasskey, Other).
    Single(SecretString),
    /// Username + password pair.
    UserPass {
        username: SecretString,
        password: SecretString,
    },
    /// SSH keypair — private key in OpenSSH PEM format, public key in
    /// `ssh-ed25519 AAAA…` format.
    SshKeyPair {
        private_key: SecretString,
        public_key: SecretString,
    },
    /// Arbitrary key/value pairs (form autofill fields).
    FormFields(BTreeMap<String, String>),
    /// Payment card details.
    PaymentCard {
        cardholder: SecretString,
        number: SecretString,
        expiry: SecretString,
        cvv: SecretString,
        /// Optional billing-address / notes fields.
        extra: BTreeMap<String, String>,
    },
}

impl std::fmt::Debug for CredentialValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(_) => f.debug_tuple("Single").field(&"[REDACTED]").finish(),
            Self::UserPass { .. } => f
                .debug_struct("UserPass")
                .field("username", &"[REDACTED]")
                .field("password", &"[REDACTED]")
                .finish(),
            Self::SshKeyPair { .. } => f
                .debug_struct("SshKeyPair")
                .field("private_key", &"[REDACTED]")
                .field("public_key", &"[REDACTED]")
                .finish(),
            Self::FormFields(fields) => f.debug_struct("FormFields").field("fields", fields).finish(),
            Self::PaymentCard { .. } => f
                .debug_struct("PaymentCard")
                .field("cardholder", &"[REDACTED]")
                .field("number", &"[REDACTED]")
                .field("expiry", &"[REDACTED]")
                .field("cvv", &"[REDACTED]")
                .field("extra", &"[REDACTED]")
                .finish(),
        }
    }
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

/// Kept for backward compatibility with older code that references this type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    pub key: String,
    pub description: Option<String>,
}

// ── Browser-style credential storage ────────────────────────────────────────

/// An HTTP cookie with standard browser attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Domain the cookie is valid for (e.g. ".github.com" for subdomains).
    pub domain: String,
    /// Path the cookie is valid for (default "/").
    #[serde(default = "default_path")]
    pub path: String,
    /// Expiration timestamp (Unix seconds). None = session cookie (but we persist anyway).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<i64>,
    /// Cookie should only be sent over HTTPS.
    #[serde(default)]
    pub secure: bool,
    /// Cookie should not be accessible to JavaScript (browser enforcement).
    /// For agents, this is informational — we still allow access but tools
    /// should respect it when simulating browser behavior.
    #[serde(default)]
    pub http_only: bool,
    /// SameSite attribute: "strict", "lax", or "none".
    #[serde(default = "default_same_site")]
    pub same_site: String,
}

fn default_path() -> String {
    "/".to_string()
}

fn default_same_site() -> String {
    "lax".to_string()
}

impl Cookie {
    /// Create a simple cookie with defaults.
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".to_string(),
            expires: None,
            secure: false,
            http_only: false,
            same_site: "lax".to_string(),
        }
    }

    /// Check if this cookie is valid for a given domain.
    /// Implements standard domain matching:
    /// - Exact match: "example.com" matches "example.com"
    /// - Subdomain match: ".example.com" matches "sub.example.com"
    pub fn matches_domain(&self, request_domain: &str) -> bool {
        let cookie_domain = self.domain.to_lowercase();
        let req_domain = request_domain.to_lowercase();

        if let Some(suffix) = cookie_domain.strip_prefix('.') {
            // Subdomain matching: .example.com matches foo.example.com
            req_domain == suffix || req_domain.ends_with(&format!(".{}", suffix))
        } else {
            // Exact match only
            req_domain == cookie_domain
        }
    }

    /// Check if this cookie is valid for a given path.
    pub fn matches_path(&self, request_path: &str) -> bool {
        request_path.starts_with(&self.path)
            || (self.path.ends_with('/')
                && request_path.starts_with(self.path.trim_end_matches('/')))
    }

    /// Check if the cookie has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(expires) = self.expires {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            expires < now
        } else {
            false // No expiry = persistent (we don't do session cookies)
        }
    }

    /// Format as a Set-Cookie header value.
    pub fn to_set_cookie_header(&self) -> String {
        let mut parts = vec![format!("{}={}", self.name, self.value)];
        parts.push(format!("Domain={}", self.domain));
        parts.push(format!("Path={}", self.path));
        if let Some(exp) = self.expires {
            // Format as HTTP date would be better, but Unix timestamp works for storage
            parts.push(format!("Max-Age={}", exp));
        }
        if self.secure {
            parts.push("Secure".to_string());
        }
        if self.http_only {
            parts.push("HttpOnly".to_string());
        }
        parts.push(format!("SameSite={}", self.same_site));
        parts.join("; ")
    }

    /// Format as a Cookie header value (just name=value).
    pub fn to_cookie_header(&self) -> String {
        format!("{}={}", self.name, self.value)
    }
}

/// Origin-scoped storage (like browser localStorage).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebStorage {
    /// The origin this storage belongs to (e.g. "https://github.com").
    pub origin: String,
    /// Key-value pairs.
    pub data: BTreeMap<String, String>,
}

impl WebStorage {
    pub fn new(origin: impl Into<String>) -> Self {
        Self {
            origin: origin.into(),
            data: BTreeMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.data.get(key)
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.data.insert(key.into(), value.into());
    }

    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.data.remove(key)
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.data.keys()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Container for all browser-style credentials.
/// Stored as a single encrypted blob in the vault under key "browser_store".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserStore {
    /// Cookies indexed by domain (normalized to lowercase).
    /// Each domain has a vec of cookies.
    pub cookies: BTreeMap<String, Vec<Cookie>>,
    /// Origin-scoped storage (localStorage equivalent).
    pub storage: BTreeMap<String, WebStorage>,
}

impl BrowserStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Cookie operations ────────────────────────────────────────────

    /// Get all non-expired cookies that match a domain and path.
    pub fn get_cookies(&self, domain: &str, path: &str) -> Vec<&Cookie> {
        let domain_lower = domain.to_lowercase();
        let mut result = Vec::new();

        for cookies in self.cookies.values() {
            for cookie in cookies {
                if !cookie.is_expired()
                    && cookie.matches_domain(&domain_lower)
                    && cookie.matches_path(path)
                {
                    result.push(cookie);
                }
            }
        }

        result
    }

    /// Get a specific cookie by name for a domain.
    pub fn get_cookie(&self, domain: &str, name: &str) -> Option<&Cookie> {
        self.get_cookies(domain, "/")
            .into_iter()
            .find(|c| c.name == name)
    }

    /// Set a cookie (replaces existing cookie with same name/domain/path).
    pub fn set_cookie(&mut self, cookie: Cookie) {
        let domain_key = cookie
            .domain
            .to_lowercase()
            .trim_start_matches('.')
            .to_string();
        let cookies = self.cookies.entry(domain_key).or_default();

        // Remove existing cookie with same name and path
        cookies.retain(|c| !(c.name == cookie.name && c.path == cookie.path));

        // Don't add if already expired
        if !cookie.is_expired() {
            cookies.push(cookie);
        }
    }

    /// Remove a specific cookie.
    pub fn remove_cookie(&mut self, domain: &str, name: &str, path: &str) {
        let domain_key = domain.to_lowercase().trim_start_matches('.').to_string();
        if let Some(cookies) = self.cookies.get_mut(&domain_key) {
            cookies.retain(|c| !(c.name == name && c.path == path));
        }
    }

    /// Clear all cookies for a domain.
    pub fn clear_cookies(&mut self, domain: &str) {
        let domain_key = domain.to_lowercase().trim_start_matches('.').to_string();
        self.cookies.remove(&domain_key);
    }

    /// Remove all expired cookies.
    pub fn purge_expired(&mut self) {
        for cookies in self.cookies.values_mut() {
            cookies.retain(|c| !c.is_expired());
        }
        // Remove empty domain entries
        self.cookies.retain(|_, v| !v.is_empty());
    }

    /// Build a Cookie header string for a request to a given URL.
    pub fn cookie_header(&self, domain: &str, path: &str, is_secure: bool) -> Option<String> {
        let cookies: Vec<_> = self
            .get_cookies(domain, path)
            .into_iter()
            .filter(|c| !c.secure || is_secure) // Only send Secure cookies over HTTPS
            .collect();

        if cookies.is_empty() {
            None
        } else {
            Some(
                cookies
                    .iter()
                    .map(|c| c.to_cookie_header())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        }
    }

    /// List all domains that have cookies.
    pub fn cookie_domains(&self) -> Vec<&String> {
        self.cookies.keys().collect()
    }

    // ── Storage operations ───────────────────────────────────────────

    /// Get storage for an origin, creating if needed.
    pub fn storage_mut(&mut self, origin: &str) -> &mut WebStorage {
        let origin_key = origin.to_lowercase();
        self.storage
            .entry(origin_key.clone())
            .or_insert_with(|| WebStorage::new(origin_key))
    }

    /// Get storage for an origin (read-only).
    pub fn storage(&self, origin: &str) -> Option<&WebStorage> {
        self.storage.get(&origin.to_lowercase())
    }

    /// Clear storage for an origin.
    pub fn clear_storage(&mut self, origin: &str) {
        self.storage.remove(&origin.to_lowercase());
    }

    /// List all origins that have storage.
    pub fn storage_origins(&self) -> Vec<&String> {
        self.storage.keys().collect()
    }
}

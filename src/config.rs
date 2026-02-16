use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProvider {
    /// Provider id (e.g. "anthropic", "openai", "google", "ollama", "custom")
    pub provider: String,
    /// Default model name (e.g. "claude-sonnet-4-20250514")
    pub model: Option<String>,
    /// API base URL (only required for custom/proxy providers)
    pub base_url: Option<String>,
}

/// Sandbox configuration for agent isolation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxConfig {
    /// Sandbox mode: "auto", "landlock+bwrap", "landlock", "bwrap", "docker", "macos", "path", "none"
    #[serde(default)]
    pub mode: String,
    /// Additional paths to deny (beyond credentials dir)
    #[serde(default)]
    pub deny_paths: Vec<PathBuf>,
    /// Paths to allow in strict mode
    #[serde(default)]
    pub allow_paths: Vec<PathBuf>,
}

/// SSRF (Server-Side Request Forgery) protection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsrfConfig {
    /// Whether SSRF protection is enabled
    #[serde(default = "SsrfConfig::default_enabled")]
    pub enabled: bool,
    /// Additional blocked CIDR ranges beyond the defaults
    #[serde(default)]
    pub blocked_cidrs: Vec<String>,
    /// Allow private IPs (override for trusted environments)
    #[serde(default)]
    pub allow_private_ips: bool,
}

impl Default for SsrfConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            blocked_cidrs: Vec::new(),
            allow_private_ips: false,
        }
    }
}

impl SsrfConfig {
    fn default_enabled() -> bool {
        true
    }
}

/// Prompt injection defense configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptGuardConfig {
    /// Whether prompt injection defense is enabled
    #[serde(default = "PromptGuardConfig::default_enabled")]
    pub enabled: bool,
    /// Action to take: "warn", "block", or "sanitize"
    #[serde(default = "PromptGuardConfig::default_action")]
    pub action: String,
    /// Sensitivity threshold (0.0-1.0, higher = more strict)
    #[serde(default = "PromptGuardConfig::default_sensitivity")]
    pub sensitivity: f64,
}

impl Default for PromptGuardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            action: Self::default_action(),
            sensitivity: Self::default_sensitivity(),
        }
    }
}

impl PromptGuardConfig {
    fn default_enabled() -> bool {
        true
    }

    fn default_action() -> String {
        "warn".to_string()
    }

    fn default_sensitivity() -> f64 {
        0.7
    }
}

/// TLS/WSS configuration for the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Whether TLS is enabled for the gateway
    #[serde(default)]
    pub enabled: bool,
    /// Path to TLS certificate file (PEM format)
    #[serde(default)]
    pub cert_path: Option<PathBuf>,
    /// Path to TLS private key file (PEM format)
    #[serde(default)]
    pub key_path: Option<PathBuf>,
    /// Generate self-signed certificate for development/local use
    #[serde(default)]
    pub self_signed: bool,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_path: None,
            key_path: None,
            self_signed: false,
        }
    }
}

/// Prometheus metrics configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Whether metrics endpoint is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Address to bind metrics server (default: 127.0.0.1:9090)
    #[serde(default = "MetricsConfig::default_listen")]
    pub listen: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: Self::default_listen(),
        }
    }
}

impl MetricsConfig {
    fn default_listen() -> String {
        "127.0.0.1:9090".to_string()
    }
}

/// Health check endpoint configuration for remote monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    /// Whether health check endpoint is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Address to bind health check server (default: 127.0.0.1:8080)
    #[serde(default = "HealthConfig::default_listen")]
    pub listen: String,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: Self::default_listen(),
        }
    }
}

impl HealthConfig {
    fn default_listen() -> String {
        "127.0.0.1:8080".to_string()
    }
}

/// Voice features configuration (STT/TTS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Whether voice features are enabled
    #[serde(default)]
    pub enabled: bool,
    /// STT provider to use (openai, google, azure, vosk)
    #[serde(default = "VoiceConfig::default_stt_provider")]
    pub stt_provider: String,
    /// TTS provider to use (openai, elevenlabs, google, azure, coqui)
    #[serde(default = "VoiceConfig::default_tts_provider")]
    pub tts_provider: String,
    /// Whether wake word detection is enabled
    #[serde(default)]
    pub wake_word_enabled: bool,
    /// Wake word phrase
    #[serde(default = "VoiceConfig::default_wake_word")]
    pub wake_word: String,
    /// Audio input device
    #[serde(default = "VoiceConfig::default_device")]
    pub input_device: String,
    /// Audio output device
    #[serde(default = "VoiceConfig::default_device")]
    pub output_device: String,
    /// Sample rate for audio capture
    #[serde(default = "VoiceConfig::default_sample_rate")]
    pub sample_rate: u32,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stt_provider: Self::default_stt_provider(),
            tts_provider: Self::default_tts_provider(),
            wake_word_enabled: false,
            wake_word: Self::default_wake_word(),
            input_device: Self::default_device(),
            output_device: Self::default_device(),
            sample_rate: Self::default_sample_rate(),
        }
    }
}

impl VoiceConfig {
    fn default_stt_provider() -> String {
        "openai".to_string()
    }

    fn default_tts_provider() -> String {
        "openai".to_string()
    }

    fn default_wake_word() -> String {
        "hey rustyclaw".to_string()
    }

    fn default_device() -> String {
        "default".to_string()
    }

    fn default_sample_rate() -> u32 {
        16000
    }
}

/// Lifecycle hooks configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Whether lifecycle hooks are enabled
    #[serde(default = "HooksConfig::default_enabled")]
    pub enabled: bool,
    /// Whether the built-in metrics hook is enabled
    #[serde(default = "HooksConfig::default_metrics_hook")]
    pub metrics_hook: bool,
    /// Whether the built-in audit log hook is enabled
    #[serde(default)]
    pub audit_log_hook: bool,
    /// Path to audit log file (default: <settings_dir>/logs/audit.log)
    #[serde(default)]
    pub audit_log_path: Option<PathBuf>,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            metrics_hook: true,
            audit_log_hook: false,
            audit_log_path: None,
        }
    }
}

impl HooksConfig {
    fn default_enabled() -> bool {
        true
    }

    fn default_metrics_hook() -> bool {
        true
    }
}

/// WebAuthn/Passkey authentication configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebAuthnConfig {
    /// Whether WebAuthn authentication is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Relying Party ID (domain, e.g., "localhost" or "example.com")
    #[serde(default = "WebAuthnConfig::default_rp_id")]
    pub rp_id: String,
    /// Relying Party origin (full URL, e.g., "https://localhost:8443")
    #[serde(default = "WebAuthnConfig::default_rp_origin")]
    pub rp_origin: String,
}

impl Default for WebAuthnConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rp_id: Self::default_rp_id(),
            rp_origin: Self::default_rp_origin(),
        }
    }
}

impl WebAuthnConfig {
    fn default_rp_id() -> String {
        "localhost".to_string()
    }

    fn default_rp_origin() -> String {
        "https://localhost:8443".to_string()
    }
}

/// DM pairing security configuration for messenger authorization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingConfig {
    /// Whether pairing security is enabled for messengers
    #[serde(default = "PairingConfig::default_enabled")]
    pub enabled: bool,
    /// Whether to require pairing codes (if false, auto-approve all senders)
    #[serde(default = "PairingConfig::default_require_code")]
    pub require_code: bool,
    /// Pairing code expiry in seconds (default: 300 = 5 minutes)
    #[serde(default = "PairingConfig::default_code_expiry")]
    pub code_expiry_secs: u64,
}

impl Default for PairingConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default for backwards compatibility
            require_code: true,
            code_expiry_secs: Self::default_code_expiry(),
        }
    }
}

impl PairingConfig {
    fn default_enabled() -> bool {
        false
    }

    fn default_require_code() -> bool {
        true
    }

    fn default_code_expiry() -> u64 {
        300 // 5 minutes
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Root state directory (e.g. `~/.rustyclaw`).
    /// All other paths are derived from this unless explicitly overridden.
    pub settings_dir: PathBuf,
    /// Path to SOUL.md file (default: `<workspace_dir>/SOUL.md`)
    pub soul_path: Option<PathBuf>,
    /// Skills directory (default: `<workspace_dir>/skills`)
    pub skills_dir: Option<PathBuf>,
    /// Agent workspace directory (default: `<settings_dir>/workspace`)
    pub workspace_dir: Option<PathBuf>,
    /// Credentials directory (default: `<settings_dir>/credentials`)
    pub credentials_dir: Option<PathBuf>,
    /// Messenger configurations
    pub messengers: Vec<MessengerConfig>,
    /// Whether to use secrets storage
    pub use_secrets: bool,
    /// Gateway WebSocket URL for the TUI to connect to
    #[serde(default)]
    pub gateway_url: Option<String>,
    /// Selected model provider and default model
    #[serde(default)]
    pub model: Option<ModelProvider>,
    /// Whether the secrets vault is encrypted with a user password
    /// (as opposed to an auto-generated key file).
    #[serde(default)]
    pub secrets_password_protected: bool,
    /// Whether TOTP two-factor authentication is enabled for the vault.
    #[serde(default)]
    pub totp_enabled: bool,
    /// Whether the agent is allowed to access secrets on behalf of the user.
    #[serde(default)]
    pub agent_access: bool,
    /// User-chosen name for this agent instance (shown in TUI title,
    /// authenticator app labels, etc.).  Defaults to "RustyClaw".
    #[serde(default = "Config::default_agent_name")]
    pub agent_name: String,
    /// Number of blank lines inserted between messages in the TUI.
    /// Set to 0 for compact output, 1 (default) for comfortable spacing.
    #[serde(default = "Config::default_message_spacing")]
    pub message_spacing: u16,
    /// Number of spaces a tab character occupies in the TUI.
    /// Defaults to 5.
    #[serde(default = "Config::default_tab_width")]
    pub tab_width: u16,
    /// Sandbox configuration for agent isolation.
    #[serde(default)]
    pub sandbox: SandboxConfig,
    /// SSRF protection configuration.
    #[serde(default)]
    pub ssrf: SsrfConfig,
    /// Prompt injection defense configuration.
    #[serde(default)]
    pub prompt_guard: PromptGuardConfig,
    /// TLS configuration for the gateway.
    #[serde(default)]
    pub tls: TlsConfig,
    /// Prometheus metrics configuration.
    #[serde(default)]
    pub metrics: MetricsConfig,
    /// Health check endpoint configuration.
    #[serde(default)]
    pub health: HealthConfig,
    /// Voice features configuration (STT/TTS).
    #[serde(default)]
    pub voice: VoiceConfig,
    /// Lifecycle hooks configuration.
    #[serde(default)]
    pub hooks: HooksConfig,
    /// WebAuthn/Passkey authentication configuration.
    #[serde(default)]
    pub webauthn: WebAuthnConfig,
    /// DM pairing security configuration for messengers.
    #[serde(default)]
    pub pairing: PairingConfig,
    /// ClawHub registry URL (default: `https://registry.clawhub.dev/api/v1`).
    #[serde(default)]
    pub clawhub_url: Option<String>,
    /// ClawHub API token for publishing / authenticated downloads.
    #[serde(default)]
    pub clawhub_token: Option<String>,
    /// System prompt for the agent (used for messenger conversations).
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Messenger polling interval in milliseconds (default: 2000).
    #[serde(default)]
    pub messenger_poll_interval_ms: Option<u32>,
}

/// Configuration for a messenger backend.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessengerConfig {
    /// Display name for this messenger instance.
    #[serde(default)]
    pub name: String,
    /// Messenger type: telegram, discord, slack, whatsapp, google-chat,
    /// teams, mattermost, irc, xmpp, signal, matrix, webhook, gmail.
    #[serde(default)]
    pub messenger_type: String,
    /// Whether this messenger is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Path to external config file (optional).
    #[serde(default)]
    pub config_path: Option<PathBuf>,
    /// Bot/API token (Telegram, Discord).
    #[serde(default)]
    pub token: Option<String>,
    /// Webhook URL (for webhook messenger).
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Optional API base URL for messenger adapters.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Optional API version (for APIs like WhatsApp Cloud).
    #[serde(default)]
    pub api_version: Option<String>,
    /// Default channel/space/room identifier.
    #[serde(default)]
    pub channel_id: Option<String>,
    /// Team/workspace identifier (Teams).
    #[serde(default)]
    pub team_id: Option<String>,
    /// WhatsApp Cloud API phone number ID.
    #[serde(default)]
    pub phone_number_id: Option<String>,
    /// Google Chat space id/name (e.g., spaces/AAAA...).
    #[serde(default)]
    pub space: Option<String>,
    /// Generic sender id/JID (XMPP bridge mode).
    #[serde(default)]
    pub from: Option<String>,
    /// Matrix homeserver URL.
    #[serde(default)]
    pub homeserver: Option<String>,
    /// Matrix user ID (@user:homeserver).
    #[serde(default)]
    pub user_id: Option<String>,
    /// Password (Matrix).
    #[serde(default)]
    pub password: Option<String>,
    /// Access token (Matrix).
    #[serde(default)]
    pub access_token: Option<String>,
    /// Phone number (Signal).
    #[serde(default)]
    pub phone: Option<String>,
    /// IRC server hostname.
    #[serde(default)]
    pub server: Option<String>,
    /// IRC server port.
    #[serde(default)]
    pub port: Option<u16>,
    /// IRC nickname.
    #[serde(default)]
    pub nickname: Option<String>,
    /// IRC username.
    #[serde(default)]
    pub username: Option<String>,
    /// IRC real name.
    #[serde(default)]
    pub realname: Option<String>,
    /// Default recipient/JID/channel for adapters.
    #[serde(default)]
    pub default_recipient: Option<String>,
    /// Gmail client ID (OAuth2).
    #[serde(default)]
    pub client_id: Option<String>,
    /// Gmail client secret (OAuth2).
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Gmail refresh token (OAuth2).
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Gmail user/email (defaults to "me").
    #[serde(default)]
    pub gmail_user: Option<String>,
    /// Gmail label to monitor (defaults to "INBOX").
    #[serde(default)]
    pub gmail_label: Option<String>,
    /// Gmail poll interval in seconds (defaults to 60).
    #[serde(default)]
    pub gmail_poll_interval: Option<u64>,
    /// Gmail: only respond to unread messages.
    #[serde(default)]
    pub gmail_unread_only: Option<bool>,
    /// Allowed chat IDs/channels (whitelist).
    #[serde(default)]
    pub allowed_chats: Vec<String>,
    /// Allowed user IDs (whitelist).
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            settings_dir: home_dir.join(".rustyclaw"),
            soul_path: None,
            skills_dir: None,
            workspace_dir: None,
            credentials_dir: None,
            messengers: Vec::new(),
            use_secrets: true,
            gateway_url: None,
            model: None,
            secrets_password_protected: false,
            totp_enabled: false,
            agent_access: false,
            agent_name: Self::default_agent_name(),
            message_spacing: Self::default_message_spacing(),
            tab_width: Self::default_tab_width(),
            sandbox: SandboxConfig::default(),
            ssrf: SsrfConfig::default(),
            prompt_guard: PromptGuardConfig::default(),
            tls: TlsConfig::default(),
            health: HealthConfig::default(),
            voice: VoiceConfig::default(),
            metrics: MetricsConfig::default(),
            hooks: HooksConfig::default(),
            webauthn: WebAuthnConfig::default(),
            pairing: PairingConfig::default(),
            clawhub_url: None,
            clawhub_token: None,
            system_prompt: None,
            messenger_poll_interval_ms: None,
        }
    }
}

impl Config {
    fn default_agent_name() -> String {
        "RustyClaw".to_string()
    }

    fn default_message_spacing() -> u16 {
        1
    }

    fn default_tab_width() -> u16 {
        5
    }

    // ── Derived path helpers (mirrors openclaw layout) ───────────

    /// Agent workspace directory — holds SOUL.md, skills/, etc.
    /// Default: `<settings_dir>/workspace`
    pub fn workspace_dir(&self) -> PathBuf {
        self.workspace_dir
            .clone()
            .unwrap_or_else(|| self.settings_dir.join("workspace"))
    }

    /// Credentials directory — holds secrets vault, key file, OAuth tokens.
    /// Default: `<settings_dir>/credentials`
    pub fn credentials_dir(&self) -> PathBuf {
        self.credentials_dir
            .clone()
            .unwrap_or_else(|| self.settings_dir.join("credentials"))
    }

    /// Default agent directory — per-agent state (sessions, etc.).
    /// Default: `<settings_dir>/agents/main`
    pub fn agent_dir(&self) -> PathBuf {
        self.settings_dir.join("agents").join("main")
    }

    /// Sessions directory for the default agent.
    pub fn sessions_dir(&self) -> PathBuf {
        self.agent_dir().join("sessions")
    }

    /// Path to SOUL.md — inside the workspace.
    pub fn soul_path(&self) -> PathBuf {
        self.soul_path
            .clone()
            .unwrap_or_else(|| self.workspace_dir().join("SOUL.md"))
    }

    /// Skills directory — inside the workspace.
    pub fn skills_dir(&self) -> PathBuf {
        self.skills_dir
            .clone()
            .unwrap_or_else(|| self.workspace_dir().join("skills"))
    }

    /// Logs directory.
    pub fn logs_dir(&self) -> PathBuf {
        self.settings_dir.join("logs")
    }

    /// Ensure the entire directory skeleton exists on disk.
    pub fn ensure_dirs(&self) -> Result<()> {
        let dirs = [
            self.settings_dir.clone(),
            self.workspace_dir(),
            self.credentials_dir(),
            self.agent_dir(),
            self.sessions_dir(),
            self.skills_dir(),
            self.logs_dir(),
        ];
        for d in &dirs {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    // ── Load / save ─────────────────────────────────────────────────

    /// Load configuration from file, with OpenClaw compatibility
    pub fn load(path: Option<PathBuf>) -> Result<Self> {
        let config_path = if let Some(p) = path {
            p
        } else {
            let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home_dir.join(".rustyclaw").join("config.toml")
        };

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let mut config: Config = toml::from_str(&content)?;
            // Migrate legacy flat layout if detected.
            config.migrate_legacy_layout()?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Save configuration to file
    pub fn save(&self, path: Option<PathBuf>) -> Result<()> {
        let config_path = if let Some(p) = path {
            p
        } else {
            self.settings_dir.join("config.toml")
        };

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    // ── Legacy migration ────────────────────────────────────────────

    /// Detect the pre-restructure flat layout and move files into the
    /// new openclaw-compatible directory hierarchy.
    fn migrate_legacy_layout(&mut self) -> Result<()> {
        let old_secrets = self.settings_dir.join("secrets.json");
        let old_key = self.settings_dir.join("secrets.key");
        let old_soul = self.settings_dir.join("SOUL.md");
        let old_skills = self.settings_dir.join("skills");

        // Only migrate if at least one legacy file exists AND the new
        // directories have not been created yet.
        let new_creds = self.credentials_dir();
        let new_workspace = self.workspace_dir();

        let has_legacy = old_secrets.exists() || old_soul.exists();
        let already_migrated = new_creds.join("secrets.json").exists()
            || new_workspace.join("SOUL.md").exists();

        if !has_legacy || already_migrated {
            return Ok(());
        }

        eprintln!("Migrating ~/.rustyclaw to new directory layout…");

        // Create target dirs.
        std::fs::create_dir_all(&new_creds)?;
        std::fs::create_dir_all(&new_workspace)?;

        // Move secrets vault → credentials/
        if old_secrets.exists() {
            let dest = new_creds.join("secrets.json");
            std::fs::rename(&old_secrets, &dest)?;
            eprintln!("  secrets.json → credentials/secrets.json");
        }
        if old_key.exists() {
            let dest = new_creds.join("secrets.key");
            std::fs::rename(&old_key, &dest)?;
            eprintln!("  secrets.key  → credentials/secrets.key");
        }

        // Move SOUL.md → workspace/
        if old_soul.exists() {
            let dest = new_workspace.join("SOUL.md");
            std::fs::rename(&old_soul, &dest)?;
            eprintln!("  SOUL.md      → workspace/SOUL.md");
        }

        // Move skills/ → workspace/skills/
        if old_skills.exists() && old_skills.is_dir() {
            let dest = new_workspace.join("skills");
            if !dest.exists() {
                std::fs::rename(&old_skills, &dest)?;
                eprintln!("  skills/      → workspace/skills/");
            }
        }

        // Update any explicit paths in config that pointed at the old locations.
        if self.soul_path.as_ref() == Some(&self.settings_dir.join("SOUL.md")) {
            self.soul_path = None; // let the helper derive it
        }
        if self.skills_dir.as_ref() == Some(&self.settings_dir.join("skills")) {
            self.skills_dir = None;
        }

        // Persist the updated config so we don't migrate again.
        self.save(None)?;

        eprintln!("Migration complete.");
        Ok(())
    }
}

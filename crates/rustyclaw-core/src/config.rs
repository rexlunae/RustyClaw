use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::memory_flush::MemoryFlushConfig;
use crate::services::ServiceDef;
use crate::workspace_context::WorkspaceContextConfig;

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
    /// Sandbox mode: "none", "path", "bwrap", "landlock"
    #[serde(default)]
    pub mode: String,
    /// Additional paths to deny (beyond credentials dir)
    #[serde(default)]
    pub deny_paths: Vec<PathBuf>,
    /// Paths to allow in strict mode
    #[serde(default)]
    pub allow_paths: Vec<PathBuf>,
}

/// SSH transport mode for the gateway.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum SshMode {
    /// Dedicated SSH port.
    #[default]
    Standalone,
    /// OpenSSH subsystem.
    Subsystem,
}

/// SSH transport configuration for the gateway.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SshGatewayConfig {
    /// Whether the SSH transport is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Mode: standalone (dedicated SSH port) or subsystem (OpenSSH subsystem).
    #[serde(default)]
    pub mode: SshMode,
    /// Bind address for standalone mode (e.g., "0.0.0.0:2222").
    #[serde(default = "SshGatewayConfig::default_bind")]
    pub bind: String,
    /// Path to the SSH host key file.
    /// Default: `<settings_dir>/ssh_host_key`
    #[serde(default)]
    pub host_key: Option<PathBuf>,
    /// Path to the authorized_clients file.
    /// Default: `<settings_dir>/authorized_clients`
    #[serde(default)]
    pub authorized_keys: Option<PathBuf>,
}

impl SshGatewayConfig {
    fn default_bind() -> String {
        "0.0.0.0:2222".to_string()
    }

    /// Resolve the host key path, falling back to `<settings_dir>/ssh_host_key`.
    pub fn host_key_path(&self, settings_dir: &std::path::Path) -> PathBuf {
        self.host_key
            .clone()
            .unwrap_or_else(|| settings_dir.join("ssh_host_key"))
    }

    /// Resolve the authorized_clients path, falling back to `<settings_dir>/authorized_clients`.
    pub fn authorized_keys_path(&self, settings_dir: &std::path::Path) -> PathBuf {
        self.authorized_keys
            .clone()
            .unwrap_or_else(|| settings_dir.join("authorized_clients"))
    }
}

/// How the gateway decides whether an incoming messenger message is worth a
/// full processing cycle.
///
/// This is the *rule* tier of the message relevance filter. The LLM
/// classifier tier (the `"smart"` mode from the original proposal) needs a
/// one-shot completion helper on `ModelContext` and is tracked as a follow-up
/// in RustyClaw#165; it will extend this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelevanceFilter {
    /// Process every incoming message (the historic default).
    #[default]
    Always,
    /// In group chats, process only messages that mention the agent by name
    /// or reply to a message the agent sent. Direct messages always count.
    Mentions,
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
    #[serde(default)]
    pub messengers: Vec<MessengerConfig>,
    /// Bindings from messenger channels to gateway threads.
    ///
    /// Kept flat rather than nested under each messenger because it is edited
    /// as one table — "which thread does this channel talk to" is a question
    /// asked across accounts, not within one.
    #[serde(default)]
    pub messenger_routes: Vec<crate::messengers::setup::ThreadRoute>,
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
    /// How incoming messenger messages are filtered before processing.
    ///
    /// `"always"` (the default) processes everything; `"mentions"` skips
    /// group-chat messages that neither mention the agent by name nor reply
    /// to a message the agent sent. Direct messages always pass.
    #[serde(default)]
    pub relevance_filter: RelevanceFilter,
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
    /// Maximum concurrent message processing tasks (default: 1 = sequential).
    /// Set to 2+ to allow agent to handle new messages while processing others.
    #[serde(default)]
    pub messenger_max_concurrent: Option<usize>,
    /// Per-tool permission overrides. Tools not listed here default to Allow.
    #[serde(default)]
    pub tool_permissions: HashMap<String, crate::tools::ToolPermission>,
    /// Rate and concurrency budgets applied to tool use, per caller.
    #[serde(default)]
    pub tool_limits: crate::tool_limits::ToolLimitsConfig,
    /// Path to TLS certificate file (PEM) for WSS gateway connections.
    #[serde(default)]
    pub tls_cert: Option<PathBuf>,
    /// Path to TLS private key file (PEM) for WSS gateway connections.
    #[serde(default)]
    pub tls_key: Option<PathBuf>,
    /// SSH transport configuration for the gateway.
    #[serde(default)]
    pub ssh: Option<SshGatewayConfig>,
    /// Pre-compaction memory flush configuration.
    #[serde(default)]
    pub memory_flush: MemoryFlushConfig,
    /// Workspace context injection configuration.
    #[serde(default)]
    pub workspace_context: WorkspaceContextConfig,
    /// Managed backend services.
    #[serde(default)]
    pub services: HashMap<String, ServiceDef>,
    /// Local inference engine configurations.
    #[serde(default)]
    pub engines: HashMap<String, crate::engines::EngineConfig>,
    /// User-defined model providers (e.g. self-hosted OpenAI-compatible
    /// servers).  Registered into the provider catalogue on load so they
    /// appear alongside built-in providers in every selector.
    #[serde(default)]
    pub custom_providers: Vec<crate::providers::CustomProviderConfig>,
    /// MCP server connections (`[mcp.servers.<name>]`). Connected by the
    /// gateway at startup when built with the `mcp` feature.
    #[serde(default)]
    pub mcp: crate::mcp::McpConfig,
}

/// Configuration for a messenger backend.
///
/// `Debug` is implemented by hand rather than derived so credentials are
/// redacted; see the impl below.
///
/// `PartialEq` lets the messenger loop notice a setup-panel edit — it
/// compares the shared config's account list against the one its
/// connections were built from on each poll tick.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MessengerConfig {
    /// Display name for this messenger instance.
    #[serde(default)]
    pub name: String,
    /// Messenger type: telegram, discord, signal, matrix, webhook.
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
    /// Allowed chat IDs/channels (whitelist).
    #[serde(default)]
    pub allowed_chats: Vec<String>,
    /// Allowed user IDs (whitelist).
    #[serde(default)]
    pub allowed_users: Vec<String>,

    // ── Slack-specific ─────────────────────────────────────────────────
    /// Slack app-level token for Socket Mode (optional).
    #[serde(default)]
    pub app_token: Option<String>,
    /// Default channel to listen on (Slack).
    #[serde(default)]
    pub default_channel: Option<String>,

    // ── IRC-specific ───────────────────────────────────────────────────
    /// IRC server hostname.
    #[serde(default)]
    pub server: Option<String>,
    /// Port number (IRC, BlueBubbles, etc.).
    #[serde(default)]
    pub port: Option<u16>,
    /// Nickname (IRC).
    #[serde(default)]
    pub nick: Option<String>,
    /// IRC channels to join.
    #[serde(default)]
    pub irc_channels: Vec<String>,
    /// Whether to use TLS (IRC).
    #[serde(default)]
    pub use_tls: Option<bool>,

    // ── Google Chat-specific ───────────────────────────────────────────
    /// Service account credentials path (Google Chat).
    #[serde(default)]
    pub credentials_path: Option<String>,
    /// Google Chat spaces to listen on.
    #[serde(default)]
    pub spaces: Vec<String>,

    // ── Teams-specific ─────────────────────────────────────────────────
    /// Bot Framework app ID (Teams).
    #[serde(default)]
    pub app_id: Option<String>,
    /// Bot Framework app password (Teams).
    #[serde(default)]
    pub app_password: Option<String>,

    // ── DM pairing security ────────────────────────────────────────────
    /// Pairing code required before accepting DMs from unknown users.
    #[serde(default)]
    pub pairing_code: Option<String>,
    /// Whether DM pairing is required for this messenger.
    #[serde(default)]
    pub require_pairing: bool,
    /// List of already-paired user IDs.
    #[serde(default)]
    pub paired_users: Vec<String>,

    // ── DM configuration (Matrix, etc.) ────────────────────────────────
    /// DM handling configuration.
    #[serde(default)]
    pub dm: Option<DmConfig>,

    // ── Vault-backed credentials ───────────────────────────────────────
    /// Vault credential names backing this account's secret fields, keyed by
    /// field name (`token`, `password`, …).
    ///
    /// A field listed here is read from the vault at connect time and its
    /// plaintext twin above is ignored, which is how a credential stops living
    /// in `config.toml`. Fields absent from this map fall back to the
    /// plaintext value, so configs written before the vault existed keep
    /// working — see [`crate::messengers::setup::plaintext_fields`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secret_refs: BTreeMap<String, String>,

    // ── Presented identity ─────────────────────────────────────────────
    /// How the agent introduces itself on this messenger. Unset fields fall
    /// back to the agent's own name and description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<crate::messengers::setup::MessengerProfile>,
}

impl std::fmt::Debug for MessengerConfig {
    /// Redacts every credential field.
    ///
    /// `MessengerConfig` is logged on initialization failure, and a derived
    /// `Debug` would put a live bot token in the log. The non-secret fields are
    /// what make a failure diagnosable, so they are kept.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        /// Distinguishes "no credential" from "a credential we are not
        /// printing" — those are very different failures to be looking at.
        fn redact(value: &Option<String>) -> &'static str {
            match value {
                Some(_) => "***",
                None => "None",
            }
        }
        f.debug_struct("MessengerConfig")
            .field("name", &self.name)
            .field("messenger_type", &self.messenger_type)
            .field("enabled", &self.enabled)
            .field("config_path", &self.config_path)
            .field("token", &redact(&self.token))
            .field("webhook_url", &redact(&self.webhook_url))
            .field("homeserver", &self.homeserver)
            .field("user_id", &self.user_id)
            .field("password", &redact(&self.password))
            .field("access_token", &redact(&self.access_token))
            .field("phone", &redact(&self.phone))
            .field("allowed_chats", &self.allowed_chats)
            .field("allowed_users", &self.allowed_users)
            .field("app_token", &redact(&self.app_token))
            .field("default_channel", &self.default_channel)
            .field("server", &self.server)
            .field("port", &self.port)
            .field("nick", &self.nick)
            .field("irc_channels", &self.irc_channels)
            .field("use_tls", &self.use_tls)
            .field("credentials_path", &self.credentials_path)
            .field("spaces", &self.spaces)
            .field("app_id", &self.app_id)
            .field("app_password", &redact(&self.app_password))
            .field("pairing_code", &redact(&self.pairing_code))
            .field("require_pairing", &self.require_pairing)
            .field("paired_users", &self.paired_users)
            .field("dm", &self.dm)
            .field("secret_refs", &self.secret_refs.keys().collect::<Vec<_>>())
            .field("profile", &self.profile)
            .finish()
    }
}

/// DM (Direct Message) configuration for messengers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DmConfig {
    /// Whether DMs are enabled.
    #[serde(default)]
    pub enabled: bool,
    /// DM policy: "allowlist" (only allow_from users), "open" (anyone), "pairing" (require code).
    #[serde(default)]
    pub policy: Option<String>,
    /// List of user IDs allowed to send DMs (for "allowlist" policy).
    #[serde(default)]
    pub allow_from: Vec<String>,
}

pub(crate) fn default_true() -> bool {
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
            messenger_routes: Vec::new(),
            use_secrets: true,
            gateway_url: None,
            model: None,
            secrets_password_protected: false,
            totp_enabled: false,
            agent_access: false,
            agent_name: Self::default_agent_name(),
            relevance_filter: RelevanceFilter::Always,
            message_spacing: Self::default_message_spacing(),
            tab_width: Self::default_tab_width(),
            sandbox: SandboxConfig::default(),
            clawhub_url: None,
            clawhub_token: None,
            system_prompt: None,
            messenger_poll_interval_ms: None,
            messenger_max_concurrent: None,
            tool_permissions: HashMap::new(),
            tool_limits: crate::tool_limits::ToolLimitsConfig::default(),
            tls_cert: None,
            tls_key: None,
            ssh: None,
            memory_flush: MemoryFlushConfig::default(),
            workspace_context: WorkspaceContextConfig::default(),
            services: HashMap::new(),
            engines: HashMap::new(),
            custom_providers: Vec::new(),
            mcp: crate::mcp::McpConfig::default(),
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

    /// Root directory holding one subdirectory per agent.
    /// Default: `<settings_dir>/agents`
    pub fn agents_root(&self) -> PathBuf {
        self.settings_dir.join("agents")
    }

    /// Default agent directory — per-agent state (sessions, etc.).
    /// Default: `<settings_dir>/agents/main`
    pub fn agent_dir(&self) -> PathBuf {
        self.agent_dir_for(crate::agents::MAIN_AGENT_ID)
    }

    /// Directory for a specific agent's state.
    /// `<settings_dir>/agents/<agent_id>`
    pub fn agent_dir_for(&self, agent_id: &str) -> PathBuf {
        self.agents_root().join(agent_id)
    }

    /// Sessions directory for the default agent.
    pub fn sessions_dir(&self) -> PathBuf {
        self.sessions_dir_for(crate::agents::MAIN_AGENT_ID)
    }

    /// Sessions directory for a specific agent.
    pub fn sessions_dir_for(&self, agent_id: &str) -> PathBuf {
        self.agent_dir_for(agent_id).join("sessions")
    }

    /// Workspace directory for a specific agent. The `main` agent keeps the
    /// installation-wide workspace (`workspace_dir()`); other agents get a
    /// private workspace inside their agent directory so each can carry its
    /// own SOUL.md, skills, and files.
    pub fn workspace_dir_for(&self, agent_id: &str) -> PathBuf {
        if agent_id == crate::agents::MAIN_AGENT_ID {
            self.workspace_dir()
        } else {
            self.agent_dir_for(agent_id).join("workspace")
        }
    }

    /// Agent registry rooted at this config's settings dir.
    pub fn agent_registry(&self) -> crate::agents::AgentRegistry {
        crate::agents::AgentRegistry::new(&self.settings_dir, &self.agent_name)
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

    /// Returns all skills directories in priority order (lowest to highest).
    /// Order: bundled OpenClaw → user OpenClaw → user RustyClaw
    pub fn skills_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        // OpenClaw bundled skills (npm global install)
        let openclaw_bundled = PathBuf::from("/usr/lib/node_modules/openclaw/skills");
        if openclaw_bundled.exists() {
            dirs.push(openclaw_bundled);
        }

        // OpenClaw user skills
        if let Some(home) = dirs::home_dir() {
            let openclaw_user = home.join(".openclaw/workspace/skills");
            if openclaw_user.exists() {
                dirs.push(openclaw_user);
            }
        }

        // RustyClaw user skills (highest priority)
        dirs.push(self.skills_dir());

        dirs
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

    /// A default config whose `settings_dir` is anchored at `config_path`'s
    /// parent directory (falling back to the home default when there is no
    /// parent). Used when a config file is corrupt or missing: the user's
    /// state — and boot.toml — lives next to the config file they asked
    /// for, not necessarily in the home directory.
    fn anchored_default(config_path: &Path) -> Self {
        let mut fallback = Config::default();
        if let Some(parent) = config_path.parent() {
            if !parent.as_os_str().is_empty() {
                fallback.settings_dir = parent.to_path_buf();
            }
        }
        fallback
    }

    /// Load configuration from file, with OpenClaw compatibility
    pub fn load(path: Option<PathBuf>) -> Result<Self> {
        let config_path = if let Some(p) = path {
            p
        } else {
            let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home_dir.join(".rustyclaw").join("config.toml")
        };

        let (mut config, skip_post) = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            match toml::from_str(&content) {
                Ok(c) => (c, false),
                Err(e) => {
                    // A torn config used to be a hard startup failure — the
                    // one file that decides where everything lives, and one
                    // bad byte meant nothing started until the user found
                    // and fixed it by hand. Move it aside and start from
                    // defaults instead: RustyClaw comes up, and the user's
                    // settings are in the quarantine file, not gone.
                    eprintln!(
                        "ERROR: Failed to parse config at {}: {e}",
                        config_path.display()
                    );
                    match crate::persist::quarantine(&config_path, &e.to_string()) {
                        Some(kept) => {
                            eprintln!(
                                "The unreadable config was moved to {} — starting with defaults. \
                                 Restore settings from that file, then delete it.",
                                kept.display()
                            );
                            // Anchor the default at the corrupt file's own
                            // directory, not the home default: that is where
                            // the user's state — and boot.toml — actually
                            // lives, especially when --config pointed at a
                            // custom location. Skip the legacy-layout
                            // migration and provider-catalogue reset (there
                            // are no user settings to migrate — running them
                            // on a defaults config could relocate legacy
                            // files using paths the user never chose, and a
                            // migration failure would turn this recoverable
                            // path back into a hard startup failure).
                            (Self::anchored_default(&config_path), true)
                        }
                        // Could not even rename it: better to stop than to
                        // silently shadow a file the next save would clobber.
                        None => return Err(e.into()),
                    }
                }
            }
        } else {
            // Same anchoring for a *missing* config file: when --config
            // pointed at a path that does not exist (first run, wrong dir),
            // boot.toml must still be looked up next to it, not in the
            // home directory.
            (Self::anchored_default(&config_path), false)
        };
        if !skip_post {
            // Migrate legacy flat layout if detected.
            config.migrate_legacy_layout()?;
            // Make user-defined providers visible to the provider catalogue.
            crate::providers::set_custom_providers(&config.custom_providers);
        }

        // Boot config (RustyClaw#175, migration rung 1): the stable,
        // boot-critical slice lives in `<settings_dir>/boot.toml` so a
        // broken extended config cannot take the LLM connection down with
        // it. A missing file is legacy single-file behavior (rung 2 — boot
        // params already come from config.toml); a present-but-unfixable
        // file is fatal, with deterministic recovery attempted first.
        let boot = crate::boot_config::BootConfig::load(&config.settings_dir.join("boot.toml"))?;
        boot.apply(&mut config);
        Ok(config)
    }

    /// Save configuration to file
    pub fn save(&self, path: Option<PathBuf>) -> Result<()> {
        let config_path = if let Some(p) = path {
            p
        } else {
            self.settings_dir.join("config.toml")
        };

        // Atomic: this file is rewritten on essentially every settings
        // change, and a torn write here used to be a failed startup.
        let content = toml::to_string_pretty(self)?;
        crate::persist::write_atomically(&config_path, content.as_bytes())?;

        // Keep the runtime provider catalogue in sync with edits made
        // through the UI (add/remove custom provider then save).
        crate::providers::set_custom_providers(&self.custom_providers);
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
        let already_migrated =
            new_creds.join("secrets.json").exists() || new_workspace.join("SOUL.md").exists();

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A torn config.toml used to be a hard startup failure — the single
    /// file that decides where everything lives, dead on one bad byte,
    /// with no backup and no fallback. Load must quarantine it and come
    /// up with defaults so RustyClaw starts and the user keeps the bytes.
    #[test]
    fn a_corrupt_config_is_quarantined_and_defaults_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"[provider\nthis is torn toml").unwrap();

        let config = Config::load(Some(path.clone())).expect("defaults must load");
        assert_eq!(config.agent_name, Config::default().agent_name);

        assert!(!path.exists(), "corrupt file should have been moved aside");
        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert_eq!(quarantined.len(), 1, "corrupt config kept for recovery");
    }

    /// Save must work into a directory that does not exist yet, and a
    /// saved config must round-trip.
    #[test]
    fn save_creates_directories_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/nested/config.toml");

        let config = Config::default();
        config.save(Some(path.clone())).expect("save must succeed");

        let loaded = Config::load(Some(path)).expect("load what was saved");
        assert_eq!(loaded.agent_name, config.agent_name);
    }

    /// The relevance filter defaults to the historic "process everything"
    /// behavior, and the rule tier opts in via `relevance_filter = "mentions"`.
    #[test]
    fn relevance_filter_serde_and_default() {
        assert_eq!(Config::default().relevance_filter, RelevanceFilter::Always);

        // The enum itself maps from the documented spellings.
        assert_eq!(
            serde_json::from_str::<RelevanceFilter>("\"always\"").unwrap(),
            RelevanceFilter::Always
        );
        assert_eq!(
            serde_json::from_str::<RelevanceFilter>("\"mentions\"").unwrap(),
            RelevanceFilter::Mentions
        );

        // A saved config round-trips the choice.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config {
            relevance_filter: RelevanceFilter::Mentions,
            ..Config::default()
        };
        config.save(Some(path.clone())).expect("save must succeed");
        let loaded = Config::load(Some(path)).expect("load what was saved");
        assert_eq!(loaded.relevance_filter, RelevanceFilter::Mentions);
    }

    // ── Boot config integration (RustyClaw#175) ──────────────────────

    /// A config.toml rooted at `dir` (settings_dir is required by serde,
    /// which also points the boot.toml lookup at the same temp dir).
    fn write_full_config(dir: &std::path::Path, extra: &str) -> std::path::PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            format!(
                "settings_dir = \"{}\"\nuse_secrets = true\n{extra}",
                dir.display()
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn no_boot_file_keeps_legacy_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_full_config(
            dir.path(),
            "[model]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
        );
        let config = Config::load(Some(path)).unwrap();
        let m = config.model.unwrap();
        assert_eq!(m.provider, "openai");
        assert_eq!(m.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn boot_config_overrides_extended_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_full_config(
            dir.path(),
            "[model]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
        );
        std::fs::write(
            dir.path().join("boot.toml"),
            "[provider]\nname = \"anthropic\"\nmodel = \"claude-sonnet-4-20250514\"\n",
        )
        .unwrap();
        let config = Config::load(Some(path)).unwrap();
        let m = config.model.unwrap();
        assert_eq!(m.provider, "anthropic", "boot provider wins");
        assert_eq!(m.model.as_deref(), Some("claude-sonnet-4-20250514"));
    }

    /// The resilience win of the split: a corrupt extended config degrades
    /// to defaults, but the boot slice still lands, so the gateway can
    /// still reach an LLM and self-heal config.toml.
    #[test]
    fn corrupt_extended_config_with_valid_boot_still_gets_boot_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "not valid toml at all").unwrap();
        std::fs::write(
            dir.path().join("boot.toml"),
            "[provider]\nname = \"ollama\"\n\n[workspace]\npath = \"/tmp/ws\"\n",
        )
        .unwrap();
        let config = Config::load(Some(dir.path().join("config.toml"))).unwrap();
        assert_eq!(config.model.unwrap().provider, "ollama");
        assert_eq!(
            config.workspace_dir,
            Some(std::path::PathBuf::from("/tmp/ws"))
        );
        assert!(
            !dir.path().join("config.toml").exists(),
            "corrupt extended config is quarantined"
        );
    }

    /// A fatal boot file must not silently boot with defaults: the caller
    /// gets a clear error instead.
    #[test]
    fn unreadable_boot_file_is_fatal_through_config_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_full_config(dir.path(), "");
        std::fs::write(
            dir.path().join("boot.toml"),
            "garbage that parses as nothing",
        )
        .unwrap();
        let err = Config::load(Some(path)).unwrap_err();
        assert!(err.to_string().contains("unreadable"), "{err}");
    }

    /// Regression for Devin review #442 (BUG_0002): a missing config file at
    /// a custom path must still find boot.toml next to it, instead of
    /// falling back to the home directory.
    #[test]
    fn boot_next_to_missing_custom_config_is_honored() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("custom/config.toml");
        std::fs::create_dir_all(dir.path().join("custom")).unwrap();
        std::fs::write(
            dir.path().join("custom/boot.toml"),
            "[provider]\nname = \"ollama\"\n",
        )
        .unwrap();
        let config = Config::load(Some(config_path)).unwrap();
        assert_eq!(config.model.unwrap().provider, "ollama");
        assert_eq!(config.settings_dir, dir.path().join("custom"));
    }
}

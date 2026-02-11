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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessengerConfig {
    pub name: String,
    pub enabled: bool,
    pub config_path: Option<PathBuf>,
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
        }
    }
}

impl Config {
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

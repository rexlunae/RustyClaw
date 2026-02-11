use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// OpenClaw-compatible settings directory
    pub settings_dir: PathBuf,
    /// Path to SOUL.md file
    pub soul_path: Option<PathBuf>,
    /// Skills directory
    pub skills_dir: Option<PathBuf>,
    /// Messenger configurations
    pub messengers: Vec<MessengerConfig>,
    /// Whether to use secrets storage
    pub use_secrets: bool,
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
            messengers: Vec::new(),
            use_secrets: true,
        }
    }
}

impl Config {
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
            let config: Config = toml::from_str(&content)?;
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
}

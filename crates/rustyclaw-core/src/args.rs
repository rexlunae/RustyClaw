use crate::config::Config;
use clap::{ArgAction, Args};
use std::path::PathBuf;

// Global flags shared across every subcommand.
//
// Mirrors openclaw's global options:
//   --profile <name>   Isolate state under ~/.rustyclaw-<name>
//   --no-color         Disable coloured terminal output
//   -c / --config      Path to a config.toml file
//   --settings-dir     Root state directory override
#[derive(Debug, Clone, Args)]
pub struct CommonArgs {
    /// Path to a config.toml file
    #[arg(
        short = 'c',
        long,
        value_name = "PATH",
        env = "RUSTYCLAW_CONFIG",
        global = true
    )]
    pub config: Option<PathBuf>,

    /// Settings directory (default: ~/.rustyclaw)
    #[arg(
        long,
        value_name = "DIR",
        env = "RUSTYCLAW_SETTINGS_DIR",
        global = true
    )]
    pub settings_dir: Option<PathBuf>,

    /// Isolate state under ~/.rustyclaw-<PROFILE>
    #[arg(long, value_name = "PROFILE", env = "RUSTYCLAW_PROFILE", global = true)]
    pub profile: Option<String>,

    /// Disable coloured terminal output
    #[arg(long = "no-color", action = ArgAction::SetTrue, env = "NO_COLOR", global = true)]
    pub no_color: bool,

    /// Path to SOUL.md
    #[arg(long, value_name = "PATH", env = "RUSTYCLAW_SOUL", global = true)]
    pub soul: Option<PathBuf>,

    /// Skills directory
    #[arg(
        long = "skills",
        value_name = "DIR",
        env = "RUSTYCLAW_SKILLS",
        global = true
    )]
    pub skills_dir: Option<PathBuf>,

    /// Disable secrets storage
    #[arg(long = "no-secrets", action = ArgAction::SetTrue, global = true)]
    pub no_secrets: bool,

    /// Gateway WebSocket URL (ws://…)
    #[arg(
        long = "gateway",
        value_name = "WS_URL",
        env = "RUSTYCLAW_GATEWAY",
        global = true
    )]
    pub gateway: Option<String>,
}

impl CommonArgs {
    /// Resolve the effective settings directory, honouring `--profile`.
    pub fn effective_settings_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.settings_dir {
            return Some(dir.clone());
        }
        if let Some(profile) = &self.profile {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            return Some(home.join(format!(".rustyclaw-{}", profile)));
        }
        None
    }

    pub fn config_path(&self) -> Option<PathBuf> {
        if let Some(config) = &self.config {
            return Some(config.clone());
        }

        if let Some(settings_dir) = self.effective_settings_dir() {
            return Some(settings_dir.join("config.toml"));
        }

        None
    }

    pub fn apply_overrides(&self, config: &mut Config) {
        if let Some(settings_dir) = self.effective_settings_dir() {
            config.settings_dir = settings_dir;
        }

        if let Some(soul) = &self.soul {
            config.soul_path = Some(soul.clone());
        }

        if let Some(skills_dir) = &self.skills_dir {
            config.skills_dir = Some(skills_dir.clone());
        }

        if self.no_secrets {
            config.use_secrets = false;
        }

        if let Some(gateway) = &self.gateway {
            config.gateway_url = Some(gateway.clone());
        }
    }
}

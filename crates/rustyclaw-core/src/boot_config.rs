//! Boot config — the minimal, stable slice of configuration needed to start
//! the gateway and reach an LLM.
//!
//! This is migration rungs 1–2 of RustyClaw#175: `boot.toml` carries the
//! boot-critical fields (provider, workspace, gateway bind) in a small flat
//! schema, while the extended `config.toml` keeps everything else. A broken
//! extended config degrades to defaults and the agent can self-heal; a
//! broken boot config is fatal after deterministic recovery attempts, because
//! without the boot fields there is no known provider or workspace and no
//! LLM yet to fix anything.
//!
//! ```toml
//! # <settings_dir>/boot.toml
//! [provider]
//! name = "anthropic"                 # provider id, as in [model] provider
//! model = "claude-sonnet-4-20250514" # optional; provider default if absent
//!
//! [workspace]
//! path = "~/.rustyclaw/workspace"    # optional; ~ is expanded
//!
//! [gateway]
//! ssh_bind = "0.0.0.0:2222"          # optional; for the SSH transport
//! ```
//!
//! API keys are *not* in boot.toml: they already resolve per-provider from
//! the vault or `*_API_KEY` env vars (see `crate::providers`).

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::config::{Config, ModelProvider, SshGatewayConfig};

/// The boot-critical fields, flat and fully optional.
///
/// `Default` is the "no boot.toml" state, which preserves the legacy
/// single-file behavior (the extended config already carries these fields).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootConfig {
    /// Provider id (e.g. "anthropic", "openai", "google", "ollama").
    pub provider: Option<String>,
    /// Default model name; provider default when absent.
    pub model: Option<String>,
    /// Workspace path; `~` is expanded at load time.
    pub workspace: Option<PathBuf>,
    /// SSH transport bind address (e.g. "0.0.0.0:2222").
    pub ssh_bind: Option<String>,
}

/// Nested TOML shape mirroring the documented sections.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BootFile {
    provider: ProviderSection,
    workspace: WorkspaceSection,
    gateway: GatewaySection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProviderSection {
    name: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WorkspaceSection {
    path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GatewaySection {
    ssh_bind: Option<String>,
}

impl BootConfig {
    /// Load `<settings_dir>/boot.toml`.
    ///
    /// Deterministic recovery ladder (issue addendum), in order:
    /// missing file → defaults (legacy single-file behavior); TOML parse
    /// failure → try JSON (wrong extension); invalid bind → drop with a
    /// warning; anything else unfixable → fatal, file quarantined so the
    /// next boot does not loop, with a message naming the quarantine path.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read boot config {}", path.display()))?;

        let file: Option<BootFile> = match toml::from_str(&content) {
            Ok(file) => Some(file),
            Err(toml_err) => match serde_json::from_str::<BootFile>(&content) {
                Ok(file) => {
                    tracing::warn!(
                        file = %path.display(),
                        "Boot config is JSON, not TOML — continuing (it will be expected as \
                         TOML next time; consider renaming the file)"
                    );
                    Some(file)
                }
                Err(_) => {
                    tracing::error!(
                        file = %path.display(),
                        %toml_err,
                        "Boot config failed to parse as TOML or JSON"
                    );
                    None
                }
            },
        };

        let Some(file) = file else {
            // All deterministic fixes exhausted. Without the boot fields we
            // do not know the provider or workspace, and there is no LLM yet
            // to self-heal: this is the one config that must be right.
            match crate::persist::quarantine(path, "unreadable boot config") {
                Some(kept) => bail!(
                    "Boot config {} is unreadable. It was moved to {} — restore the correct \
                     settings from that file, then restart the gateway.",
                    path.display(),
                    kept.display()
                ),
                None => bail!(
                    "Boot config {} is unreadable and could not be moved aside. Fix or remove \
                     it, then restart the gateway.",
                    path.display()
                ),
            }
        };

        let mut boot = BootConfig {
            provider: file.provider.name,
            model: file.provider.model,
            workspace: file.workspace.path,
            ssh_bind: file.gateway.ssh_bind,
        };

        // The gateway parses the bind as a SocketAddr at startup, so an
        // invalid value would fail later with a worse message. Drop it here
        // instead: the extended config or CLI can still supply a valid one.
        if let Some(bind) = &boot.ssh_bind {
            if bind.parse::<std::net::SocketAddr>().is_err() {
                tracing::warn!(
                    file = %path.display(),
                    %bind,
                    "Boot config has an invalid gateway.ssh_bind — ignoring it"
                );
                boot.ssh_bind = None;
            }
        }

        if let Some(workspace) = &boot.workspace {
            boot.workspace = Some(expand_tilde(workspace));
        }

        Ok(boot)
    }

    /// Apply boot-critical fields onto a loaded config.
    ///
    /// Boot wins over the extended config for the fields it sets (they are
    /// the stable, rarely-changed source of truth), while preserving details
    /// the boot schema does not carry — provided they still make sense.
    pub fn apply(&self, config: &mut Config) {
        if let Some(provider) = &self.provider {
            let existing = config.model.as_ref();
            // A custom `base_url` belongs to the provider it was configured
            // for. Carrying it across a provider switch would silently
            // redirect model traffic to the previous provider's endpoint, so
            // it is preserved only when the provider is unchanged.
            let same_provider = existing.is_some_and(|m| m.provider.eq_ignore_ascii_case(provider));
            let model = self
                .model
                .clone()
                .or_else(|| existing.and_then(|m| m.model.clone()));
            let base_url = if same_provider {
                existing.and_then(|m| m.base_url.clone())
            } else {
                None
            };
            config.model = Some(ModelProvider {
                provider: provider.clone(),
                model,
                base_url,
            });
        } else if let Some(model) = &self.model {
            // A model without a provider only makes sense on an existing one.
            if let Some(existing) = config.model.as_mut() {
                existing.model = Some(model.clone());
            }
        }

        if let Some(workspace) = &self.workspace {
            config.workspace_dir = Some(workspace.clone());
        }

        if let Some(bind) = &self.ssh_bind {
            match config.ssh.as_mut() {
                Some(ssh) => ssh.bind = bind.clone(),
                // The gateway only listens on the bind when SSH is enabled
                // and in standalone mode (main.rs), so creating a section
                // with the default `enabled: false` would silently ignore
                // the boot bind. Create an enabled standalone section.
                None => {
                    config.ssh = Some(SshGatewayConfig {
                        enabled: true,
                        mode: crate::config::SshMode::Standalone,
                        bind: bind.clone(),
                        host_key: None,
                        authorized_keys: None,
                    });
                }
            }
        }
    }
}

/// Expand a leading `~` to the user's home directory. Non-`~` paths and a
/// missing home directory pass through unchanged.
fn expand_tilde(path: &Path) -> PathBuf {
    let Some(first) = path.components().next() else {
        return path.to_path_buf();
    };
    let std::path::Component::Normal(first) = first else {
        return path.to_path_buf();
    };
    if first != "~" {
        return path.to_path_buf();
    }
    let Some(home) = dirs::home_dir() else {
        return path.to_path_buf();
    };
    let rest: PathBuf = path.components().skip(1).collect();
    home.join(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn missing_boot_file_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let boot = BootConfig::load(&dir.path().join("boot.toml")).unwrap();
        assert_eq!(boot, BootConfig::default());
    }

    #[test]
    fn toml_boot_file_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "boot.toml",
            r#"
[provider]
name = "anthropic"
model = "claude-sonnet-4-20250514"

[workspace]
path = "/srv/ws"

[gateway]
ssh_bind = "0.0.0.0:2222"
"#,
        );
        let boot = BootConfig::load(&path).unwrap();
        assert_eq!(boot.provider.as_deref(), Some("anthropic"));
        assert_eq!(boot.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(boot.workspace.as_deref(), Some(Path::new("/srv/ws")));
        assert_eq!(boot.ssh_bind.as_deref(), Some("0.0.0.0:2222"));
    }

    #[test]
    fn sections_are_optional() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "boot.toml", "[provider]\nname = \"ollama\"\n");
        let boot = BootConfig::load(&path).unwrap();
        assert_eq!(boot.provider.as_deref(), Some("ollama"));
        assert!(boot.model.is_none());
        assert!(boot.workspace.is_none());
        assert!(boot.ssh_bind.is_none());
    }

    #[test]
    fn json_is_accepted_as_a_wrong_extension_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "boot.toml",
            r#"{"provider": {"name": "openai", "model": "gpt-4o"}, "workspace": {"path": "/w"}}"#,
        );
        let boot = BootConfig::load(&path).unwrap();
        assert_eq!(boot.provider.as_deref(), Some("openai"));
        assert_eq!(boot.model.as_deref(), Some("gpt-4o"));
        assert_eq!(boot.workspace.as_deref(), Some(Path::new("/w")));
    }

    #[test]
    fn unparseable_boot_file_is_fatal_and_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "boot.toml", "this is neither toml nor json");
        let err = BootConfig::load(&path).unwrap_err();
        assert!(err.to_string().contains("unreadable"), "{err}");
        assert!(!path.exists(), "fatal boot file must be moved aside");
        let kept: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert_eq!(kept.len(), 1, "boot file kept for recovery");
    }

    #[test]
    fn invalid_ssh_bind_is_dropped_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "boot.toml",
            "[gateway]\nssh_bind = \"not-an-address\"\n",
        );
        let boot = BootConfig::load(&path).unwrap();
        assert!(boot.ssh_bind.is_none());
    }

    #[test]
    fn tilde_in_workspace_path_is_expanded() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "boot.toml",
            "[workspace]\npath = \"~/rusty/ws\"\n",
        );
        let boot = BootConfig::load(&path).unwrap();
        let home = dirs::home_dir().unwrap();
        assert_eq!(boot.workspace, Some(home.join("rusty/ws")));
    }

    #[test]
    fn apply_preserves_base_url_when_provider_is_unchanged() {
        let mut config = Config {
            model: Some(crate::config::ModelProvider {
                provider: "openai".into(),
                model: Some("gpt-4o".into()),
                base_url: Some("https://proxy.example".into()),
            }),
            workspace_dir: Some(PathBuf::from("/old")),
            ssh: None,
            ..Config::default()
        };
        let boot = BootConfig {
            provider: Some("openai".into()),
            model: Some("gpt-4o".into()),
            workspace: Some(PathBuf::from("/new")),
            ssh_bind: Some("0.0.0.0:2222".into()),
        };
        boot.apply(&mut config);

        let m = config.model.unwrap();
        assert_eq!(m.provider, "openai");
        assert_eq!(m.model.as_deref(), Some("gpt-4o"));
        assert_eq!(m.base_url.as_deref(), Some("https://proxy.example"));
        assert_eq!(config.workspace_dir, Some(PathBuf::from("/new")));
        let ssh = config.ssh.unwrap();
        assert_eq!(ssh.bind, "0.0.0.0:2222");
        // No ssh section existed: the boot bind creates an *enabled*
        // standalone section, not a disabled default that main.rs would
        // silently ignore.
        assert!(ssh.enabled, "boot ssh_bind must enable the SSH transport");
        assert_eq!(ssh.mode, crate::config::SshMode::Standalone);
    }

    /// Regression for Devin review #442 (BUG_0001 / SEC_0001): switching the
    /// provider in boot.toml must NOT carry the old provider's custom
    /// base_url over — that would silently redirect model traffic to the
    /// previous provider's endpoint.
    #[test]
    fn apply_clears_base_url_when_provider_switches() {
        let mut config = Config {
            model: Some(crate::config::ModelProvider {
                provider: "openai".into(),
                model: Some("gpt-4o".into()),
                base_url: Some("https://proxy.example".into()),
            }),
            ..Config::default()
        };
        let boot = BootConfig {
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet-4-20250514".into()),
            ..BootConfig::default()
        };
        boot.apply(&mut config);

        let m = config.model.unwrap();
        assert_eq!(m.provider, "anthropic");
        assert_eq!(m.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert!(
            m.base_url.is_none(),
            "provider switch must drop old base_url"
        );
    }

    #[test]
    fn apply_model_without_provider_attaches_to_existing() {
        let mut config = Config {
            model: Some(crate::config::ModelProvider {
                provider: "openai".into(),
                model: None,
                base_url: None,
            }),
            ..Config::default()
        };
        let boot = BootConfig {
            model: Some("gpt-4o".into()),
            ..BootConfig::default()
        };
        boot.apply(&mut config);
        assert_eq!(config.model.unwrap().model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn apply_on_default_config_sets_only_boot_fields() {
        let mut config = Config::default();
        let boot = BootConfig {
            provider: Some("ollama".into()),
            workspace: Some(PathBuf::from("/ws")),
            ..BootConfig::default()
        };
        boot.apply(&mut config);
        assert_eq!(config.model.unwrap().provider, "ollama");
        assert_eq!(config.workspace_dir, Some(PathBuf::from("/ws")));
        assert!(config.ssh.is_none(), "ssh_bind unset → no ssh section");
    }
}

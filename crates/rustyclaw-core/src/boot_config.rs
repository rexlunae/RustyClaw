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

        let file: Option<BootFile> = match std::fs::read_to_string(path) {
            Err(read_err) => {
                // A boot file that cannot be read at all (binary/truncated
                // content, a directory named boot.toml, unreadable
                // permissions) must go through the same quarantine-and-bail
                // path as unparseable content; otherwise the gateway would
                // fail on every start with no way out. stderr, not tracing:
                // the subscriber is installed only after Config::load
                // returns, so a tracing event here would be dropped.
                eprintln!(
                    "ERROR: Boot config {} could not be read: {read_err}",
                    path.display()
                );
                None
            }
            Ok(content) => match toml::from_str(&content) {
                Ok(file) => Some(file),
                Err(toml_err) => match serde_json::from_str::<BootFile>(&content) {
                    Ok(file) => {
                        // stderr, not tracing: the gateway installs its
                        // subscriber only after Config::load returns, so a
                        // tracing event here would be dropped — the user would
                        // never learn their boot file is being read as JSON.
                        eprintln!(
                            "WARNING: Boot config {} is JSON, not TOML — continuing (it will \
                             be expected as TOML next time; consider renaming the file)",
                            path.display()
                        );
                        Some(file)
                    }
                    Err(_) => {
                        eprintln!(
                            "ERROR: Boot config {} failed to parse as TOML or JSON: {toml_err}",
                            path.display()
                        );
                        None
                    }
                },
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
                // stderr, not tracing: see the note in `load` above.
                eprintln!(
                    "WARNING: Boot config {} has an invalid gateway.ssh_bind ({bind}) — \
                     ignoring it",
                    path.display()
                );
                boot.ssh_bind = None;
            }
        }

        if let Some(workspace) = &boot.workspace {
            boot.workspace = Some(expand_tilde(workspace));
        }

        Ok(boot)
    }

    /// The boot-critical slice of an already-loaded config.
    ///
    /// The inverse of [`apply`](Self::apply), and the basis of both the
    /// migration in `Config::load` and the write-through in `Config::save`.
    ///
    /// `ssh_bind` is taken only from a config that actually has an `[ssh]`
    /// section. Manufacturing one from the gateway's built-in default would
    /// write an address the user never chose into the file that outranks
    /// their config.
    pub fn from_config(config: &Config) -> Self {
        Self {
            provider: config.model.as_ref().map(|m| m.provider.clone()),
            model: config.model.as_ref().and_then(|m| m.model.clone()),
            workspace: config.workspace_dir.clone(),
            ssh_bind: config.ssh.as_ref().map(|s| s.bind.clone()),
        }
    }

    /// Whether this carries anything worth writing.
    ///
    /// An all-`None` boot config applies nothing, so writing one buys no
    /// resilience and leaves a puzzling empty file next to the real config.
    pub fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.model.is_none()
            && self.workspace.is_none()
            && self.ssh_bind.is_none()
    }

    /// Write this boot slice to `path`, atomically.
    ///
    /// Built through `toml` rather than formatted by hand: a Windows
    /// workspace path is full of backslashes, and a file this one exists to
    /// make unbreakable is the last place to hand-roll escaping. Sections
    /// with nothing in them are omitted rather than emitted empty.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut doc = toml::Table::new();

        let mut provider = toml::Table::new();
        if let Some(name) = &self.provider {
            provider.insert("name".into(), name.clone().into());
        }
        if let Some(model) = &self.model {
            provider.insert("model".into(), model.clone().into());
        }
        if !provider.is_empty() {
            doc.insert("provider".into(), provider.into());
        }

        if let Some(workspace) = &self.workspace {
            let mut section = toml::Table::new();
            section.insert(
                "path".into(),
                workspace.to_string_lossy().into_owned().into(),
            );
            doc.insert("workspace".into(), section.into());
        }

        if let Some(bind) = &self.ssh_bind {
            let mut section = toml::Table::new();
            section.insert("ssh_bind".into(), bind.clone().into());
            doc.insert("gateway".into(), section.into());
        }

        // The header earns its place: this file outranks config.toml for the
        // fields in it, and someone finding it for the first time — it is
        // written for them, not by them — needs to know that before they edit
        // the other one and wonder why nothing changed.
        let body = format!(
            "# RustyClaw boot config — the small, stable slice needed to start\n\
             # the gateway and reach a model. Written automatically; safe to edit.\n\
             #\n\
             # These fields win over config.toml, and changes made through the\n\
             # app are written back here too, so the two cannot drift.\n\n{}",
            toml::to_string_pretty(&doc)?
        );
        crate::persist::write_atomically(path, body.as_bytes())
            .with_context(|| format!("Failed to write boot config {}", path.display()))
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
            let model = self.model.clone().or_else(|| {
                // The old model name belongs to the old provider; carrying it
                // across a switch asks the new provider for a model it does
                // not have (e.g. "gpt-4o" under provider "anthropic").
                // Fall back to the new provider's default instead.
                if same_provider {
                    existing.and_then(|m| m.model.clone())
                } else {
                    None
                }
            });
            let base_url = if same_provider {
                existing.and_then(|m| m.base_url.clone())
            } else {
                None
            };
            // A pinned trust anchor names a certificate for one provider's
            // endpoint, so it goes the same way as `base_url`: kept while the
            // provider is unchanged, dropped on a switch, where it could only
            // pin the new endpoint to the old provider's CA.
            //
            // Dropping it silently is not acceptable even when it is right.
            // `Config::load` installs the pin immediately after this, so an
            // operator who configured one would otherwise go on believing
            // provider traffic was pinned while it quietly used the system
            // trust store. stderr, not tracing: this runs before the gateway
            // installs its subscriber (see the note in `load`).
            let tls_ca_cert = if same_provider {
                existing.and_then(|m| m.tls_ca_cert.clone())
            } else {
                if let Some(dropped) = existing.and_then(|m| m.tls_ca_cert.as_ref()) {
                    eprintln!(
                        "WARNING: boot.toml switches the provider to {provider}, so the TLS \
                         trust-anchor pin {} configured for the previous provider no longer \
                         applies — provider traffic will use the system trust store. Re-pin \
                         under the new provider if it should stay pinned.",
                        dropped.display()
                    );
                }
                None
            };
            config.model = Some(ModelProvider {
                provider: provider.clone(),
                model,
                base_url,
                tls_ca_cert,
                // Tuning, not identity. These describe how the user wants
                // requests shaped and mean the same thing under any provider,
                // so unlike `model`/`base_url`/`tls_ca_cert` they are not the
                // old provider's to lose. Hard-coding them to `None` was
                // harmless only while no install had a boot.toml; once one is
                // written for every install it silently erases them on the
                // next start, and the following save makes that permanent.
                reasoning_effort: existing.and_then(|m| m.reasoning_effort.clone()),
                max_tokens: existing.and_then(|m| m.max_tokens),
                temperature: existing.and_then(|m| m.temperature),
                token_budget: existing.and_then(|m| m.token_budget),
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
                // No `[ssh]` section: record the bind on a *disabled*
                // standalone section rather than enabling the transport.
                // A boot bind is an address, not an on-switch — silently
                // opening a network listener the user never enabled would
                // be a footgun (e.g. ssh_bind = "0.0.0.0:2222" exposing the
                // gateway to the LAN). The bind takes effect once SSH is
                // enabled, exactly as it does for an existing disabled
                // section.
                None => {
                    config.ssh = Some(SshGatewayConfig {
                        enabled: false,
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
                tls_ca_cert: None,
                reasoning_effort: None,
                max_tokens: None,
                temperature: None,
                token_budget: None,
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
        // No ssh section existed: the boot bind records the address on a
        // *disabled* standalone section (SEC_0001). A bind is an address,
        // not an on-switch — silently opening a network listener the user
        // never enabled would expose the gateway (e.g. `0.0.0.0:2222` to
        // the LAN). The bind takes effect once SSH is enabled, exactly as
        // it does for an existing disabled section.
        assert!(
            !ssh.enabled,
            "boot ssh_bind must not enable the SSH transport"
        );
        assert_eq!(ssh.mode, crate::config::SshMode::Standalone);
    }

    /// Everything `boot.toml` does not carry must survive `apply`.
    ///
    /// `apply` rebuilt `ModelProvider` from scratch with these hard-coded to
    /// `None`, which cost nothing while no install had a boot.toml. Writing
    /// one for every install turned it into a silent wipe of the user's model
    /// tuning on the next start — and the save after that made it permanent.
    #[test]
    fn apply_preserves_model_settings_the_boot_slice_does_not_carry() {
        let mut config = Config {
            model: Some(crate::config::ModelProvider {
                provider: "openai".into(),
                model: Some("gpt-4o".into()),
                base_url: None,
                tls_ca_cert: Some(PathBuf::from("/etc/pin.pem")),
                reasoning_effort: Some("high".into()),
                max_tokens: Some(9001),
                temperature: Some(0.25),
                token_budget: Some(5_000_000),
            }),
            ..Config::default()
        };
        let boot = BootConfig {
            provider: Some("openai".into()),
            model: Some("gpt-4o".into()),
            ..BootConfig::default()
        };
        boot.apply(&mut config);

        let m = config.model.unwrap();
        assert_eq!(m.tls_ca_cert.as_deref(), Some(Path::new("/etc/pin.pem")));
        assert_eq!(m.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(m.max_tokens, Some(9001));
        assert_eq!(m.temperature, Some(0.25));
        assert_eq!(m.token_budget, Some(5_000_000));
    }

    /// A pin names a certificate for one provider's endpoint, so a provider
    /// switch drops it — pinning the new endpoint to the old provider's CA
    /// would be worse than not pinning. The tuning fields are not endpoint
    /// bound and still survive.
    #[test]
    fn apply_drops_only_the_endpoint_bound_fields_on_a_provider_switch() {
        let mut config = Config {
            model: Some(crate::config::ModelProvider {
                provider: "openai".into(),
                model: Some("gpt-4o".into()),
                base_url: Some("https://proxy.example".into()),
                tls_ca_cert: Some(PathBuf::from("/etc/openai-pin.pem")),
                reasoning_effort: Some("high".into()),
                max_tokens: Some(9001),
                temperature: Some(0.25),
                token_budget: Some(5_000_000),
            }),
            ..Config::default()
        };
        let boot = BootConfig {
            provider: Some("anthropic".into()),
            ..BootConfig::default()
        };
        boot.apply(&mut config);

        let m = config.model.unwrap();
        assert!(m.base_url.is_none(), "endpoint belongs to the old provider");
        assert!(m.tls_ca_cert.is_none(), "so does its trust anchor");
        assert_eq!(m.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(m.max_tokens, Some(9001));
        assert_eq!(m.temperature, Some(0.25));
        assert_eq!(m.token_budget, Some(5_000_000));
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
                tls_ca_cert: None,
                reasoning_effort: None,
                max_tokens: None,
                temperature: None,
                token_budget: None,
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

    /// Regression for Devin review #442 (BUG_0001): when the boot file names
    /// a provider but no model, the old provider's model name must NOT be
    /// carried over — the new provider would be asked for a model it does not
    /// have (e.g. "gpt-4o" under "anthropic"), failing every request. The
    /// contract is "model optional; provider default if absent".
    #[test]
    fn apply_provider_switch_drops_old_model_name() {
        let mut config = Config {
            model: Some(crate::config::ModelProvider {
                provider: "openai".into(),
                model: Some("gpt-4o".into()),
                base_url: Some("https://api.openai.com/v1".into()),
                tls_ca_cert: None,
                reasoning_effort: None,
                max_tokens: None,
                temperature: None,
                token_budget: None,
            }),
            ..Config::default()
        };
        let boot = BootConfig {
            provider: Some("anthropic".into()),
            model: None,
            ..BootConfig::default()
        };
        boot.apply(&mut config);

        let m = config.model.unwrap();
        assert_eq!(m.provider, "anthropic");
        assert!(
            m.model.is_none(),
            "provider switch without a boot model must fall back to the \
             new provider's default, not inherit gpt-4o"
        );
        assert!(
            m.base_url.is_none(),
            "provider switch must drop old base_url"
        );
    }

    /// Regression for Devin review #442 (BUG_0001): with the SAME provider,
    /// an absent boot model still inherits the existing model name.
    #[test]
    fn apply_same_provider_inherits_existing_model() {
        let mut config = Config {
            model: Some(crate::config::ModelProvider {
                provider: "openai".into(),
                model: Some("gpt-4o".into()),
                base_url: Some("https://proxy.example".into()),
                tls_ca_cert: None,
                reasoning_effort: None,
                max_tokens: None,
                temperature: None,
                token_budget: None,
            }),
            ..Config::default()
        };
        let boot = BootConfig {
            provider: Some("openai".into()),
            model: None,
            ..BootConfig::default()
        };
        boot.apply(&mut config);

        let m = config.model.unwrap();
        assert_eq!(m.provider, "openai");
        assert_eq!(m.model.as_deref(), Some("gpt-4o"));
        assert_eq!(m.base_url.as_deref(), Some("https://proxy.example"));
    }

    #[test]
    fn apply_model_without_provider_attaches_to_existing() {
        let mut config = Config {
            model: Some(crate::config::ModelProvider {
                provider: "openai".into(),
                model: None,
                base_url: None,
                tls_ca_cert: None,
                reasoning_effort: None,
                max_tokens: None,
                temperature: None,
                token_budget: None,
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

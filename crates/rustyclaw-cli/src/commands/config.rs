//! `rustyclaw config get|set|unset` — read and mutate config values by path.

use anyhow::Result;
use rustyclaw_core::config::Config;

pub(crate) fn config_get(config: &Config, path: &str) -> String {
    match path {
        "settings_dir" => config.settings_dir.display().to_string(),
        "workspace_dir" | "workspace" => config.workspace_dir().display().to_string(),
        "soul_path" | "soul" => config.soul_path().display().to_string(),
        "skills_dir" | "skills" => config.skills_dir().display().to_string(),
        "gateway_url" | "gateway" => config
            .gateway_url
            .as_deref()
            .unwrap_or("(not set)")
            .to_string(),
        "model.provider" | "provider" => config
            .model
            .as_ref()
            .map(|m| m.provider.clone())
            .unwrap_or_else(|| "(not set)".into()),
        "model.model" | "model" => config
            .model
            .as_ref()
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "(not set)".into()),
        "secrets_password_protected" => config.secrets_password_protected.to_string(),
        _ => format!("(unknown config path: {})", path),
    }
}

pub(crate) fn config_set(config: &mut Config, path: &str, value: &str) -> Result<()> {
    match path {
        "workspace_dir" | "workspace" => {
            config.workspace_dir = Some(value.into());
        }
        "soul_path" | "soul" => {
            config.soul_path = Some(value.into());
        }
        "skills_dir" | "skills" => {
            config.skills_dir = Some(value.into());
        }
        "gateway_url" | "gateway" => {
            config.gateway_url = Some(value.to_string());
        }
        "model.provider" | "provider" => {
            let m = config
                .model
                .get_or_insert_with(|| rustyclaw_core::config::ModelProvider {
                    provider: String::new(),
                    model: None,
                    base_url: None,
                });
            m.provider = value.to_string();
        }
        "model.model" | "model" => {
            let m = config
                .model
                .get_or_insert_with(|| rustyclaw_core::config::ModelProvider {
                    provider: String::new(),
                    model: None,
                    base_url: None,
                });
            m.model = Some(value.to_string());
        }
        _ => anyhow::bail!("Unknown config path: {}", path),
    }
    Ok(())
}

pub(crate) fn config_unset(config: &mut Config, path: &str) -> Result<()> {
    match path {
        "workspace_dir" | "workspace" => config.workspace_dir = None,
        "soul_path" | "soul" => config.soul_path = None,
        "skills_dir" | "skills" => config.skills_dir = None,
        "gateway_url" | "gateway" => config.gateway_url = None,
        "model" | "model.provider" | "model.model" => config.model = None,
        _ => anyhow::bail!("Unknown config path: {}", path),
    }
    Ok(())
}

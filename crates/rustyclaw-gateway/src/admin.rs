//! Connection-admin frame handlers.
//!
//! Handles the client frames that mutate gateway/runtime configuration:
//! `Reload` (re-read config from disk), `ModelSwitch` (change active provider/
//! model), `SetAgentName`, and `SetWorkingDirectory`. Each updates the relevant
//! shared state and, where appropriate, streams a status frame back.

use rustyclaw_core::ignore::Ignore;
use std::sync::Arc;

use anyhow::Result;
use tracing::debug;

use rustyclaw_core::config::{Config, ModelProvider};
use rustyclaw_core::gateway::protocol;
use rustyclaw_core::gateway::protocol::server::send_reload_result;
use rustyclaw_core::gateway::{ModelContext, StatusType, transport};
use rustyclaw_core::providers as crate_providers;

use crate::session::init_copilot_session;
use crate::{SharedConfig, SharedCopilotSession, SharedModelCtx, SharedModelRegistry, SharedVault};

/// Handle a `Reload`: re-read config from disk and refresh model/session state.
pub(crate) async fn handle_reload(
    writer: &mut dyn transport::TransportWriter,
    config: &Config,
    vault: &SharedVault,
    shared_config: &SharedConfig,
    shared_model_ctx: &SharedModelCtx,
    shared_copilot_session: &SharedCopilotSession,
    model_registry: &SharedModelRegistry,
) -> Result<()> {
    let settings_dir = config.settings_dir.clone();
    let config_path = settings_dir.join("config.toml");
    match Config::load(Some(config_path)) {
        Ok(new_config) => {
            let new_model_ctx = {
                let mut v = vault.lock().await;
                ModelContext::resolve(&new_config, &mut v)
                    .ok()
                    .map(Arc::new)
            };

            let (provider, model) = if let Some(ref ctx) = new_model_ctx {
                (ctx.provider.clone(), ctx.model.clone())
            } else {
                ("(none)".to_string(), "(none)".to_string())
            };

            // Reinitialize Copilot session if the new model needs it
            if let Some(ref ctx) = new_model_ctx {
                let new_session =
                    init_copilot_session(&ctx.provider, ctx.api_key.as_deref(), vault).await;
                let mut session = shared_copilot_session.write().await;
                *session = new_session;
            }

            // Refresh the model registry from the new provider so
            // the catalog matches the active connection.
            if let Some(ref ctx) = new_model_ctx {
                let base = if ctx.base_url.is_empty() {
                    None
                } else {
                    Some(ctx.base_url.as_str())
                };
                let mut reg = model_registry.write().await;
                if let Err(e) = reg
                    .populate_from_provider(&ctx.provider, ctx.api_key.as_deref(), base)
                    .await
                {
                    tracing::warn!(
                        target: "rustyclaw::models",
                        provider = %ctx.provider,
                        error = %format!("{:#}", e),
                        "Failed to refresh model registry after Reload"
                    );
                } else if !ctx.model.is_empty() {
                    let qualified = if ctx.model.starts_with(&format!("{}/", ctx.provider)) {
                        ctx.model.clone()
                    } else {
                        format!("{}/{}", ctx.provider, ctx.model)
                    };
                    reg.set_active(&qualified).ignore();
                }
            }

            {
                let mut cfg = shared_config.write().await;
                *cfg = new_config;
            }
            {
                let mut ctx = shared_model_ctx.write().await;
                *ctx = new_model_ctx.clone();
            }

            send_reload_result(writer, true, &provider, &model, None).await?;

            if let Some(ref ctx) = new_model_ctx {
                let display = crate_providers::display_name_for_provider(&ctx.provider);
                let detail = format!("{} / {} (reloaded)", display, ctx.model);
                protocol::server::send_status(writer, StatusType::ModelConfigured, &detail).await?;
            }
        }
        Err(e) => {
            protocol::server::send_error(writer, &format!("Failed to reload config: {}", e))
                .await?;
        }
    }
    Ok(())
}

/// Handle a `ModelSwitch`: change the active provider/model and persist it.
pub(crate) async fn handle_model_switch(
    writer: &mut dyn transport::TransportWriter,
    vault: &SharedVault,
    shared_config: &SharedConfig,
    shared_model_ctx: &SharedModelCtx,
    shared_copilot_session: &SharedCopilotSession,
    provider: String,
    model: String,
) -> Result<()> {
    debug!("Model switch request: {} / {}", provider, model);
    let cfg_model = shared_config.read().await.model.clone();
    let base_url = crate_providers::base_url_for_provider(&provider)
        .unwrap_or("")
        .to_string();
    let api_key = {
        let key_name = crate_providers::secret_key_for_provider(&provider);
        if let Some(name) = key_name {
            let mut v = vault.lock().await;
            v.get_secret(name, true)
                .ok()
                .flatten()
                .or_else(|| std::env::var(name).ok())
        } else {
            None
        }
    };

    let mut new_ctx = ModelContext {
        provider: provider.clone(),
        model: model.clone(),
        base_url,
        api_key: api_key.clone(),
        reasoning_effort: cfg_model.as_ref().and_then(|m| m.reasoning_effort.clone()),
        max_tokens: cfg_model.as_ref().and_then(|m| m.max_tokens),
        temperature: cfg_model.as_ref().and_then(|m| m.temperature),
        top_p: cfg_model.as_ref().and_then(|m| m.top_p),
    };
    // A runtime model switch honours the curated defaults exactly like a
    // boot-time resolve — switching to a Qwen model mid-session should not
    // silently lose the sampling settings it needs.
    {
        let cfg = shared_config.read().await;
        new_ctx.apply_curated_defaults(&cfg.settings_dir);
    }
    let new_ctx = Arc::new(new_ctx);

    // Reinitialize Copilot session if needed
    let new_session = init_copilot_session(&provider, api_key.as_deref(), vault).await;
    {
        let mut session = shared_copilot_session.write().await;
        *session = new_session;
    }
    {
        let mut ctx = shared_model_ctx.write().await;
        *ctx = Some(new_ctx);
    }

    // Also update the config so it persists across restarts
    {
        let mut cfg = shared_config.write().await;
        let base = crate_providers::base_url_for_provider(&provider).map(String::from);
        cfg.model = Some(ModelProvider {
            provider: provider.clone(),
            model: Some(model.clone()),
            base_url: base,
            tls_ca_cert: None,
            reasoning_effort: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            token_budget: None,
        });
        crate::helpers::persist_config(&cfg);
    }

    let display = crate_providers::display_name_for_provider(&provider);
    send_reload_result(writer, true, &provider, &model, None).await?;
    let detail = format!("{} / {}", display, model);
    protocol::server::send_status(writer, StatusType::ModelConfigured, &detail).await?;
    Ok(())
}

/// Handle a `SetAgentName`: update the agent name in config and shared state.
pub(crate) async fn handle_set_agent_name(
    config: &mut Config,
    shared_config: &SharedConfig,
    name: String,
) {
    debug!("Agent name change: {}", name);
    {
        let mut cfg = shared_config.write().await;
        cfg.agent_name = name.clone();
        crate::helpers::persist_config(&cfg);
    }
    config.agent_name = name;
}

/// Handle a `SetWorkingDirectory`: repoint the workspace.
///
/// The sandbox is deliberately *not* re-initialised here. It never could be —
/// it lives in a `OnceLock` set once at gateway startup, so the re-init this
/// used to attempt was a silent no-op with a comment claiming otherwise. Nor
/// does it need to be: the policy's denies (credentials dir, configured deny
/// paths) are directory-independent, and every execution site substitutes the
/// per-command working directory into its copy of the policy — which, with a
/// turn per thread, is also the only shape that works, since two turns in
/// different projects need different workspaces at the same time.
pub(crate) fn handle_set_working_directory(config: &mut Config, path: std::path::PathBuf) {
    debug!("Working directory change: {}", path.display());
    config.workspace_dir = Some(path);
}

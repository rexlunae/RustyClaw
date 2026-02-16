use super::auth;
use super::providers;
use super::{CopilotSession, SharedConfig, SharedModelCtx, SharedVault};
use crate::providers as crate_providers;
use crate::secret::{ExposeSecret, SecretString};
use crate::{config::Config, gateway::types::ParsedToolCall};
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const HEARTBEAT_OK: &str = "HEARTBEAT_OK";
const DISABLED_POLL_SECS: u64 = 10;
const MIN_HEARTBEAT_INTERVAL_SECS: u64 = 30;

fn build_heartbeat_prompt(checklist: &str, system_prompt: Option<&str>) -> String {
    let mut prompt = String::new();
    if let Some(system_prompt) = system_prompt {
        let trimmed = system_prompt.trim();
        if !trimmed.is_empty() {
            prompt.push_str(trimmed);
            prompt.push_str("\n\n");
        }
    }
    prompt.push_str(
        "You are running RustyClaw heartbeat monitoring. \
Return exactly HEARTBEAT_OK if no human action is required. \
If human action is required, return a concise alert with concrete next steps. \
Do not call tools.",
    );
    prompt.push_str("\n\nHeartbeat checklist:\n");
    prompt.push_str(checklist.trim());
    prompt
}

fn normalize_heartbeat_result(text: &str, tool_calls: &[ParsedToolCall]) -> Option<String> {
    let trimmed = text.trim();
    if tool_calls.is_empty() && trimmed == HEARTBEAT_OK {
        return None;
    }

    if !tool_calls.is_empty() {
        let names: Vec<String> = tool_calls.iter().map(|t| t.name.clone()).collect();
        if trimmed.is_empty() || trimmed == HEARTBEAT_OK {
            return Some(format!(
                "Heartbeat requested unexpected tool calls: {}",
                names.join(", ")
            ));
        }
    }

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn effective_interval_secs(interval_secs: u64) -> u64 {
    interval_secs.max(MIN_HEARTBEAT_INTERVAL_SECS)
}

async fn run_heartbeat_once(
    http: &reqwest::Client,
    config: &Config,
    shared_model_ctx: &SharedModelCtx,
    vault: &SharedVault,
    copilot_session: Option<&CopilotSession>,
) -> Result<Option<String>> {
    let heartbeat_path = config.workspace_dir().join("HEARTBEAT.md");
    let checklist = match std::fs::read_to_string(&heartbeat_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if checklist.trim().is_empty() {
        return Ok(None);
    }

    let model_ctx = {
        let ctx = shared_model_ctx.read().await;
        ctx.clone()
    };
    let Some(model_ctx) = model_ctx else {
        return Ok(None);
    };

    let mut api_key = model_ctx.api_key.clone();
    if api_key.is_none() {
        if let Some(key_name) = crate_providers::secret_key_for_provider(&model_ctx.provider) {
            let mut v = vault.lock().await;
            if let Ok(Some(key)) = v.get_secret(key_name, true) {
                api_key = Some(SecretString::new(key));
            }
        }
    }

    let effective_key = auth::resolve_bearer_token(
        http,
        &model_ctx.provider,
        api_key.as_ref().map(|v| v.expose_secret()),
        copilot_session,
    )
    .await?;

    let request = super::ProviderRequest {
        messages: vec![super::ChatMessage::text(
            "user",
            &build_heartbeat_prompt(&checklist, config.system_prompt.as_deref()),
        )],
        model: model_ctx.model.clone(),
        provider: model_ctx.provider.clone(),
        base_url: model_ctx.base_url.clone(),
        api_key: effective_key.map(SecretString::new),
    };

    let model_resp = if request.provider == "anthropic" {
        providers::call_anthropic_with_tools(http, &request, None).await?
    } else if request.provider == "google" {
        providers::call_google_with_tools(http, &request).await?
    } else {
        providers::call_openai_with_tools(http, &request).await?
    };

    Ok(normalize_heartbeat_result(
        &model_resp.text,
        &model_resp.tool_calls,
    ))
}

pub async fn run_heartbeat_loop(
    shared_config: SharedConfig,
    shared_model_ctx: SharedModelCtx,
    vault: SharedVault,
    copilot_session: Option<Arc<CopilotSession>>,
    cancel: CancellationToken,
) -> Result<()> {
    let http = reqwest::Client::new();

    loop {
        let cfg = { shared_config.read().await.clone() };
        let sleep_secs = if cfg.heartbeat.enabled {
            effective_interval_secs(cfg.heartbeat.interval_secs)
        } else {
            DISABLED_POLL_SECS
        };

        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(sleep_secs)) => {}
        }

        let cfg = { shared_config.read().await.clone() };
        if !cfg.heartbeat.enabled {
            continue;
        }

        match run_heartbeat_once(
            &http,
            &cfg,
            &shared_model_ctx,
            &vault,
            copilot_session.as_deref(),
        )
        .await
        {
            Ok(Some(alert)) => {
                eprintln!("[heartbeat] Human attention required: {}", alert);
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("[heartbeat] Check failed: {}", e);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_ok_is_silent_without_tool_calls() {
        let out = normalize_heartbeat_result("HEARTBEAT_OK", &[]);
        assert!(out.is_none());
    }

    #[test]
    fn non_ok_response_requires_attention() {
        let out = normalize_heartbeat_result("Disk usage is above threshold.", &[]);
        assert_eq!(out.as_deref(), Some("Disk usage is above threshold."));
    }

    #[test]
    fn tool_call_is_attention_even_with_ok_text() {
        let tool_calls = vec![ParsedToolCall {
            id: "tc_1".to_string(),
            name: "execute_command".to_string(),
            arguments: serde_json::json!({"command":"echo hi"}),
        }];
        let out = normalize_heartbeat_result("HEARTBEAT_OK", &tool_calls);
        assert!(out.is_some());
    }

    #[test]
    fn interval_has_minimum_floor() {
        assert_eq!(effective_interval_secs(5), MIN_HEARTBEAT_INTERVAL_SECS);
        assert_eq!(effective_interval_secs(120), 120);
    }
}

//! Runs the target agent when an external trigger fires.
//!
//! This is a headless agent turn — no client connection — modeled on the
//! messenger handler's processing loop: build the agent's system prompt,
//! present the trigger context as the user message, and run the model with
//! the full tool loop. The outcome is appended to the trigger's run history
//! (`<settings_dir>/triggers/runs/<id>.jsonl`).

use std::sync::Arc;

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{ChatMessage, ModelContext, ProviderRequest, ToolCallResult};
use rustyclaw_core::tools;
use rustyclaw_core::triggers::{TriggerRunRecord, TriggerStore};
use tracing::{debug, error, info, warn};

use crate::trigger_manager::TriggerFire;
use crate::{
    SharedModelRegistry, SharedSkillManager, SharedTaskManager, SharedVault, providers,
    secrets_handler, skills_handler,
};

/// Maximum tool loop rounds for a trigger-fired turn (matches the
/// messenger handler).
const MAX_TOOL_ROUNDS: usize = 25;

/// Handle one trigger fire end to end. Never panics the caller: all
/// failures are logged and recorded in the trigger's run history.
#[allow(clippy::too_many_arguments)]
pub async fn run_trigger_fire(
    http: &reqwest::Client,
    base_config: &Config,
    fire: TriggerFire,
    model_ctx: Option<Arc<ModelContext>>,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    task_mgr: &SharedTaskManager,
    model_registry: &SharedModelRegistry,
    copilot_session: Option<&crate::CopilotSession>,
) {
    let store = TriggerStore::open(&base_config.settings_dir);
    // Scoped around the whole run, not around a tool call: a transfer this
    // turn starts must be listed and stoppable from the agent's panel, and
    // without an origin it would be neither — and unstoppable with it.
    let outcome = rustyclaw_core::downloads::with_origin(
        rustyclaw_core::downloads::headless_origin(&fire.agent_id),
        run_inner(
            http,
            base_config,
            &fire,
            model_ctx,
            vault,
            skill_mgr,
            task_mgr,
            model_registry,
            copilot_session,
        ),
    )
    .await;

    let (ok, detail) = match outcome {
        Ok(response) => {
            info!(trigger = %fire.trigger_id, agent = %fire.agent_id, "Trigger run completed");
            let excerpt: String = response.chars().take(500).collect();
            (true, excerpt)
        }
        Err(e) => {
            error!(trigger = %fire.trigger_id, agent = %fire.agent_id, error = %e, "Trigger run failed");
            (false, format!("{:#}", e))
        }
    };
    let record = TriggerRunRecord {
        ts: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        agent_id: fire.agent_id.clone(),
        ok,
        detail,
    };
    if let Err(e) = store.record_run(&fire.trigger_id, &record) {
        warn!(trigger = %fire.trigger_id, error = %e, "Failed to record trigger run");
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_inner(
    http: &reqwest::Client,
    base_config: &Config,
    fire: &TriggerFire,
    model_ctx: Option<Arc<ModelContext>>,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    task_mgr: &SharedTaskManager,
    model_registry: &SharedModelRegistry,
    copilot_session: Option<&crate::CopilotSession>,
) -> anyhow::Result<String> {
    let Some(model_ctx) = model_ctx else {
        anyhow::bail!("No model configured — cannot run trigger-fired agent turn");
    };

    // Aim the turn at the target agent: its workspace and (when set) its
    // manifest system prompt, mirroring what an AgentSwitch does for a
    // connected client.
    let registry = base_config.agent_registry();
    let Some(agent) = registry.get(&fire.agent_id) else {
        anyhow::bail!("Trigger targets unknown agent '{}'", fire.agent_id);
    };
    let mut config = base_config.clone();
    config.workspace_dir = Some(config.workspace_dir_for(&fire.agent_id));
    if let Some(prompt) = registry
        .manifest(&fire.agent_id)
        .and_then(|m| m.system_prompt)
    {
        config.system_prompt = Some(prompt);
    }
    let workspace_dir = config.workspace_dir();
    std::fs::create_dir_all(&workspace_dir).ok();
    rustyclaw_core::runtime_ctx::set_active_agent(&fire.agent_id);

    let trigger_name = TriggerStore::open(&base_config.settings_dir)
        .get(&fire.trigger_id)
        .ok()
        .flatten()
        .map(|d| d.name)
        .unwrap_or_else(|| fire.trigger_id.clone());

    let system_prompt = crate::system_prompt::build_system_prompt_full(
        &config,
        task_mgr,
        None,
        skill_mgr,
        crate::system_prompt::SessionContext {
            platform: Some("trigger"),
            origin: Some(rustyclaw_core::gateway::SessionOrigin::Trigger),
            ..Default::default()
        },
    )
    .await;
    let context_json =
        serde_json::to_string_pretty(&fire.context).unwrap_or_else(|_| "{}".to_string());
    let user_message = format!(
        "External trigger '{}' (id: {}) fired for agent '{}'.\n\n\
         Trigger context:\n```json\n{}\n```\n\n\
         Handle this event according to your instructions. If no action is \
         needed, reply with a brief note saying so.",
        trigger_name, fire.trigger_id, agent.name, context_json
    );

    let effective_key = crate::auth::resolve_bearer_token(
        http,
        &model_ctx.provider,
        model_ctx.api_key.as_deref(),
        copilot_session,
    )
    .await
    .ok()
    .flatten();

    let mut resolved = ProviderRequest {
        provider: model_ctx.provider.clone(),
        model: model_ctx.model.clone(),
        base_url: model_ctx.base_url.clone(),
        api_key: effective_key,
        messages: vec![
            ChatMessage::text("system", &system_prompt),
            ChatMessage::text("user", &user_message),
        ],
        allowed_tools: None,
    };

    let session_key = format!("trigger:{}", fire.trigger_id);
    let mut final_response = String::new();

    for _round in 0..MAX_TOOL_ROUNDS {
        let model_resp = providers::call_with_tools(http, &resolved, None).await?;

        if !model_resp.text.is_empty() {
            final_response.push_str(&model_resp.text);
        }
        if model_resp.tool_calls.is_empty() {
            break;
        }

        let mut tool_results: Vec<ToolCallResult> = Vec::new();
        for tc in &model_resp.tool_calls {
            debug!(trigger = %fire.trigger_id, tool = %tc.name, "Trigger turn tool call");

            // Honor the user's per-tool permission policy. A trigger-fired
            // turn is fully autonomous — there is no user present to answer
            // an approval prompt — so anything not outright `Allow` is
            // refused (Ask cannot be satisfied headlessly; SkillOnly has no
            // active skill here; Deny is Deny).
            let permission = config
                .tool_permissions
                .get(&tc.name)
                .cloned()
                .unwrap_or_default();
            if !matches!(permission, rustyclaw_core::tools::ToolPermission::Allow) {
                let reason = match permission {
                    rustyclaw_core::tools::ToolPermission::Deny => {
                        "denied by user policy".to_string()
                    }
                    rustyclaw_core::tools::ToolPermission::Ask => {
                        "requires user approval, which is unavailable in an autonomous \
                         trigger-fired run"
                            .to_string()
                    }
                    rustyclaw_core::tools::ToolPermission::SkillOnly(_) => {
                        "restricted to skill invocations".to_string()
                    }
                    rustyclaw_core::tools::ToolPermission::Allow => unreachable!(),
                };
                tool_results.push(ToolCallResult {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    output: format!("Tool '{}' {}.", tc.name, reason),
                    is_error: true,
                });
                continue;
            }

            let (output, is_error) = if tools::is_secrets_tool(&tc.name) {
                match secrets_handler::execute_secrets_tool(&tc.name, &tc.arguments, vault).await {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if tools::is_skill_tool(&tc.name) {
                match skills_handler::execute_skill_tool(&tc.name, &tc.arguments, skill_mgr).await {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if crate::task_handler::is_task_tool(&tc.name) {
                match crate::task_handler::execute_task_tool(
                    &tc.name,
                    &tc.arguments,
                    task_mgr,
                    Some(&session_key),
                )
                .await
                {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if crate::model_handler::is_model_tool(&tc.name) {
                match crate::model_handler::execute_model_tool(
                    &tc.name,
                    &tc.arguments,
                    model_registry,
                )
                .await
                {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if crate::mcp_handler::is_mcp_tool(&tc.name)
                || crate::canvas_handler::is_canvas_tool(&tc.name)
            {
                // Same limitation as the messenger handler: these tool
                // families need managers that aren't threaded into headless
                // turns yet.
                (
                    format!("Tool '{}' is not available in trigger-fired turns", tc.name),
                    true,
                )
            } else {
                // Scoped to this trigger's session, so a process it leaves
                // running is not reachable from another trigger or from a
                // user conversation, and its tool budget is its own.
                match rustyclaw_core::tool_limits::check_rate(Some(&session_key), &tc.name) {
                    Err(err) => (err.to_string(), true),
                    Ok(()) => match rustyclaw_core::tool_caller::with_caller(
                        session_key.clone(),
                        tools::execute_tool(&tc.name, &tc.arguments, &workspace_dir),
                    )
                    .await
                    {
                        Ok(text) => (text, false),
                        Err(err) => (err.to_string(), true),
                    },
                }
            };
            tool_results.push(ToolCallResult {
                id: tc.id.clone(),
                name: tc.name.clone(),
                output,
                is_error,
            });
        }

        providers::append_tool_round(
            &resolved.provider,
            &mut resolved.messages,
            &model_resp,
            &tool_results,
        );
    }

    Ok(final_response)
}

//! Text-message dispatch: turning a client chat request into model calls,
//! running the tool loop, handling user-prompt / DOM-query tool round-trips,
//! and streaming results back. [`crate::chat`] assembles the per-frame context
//! and delegates the model/tool loop to [`dispatch_text_message`].

use anyhow::{Context, Result};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;
use tracing::{debug, trace};

use rustyclaw_core::gateway::{
    ChatMessage, ChatRequest, CopilotSession, ModelContext, ModelResponse, ServerFrame,
    ServerFrameType, ServerPayload, ToolCallResult, protocol, transport,
};
use rustyclaw_core::observability::ObserverEvent;
use rustyclaw_core::tools;

use crate::thread_updates::{send_thread_messages_update, send_threads_update};
use crate::{
    COMPACTION_THRESHOLD, SharedConfig, SharedCopilotSession, SharedObserver, SharedSkillManager,
    SharedTaskManager, SharedVault, ToolCancelFlag, auth, errors, helpers, providers,
    tool_executor,
};
use protocol::server::send_frame;

async fn execute_user_prompt(
    writer: &mut dyn transport::TransportWriter,
    call_id: &str,
    arguments: &serde_json::Value,
    user_prompt_rx: &Arc<
        Mutex<
            tokio::sync::mpsc::Receiver<(
                String,
                bool,
                rustyclaw_core::user_prompt_types::PromptResponseValue,
            )>,
        >,
    >,
) -> (String, bool) {
    use rustyclaw_core::user_prompt_types::{FormField, PromptOption, PromptType, UserPrompt};

    // Parse arguments into a UserPrompt
    let prompt_type_str = arguments
        .get("prompt_type")
        .and_then(|v| v.as_str())
        .unwrap_or("text");
    let title = arguments
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Question")
        .to_string();
    let description = arguments
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let options: Vec<PromptOption> = arguments
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|o| {
                    if let Some(s) = o.as_str() {
                        PromptOption {
                            label: s.to_string(),
                            description: None,
                            value: None,
                        }
                    } else {
                        PromptOption {
                            label: o
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .to_string(),
                            description: o
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            value: o
                                .get("value")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let prompt_type = match prompt_type_str {
        "select" => {
            let default = arguments
                .get("default_value")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            PromptType::Select { options, default }
        }
        "multi_select" => {
            let defaults: Vec<usize> = arguments
                .get("default_value")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as usize))
                        .collect()
                })
                .unwrap_or_default();
            PromptType::MultiSelect { options, defaults }
        }
        "confirm" => {
            let default = arguments
                .get("default_value")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            PromptType::Confirm { default }
        }
        "text" => {
            let placeholder = arguments
                .get("placeholder")
                .or_else(|| arguments.get("description"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let default = arguments
                .get("default_value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            PromptType::TextInput {
                placeholder,
                default,
            }
        }
        "form" => {
            let fields: Vec<FormField> = arguments
                .get("fields")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|f| FormField {
                            name: f
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("field")
                                .to_string(),
                            label: f
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Field")
                                .to_string(),
                            placeholder: f
                                .get("placeholder")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            default: f
                                .get("default")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            required: f.get("required").and_then(|v| v.as_bool()).unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default();
            PromptType::Form { fields }
        }
        _ => PromptType::TextInput {
            placeholder: None,
            default: None,
        },
    };

    let prompt = UserPrompt {
        id: call_id.to_string(),
        title,
        description,
        prompt_type,
    };

    // Send the prompt directly to the TUI (embedded in the binary frame).
    if let Err(e) = protocol::server::send_user_prompt_request(writer, call_id, &prompt).await {
        return (format!("Failed to send user prompt: {}", e), true);
    }

    // Wait for the user's response (with 5 minute timeout).
    let rx_result = {
        let mut rx = user_prompt_rx.lock().await;
        tokio::time::timeout(std::time::Duration::from_secs(300), rx.recv()).await
    };

    match rx_result {
        Ok(Some((id, dismissed, value))) if id == call_id => {
            if dismissed {
                (
                    "User dismissed the prompt without answering.".to_string(),
                    false,
                )
            } else {
                use rustyclaw_core::user_prompt_types::PromptResponseValue;
                match value {
                    PromptResponseValue::Text(s) => (s, false),
                    PromptResponseValue::Confirm(b) => {
                        (if b { "yes" } else { "no" }.to_string(), false)
                    }
                    PromptResponseValue::Selected(items) => (items.join(", "), false),
                    PromptResponseValue::Form(fields) => {
                        let formatted: Vec<String> = fields
                            .iter()
                            .map(|(k, v)| format!("{}: {}", k, v))
                            .collect();
                        (formatted.join("\n"), false)
                    }
                }
            }
        }
        Ok(Some(_)) => ("Mismatched prompt response ID.".to_string(), true),
        Ok(None) => ("User prompt channel closed.".to_string(), true),
        Err(_) => ("User prompt timed out after 5 minutes.".to_string(), true),
    }
}

/// Execute the `client_dom_query` tool by sending a DOM query to the
/// desktop client and waiting for the response on the dom_query channel.
async fn execute_dom_query(
    writer: &mut dyn transport::TransportWriter,
    call_id: &str,
    arguments: &serde_json::Value,
    dom_query_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, String, bool)>>>,
) -> (String, bool) {
    let js = arguments
        .get("js")
        .and_then(|v| v.as_str())
        .unwrap_or("document.title");

    if let Err(e) = protocol::server::send_dom_query(writer, call_id, js).await {
        return (format!("Failed to send DOM query: {}", e), true);
    }

    // Wait for the client's response (30 second timeout).
    let rx_result = {
        let mut rx = dom_query_rx.lock().await;
        tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv()).await
    };

    match rx_result {
        Ok(Some((id, result, is_error))) if id == call_id => (result, is_error),
        Ok(Some(_)) => ("Mismatched DOM query response ID.".to_string(), true),
        Ok(None) => ("DOM query channel closed.".to_string(), true),
        Err(_) => (
            "DOM query timed out after 30 seconds. The client may not support DOM queries."
                .to_string(),
            true,
        ),
    }
}

/// Route an incoming text frame to the appropriate handler.
///
/// Implements an agentic tool loop: the model is called, and if it
/// requests tool calls, the gateway executes them locally and feeds
/// the results back into the conversation, repeating until the model
/// produces a final text response (or a safety limit is hit).
///
/// The `tool_cancel` flag can be set by another task to interrupt the
/// tool loop gracefully.
async fn await_model_with_cancel<F>(
    fut: F,
    tool_cancel: &ToolCancelFlag,
    timeout: std::time::Duration,
) -> Result<Option<ModelResponse>>
where
    F: Future<Output = Result<ModelResponse>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    tokio::pin!(fut);

    loop {
        if tool_cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }

        let now = tokio::time::Instant::now();
        if now >= deadline {
            anyhow::bail!("Model request timed out after {}s", timeout.as_secs());
        }

        // Poll both the model future and a short timer so cancel requests are
        // observed quickly even while waiting on provider/network latency.
        let tick = std::time::Duration::from_millis(200).min(deadline - now);
        tokio::select! {
            res = &mut fut => return res.map(Some),
            _ = tokio::time::sleep(tick) => {}
        }
    }
}

pub(crate) async fn dispatch_text_message(
    http: &reqwest::Client,
    req: &ChatRequest,
    model_ctx: Option<&ModelContext>,
    copilot_session: Option<&CopilotSession>,
    writer: &mut dyn transport::TransportWriter,
    workspace_dir: &std::path::Path,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    task_mgr: &SharedTaskManager,
    observer: Option<&SharedObserver>,
    tool_cancel: &ToolCancelFlag,
    shared_config: &SharedConfig,
    shared_copilot_session: &SharedCopilotSession,
    approval_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, bool)>>>,
    user_prompt_rx: &Arc<
        Mutex<
            tokio::sync::mpsc::Receiver<(
                String,
                bool,
                rustyclaw_core::user_prompt_types::PromptResponseValue,
            )>,
        >,
    >,
    credential_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, bool, Option<String>)>>>,
    dom_query_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, String, bool)>>>,
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    threads_path: &std::path::Path,
) -> Result<()> {
    let mut resolved = match providers::resolve_request(req.clone(), model_ctx) {
        Ok(r) => r,
        Err(msg) => {
            let error_frame = ServerFrame {
                frame_type: ServerFrameType::Error,
                payload: ServerPayload::Error {
                    ok: false,
                    message: msg,
                },
            };
            send_frame(writer, &error_frame)
                .await
                .context("Failed to send error frame")?;
            return Ok(());
        }
    };

    // If we still don't have an API key, try fetching it fresh from
    // the vault.  This handles the case where a key was stored after
    // the gateway started (e.g. user entered it via the TUI dialog).
    if resolved.api_key.is_none() {
        if let Some(key_name) =
            rustyclaw_core::providers::secret_key_for_provider(&resolved.provider)
        {
            let mut v = vault.lock().await;
            if let Ok(Some(key)) = v.get_secret(key_name, true) {
                resolved.api_key = Some(key);
            }
        }
    }

    // If we have an API key for a Copilot provider but no CopilotSession,
    // create one so the OAuth-to-session-token exchange works.  Without
    // this, the raw OAuth token would be sent as-is and rejected by the
    // Copilot API, re-triggering the device flow in an infinite loop.
    let mut local_copilot: Option<Arc<CopilotSession>> = None;
    if copilot_session.is_none()
        && rustyclaw_core::providers::needs_copilot_session(&resolved.provider)
        && resolved.api_key.is_some()
    {
        let session = Arc::new(CopilotSession::new(
            resolved.api_key.clone().expect("checked is_some above"),
        ));
        local_copilot = Some(session.clone());
        let mut s = shared_copilot_session.write().await;
        *s = Some(session);
    }

    // Store the original API key for non-Copilot providers.
    // For Copilot, we'll refresh the session token on each loop iteration.
    let mut original_api_key = resolved.api_key.clone();

    // ── Agentic tool loop ───────────────────────────────────────────
    // No hard limit — the model will stop when it's done. The user can
    // cancel by sending a {"type": "cancel"} message (e.g., pressing Esc).
    // We use a very high limit as a safety net against infinite loops.
    const MAX_TOOL_ROUNDS: usize = 500;
    /// Maximum consecutive auto-continuations before giving up.
    /// Prevents infinite loops when the model keeps narrating intent
    /// but never actually makes tool calls.
    const MAX_AUTO_CONTINUES: usize = 2;

    let context_limit = helpers::context_window_for_model(&resolved.model);
    let mut consecutive_continues: usize = 0;

    // Track all tool_use IDs already present in the conversation. New model
    // responses are checked against this set to prevent collisions — some
    // adapters/proxies can re-emit the same ID across turns.
    let mut seen_tool_ids = collect_existing_tool_ids(&resolved.messages);

    // Memory flush controller - tracks whether we've flushed this conversation
    use rustyclaw_core::memory_flush::MemoryFlush;
    let flush_config = {
        let cfg = shared_config.read().await;
        cfg.memory_flush.clone()
    };
    let mut memory_flush = MemoryFlush::new(flush_config);

    for _round in 0..MAX_TOOL_ROUNDS {
        // ── Check for cancellation ──────────────────────────────────
        if tool_cancel.load(Ordering::Relaxed) {
            protocol::server::send_info(writer, "Tool loop cancelled by user.").await?;
            providers::send_response_done(writer).await?;
            return Ok(());
        }

        // Refresh the bearer token before each model call.
        // For Copilot providers, this ensures the session token is still valid.
        let effective_copilot = local_copilot.as_deref().or(copilot_session);
        match auth::resolve_bearer_token(
            http,
            &resolved.provider,
            original_api_key.as_deref(),
            effective_copilot,
        )
        .await
        {
            Ok(token) => {
                resolved.api_key = token;
                // Use the plan-specific API base URL from the session exchange
                // (e.g. api.individual.githubcopilot.com) when available.
                if let Some(session) = effective_copilot {
                    if rustyclaw_core::providers::needs_copilot_session(&resolved.provider) {
                        if let Some(base) = session.api_base_url().await {
                            resolved.base_url = base;
                        }
                    }
                }
            }
            Err(err) => {
                let traced = errors::GatewayError::TokenRefresh {
                    message: format!("{err}"),
                }
                .into_traced_with_source(err);
                match errors::handle(
                    traced,
                    writer,
                    &mut resolved,
                    &mut original_api_key,
                    vault,
                    credential_rx,
                    tool_cancel,
                )
                .await?
                {
                    std::ops::ControlFlow::Continue(()) => {
                        if rustyclaw_core::providers::needs_copilot_session(&resolved.provider)
                            && original_api_key.is_some()
                        {
                            let session = Arc::new(CopilotSession::new(
                                original_api_key.clone().expect("checked is_some above"),
                            ));
                            local_copilot = Some(session.clone());
                            let mut s = shared_copilot_session.write().await;
                            *s = Some(session);
                        }
                        continue;
                    }
                    std::ops::ControlFlow::Break(()) => return Ok(()),
                }
            }
        }

        // ── Pre-compaction memory flush ─────────────────────────────
        // Check if we should trigger a memory flush before compaction
        let estimated = helpers::estimate_tokens(&resolved.messages);
        let threshold = (context_limit as f64 * COMPACTION_THRESHOLD) as usize;

        if memory_flush.should_flush(estimated, context_limit, COMPACTION_THRESHOLD) {
            let (system_msg, user_msg) = memory_flush.build_flush_messages();

            // Inject memory flush prompt
            // The agent will process this and can use tools to write to memory files
            resolved
                .messages
                .push(ChatMessage::text("system", &system_msg));
            resolved.messages.push(ChatMessage::text("user", &user_msg));

            // Mark as flushed to prevent repeated injections
            memory_flush.mark_flushed();

            // Notify the TUI about the memory flush
            let _ =
                protocol::server::send_info(writer, "💾 Memory flush triggered before compaction")
                    .await;
        }

        // ── Auto-compact if context is getting large ────────────────
        // Proceed with compaction if over threshold
        if estimated > threshold {
            match providers::compact_conversation(http, &mut resolved, context_limit, writer).await
            {
                Ok(()) => {} // compacted in-place
                Err(err) => {
                    let traced = errors::GatewayError::ContextCompaction {
                        message: format!("{err}"),
                    }
                    .into_traced_with_source(err);
                    // Non-fatal — handle logs and continues.
                    let _ = errors::handle(
                        traced,
                        writer,
                        &mut resolved,
                        &mut original_api_key,
                        vault,
                        credential_rx,
                        tool_cancel,
                    )
                    .await;
                }
            }
        }

        // Record LLM request event for observability
        let request_start = std::time::Instant::now();
        if let Some(obs) = observer {
            obs.record_event(&ObserverEvent::LlmRequest {
                provider: resolved.provider.clone(),
                model: resolved.model.clone(),
                messages_count: resolved.messages.len(),
            });
        }

        let model_timeout = std::time::Duration::from_secs(180);
        let result = if resolved.provider == "anthropic" {
            // Anthropic: use streaming mode with writer for real-time chunks.
            // Still enforce timeout/cancel around the provider future.
            await_model_with_cancel(
                providers::call_anthropic_with_tools(http, &resolved, Some(writer)),
                tool_cancel,
                model_timeout,
            )
            .await
        } else if resolved.provider == "google" {
            await_model_with_cancel(
                providers::call_google_with_tools(http, &resolved),
                tool_cancel,
                model_timeout,
            )
            .await
        } else {
            await_model_with_cancel(
                providers::call_openai_with_tools(http, &resolved, Some(writer)),
                tool_cancel,
                model_timeout,
            )
            .await
        };

        // Record LLM response event
        let request_duration = request_start.elapsed();
        let (success, error_msg) = match &result {
            Ok(_) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        };
        if let Some(obs) = observer {
            obs.record_event(&ObserverEvent::LlmResponse {
                provider: resolved.provider.clone(),
                model: resolved.model.clone(),
                duration: request_duration,
                success,
                error_message: error_msg,
                input_tokens: None, // TODO: extract from response if available
                output_tokens: None,
            });
        }

        let mut model_resp = match result {
            Ok(Some(r)) => r,
            Ok(None) => {
                let traced = errors::GatewayError::Cancelled.into_traced();
                match errors::handle(
                    traced,
                    writer,
                    &mut resolved,
                    &mut original_api_key,
                    vault,
                    credential_rx,
                    tool_cancel,
                )
                .await?
                {
                    std::ops::ControlFlow::Continue(()) => continue,
                    std::ops::ControlFlow::Break(()) => return Ok(()),
                }
            }
            Err(err) => {
                let traced = errors::classify_model_error(err, &resolved.provider);
                match errors::handle(
                    traced,
                    writer,
                    &mut resolved,
                    &mut original_api_key,
                    vault,
                    credential_rx,
                    tool_cancel,
                )
                .await?
                {
                    std::ops::ControlFlow::Continue(()) => {
                        if rustyclaw_core::providers::needs_copilot_session(&resolved.provider)
                            && original_api_key.is_some()
                        {
                            let session = Arc::new(CopilotSession::new(
                                original_api_key.clone().expect("checked is_some above"),
                            ));
                            local_copilot = Some(session.clone());
                            let mut s = shared_copilot_session.write().await;
                            *s = Some(session);
                        }
                        continue;
                    }
                    std::ops::ControlFlow::Break(()) => return Ok(()),
                }
            }
        };

        // Stream any text content to the client.
        // For Anthropic, text is already streamed via the writer, so skip if empty.
        // For other providers, send the accumulated text.
        trace!(
            provider = %resolved.provider,
            text_len = model_resp.text.len(),
            tool_calls = model_resp.tool_calls.len(),
            "Model response received"
        );
        if !model_resp.text.is_empty()
            && resolved.provider != "anthropic"
            && resolved.provider == "google"
        {
            trace!(chars = model_resp.text.len(), "Sending chunk to TUI");
            providers::send_chunk(writer, &model_resp.text).await?;
        }

        // Check if the model is truly done or if something went wrong
        let finish_reason = model_resp.finish_reason.as_deref().unwrap_or("stop");

        if model_resp.tool_calls.is_empty() {
            // No tool calls requested
            if finish_reason == "stop" || finish_reason == "end_turn" {
                // ── Auto-continuation for incomplete intent ─────────────
                // Sometimes the model narrates what it plans to do ("Let me check...")
                // but returns finish_reason=stop without making a tool call.
                // Detect this and prompt the model to continue.
                let should_continue = tool_executor::should_auto_continue(
                    &model_resp.text,
                    consecutive_continues,
                    MAX_AUTO_CONTINUES,
                );

                if should_continue {
                    consecutive_continues += 1;
                    debug!(
                        text_chars = model_resp.text.len(),
                        attempt = consecutive_continues,
                        max_attempts = MAX_AUTO_CONTINUES,
                        "Detected incomplete intent, prompting continuation"
                    );

                    // NOTE: do NOT re-send the text here — for non-Anthropic
                    // providers it was already sent above; for Anthropic it was
                    // already streamed via the SSE handler.  Re-sending would
                    // cause the TUI to show the same text twice.

                    // Append assistant message and continuation prompt
                    resolved
                        .messages
                        .push(ChatMessage::text("assistant", &model_resp.text));
                    resolved.messages.push(ChatMessage::text(
                        "user",
                        "Continue. Execute the action you described.",
                    ));

                    // Don't send response_done — continue the tool loop
                    continue;
                }

                // Model explicitly finished — we're done
                // Record assistant response in thread history
                let mut updated_thread_id = None;
                if !model_resp.text.is_empty() {
                    if let Some(thread) = thread_mgr.foreground_mut() {
                        updated_thread_id = Some(thread.id);
                        thread.add_message(
                            rustyclaw_core::threads::MessageRole::Assistant,
                            &model_resp.text,
                        );
                    }
                    // Persist the final assistant turn so reconnecting
                    // clients see it via ThreadHistoryRequest.
                    let _ = thread_mgr.save_to_file(threads_path);
                    // Auto-ingest assistant response into Steel Memory
                    #[cfg(feature = "semantic-memory")]
                    {
                        let ws = workspace_dir.to_path_buf();
                        let text = model_resp.text.clone();
                        tokio::spawn(async move {
                            if let Ok(mem) = rustyclaw_core::steel_memory::SteelMemory::new(&ws) {
                                let _ = mem
                                    .add_memory(&text, "conversations", "assistant", None)
                                    .await;
                            }
                        });
                    }
                }
                providers::send_response_done(writer).await?;
                if let Some(thread_id) = updated_thread_id {
                    send_thread_messages_update(writer, thread_id, thread_mgr).await?;
                }
                return Ok(());
            } else if finish_reason == "length" {
                let traced = errors::GatewayError::TokenLimit.into_traced();
                match errors::handle(
                    traced,
                    writer,
                    &mut resolved,
                    &mut original_api_key,
                    vault,
                    credential_rx,
                    tool_cancel,
                )
                .await?
                {
                    std::ops::ControlFlow::Continue(()) => continue,
                    std::ops::ControlFlow::Break(()) => return Ok(()),
                }
            } else {
                let traced = errors::GatewayError::UnexpectedFinish {
                    reason: finish_reason.to_string(),
                }
                .into_traced();
                match errors::handle(
                    traced,
                    writer,
                    &mut resolved,
                    &mut original_api_key,
                    vault,
                    credential_rx,
                    tool_cancel,
                )
                .await?
                {
                    std::ops::ControlFlow::Continue(()) => continue,
                    std::ops::ControlFlow::Break(()) => return Ok(()),
                }
            }
        }

        // Reset continuation counter — model made an actual tool call
        consecutive_continues = 0;

        // ── Execute each requested tool ─────────────────────────────
        let mut tool_results: Vec<ToolCallResult> = Vec::new();

        // Snapshot current tool permissions (cheap clone of a HashMap).
        let tool_permissions = {
            let cfg = shared_config.read().await;
            cfg.tool_permissions.clone()
        };

        for tc in &model_resp.tool_calls {
            // Record tool call start event
            let tool_start = std::time::Instant::now();
            if let Some(obs) = observer {
                obs.record_event(&ObserverEvent::ToolCallStart {
                    tool: tc.name.clone(),
                });
            }

            // Stringify arguments once for the wire protocol (tool args are
            // inherently schemaless JSON from the LLM).
            let args_str = serde_json::to_string(&tc.arguments).unwrap_or_default();

            // ── Permission check ────────────────────────────────────
            let permission = tool_permissions.get(&tc.name).cloned().unwrap_or_default(); // default = Allow

            let (output, is_error) = match permission {
                tools::ToolPermission::Deny => {
                    // Notify the client about the denied tool call.
                    protocol::server::send_tool_call(writer, &tc.id, &tc.name, &args_str).await?;

                    let msg = format!(
                        "Tool '{}' is denied by user policy. The user has blocked this tool from being executed.",
                        tc.name
                    );
                    (msg, true)
                }
                tools::ToolPermission::SkillOnly(_) => {
                    // In direct chat, SkillOnly tools are denied.
                    protocol::server::send_tool_call(writer, &tc.id, &tc.name, &args_str).await?;

                    let msg = format!(
                        "Tool '{}' is restricted to skill-based invocations only. It cannot be used in direct chat.",
                        tc.name
                    );
                    (msg, true)
                }
                tools::ToolPermission::Ask => {
                    // Send approval request to the TUI and wait for response.
                    protocol::server::send_tool_approval_request(
                        writer, &tc.id, &tc.name, &args_str,
                    )
                    .await?;

                    // Wait for the user's response (with timeout).
                    let approved = {
                        let mut rx = approval_rx.lock().await;
                        match tokio::time::timeout(std::time::Duration::from_secs(120), rx.recv())
                            .await
                        {
                            Ok(Some((id, approved))) if id == tc.id => approved,
                            Ok(Some(_)) => false, // Mismatched ID — treat as denied
                            Ok(None) => false,    // Channel closed
                            Err(_) => false,      // Timeout
                        }
                    };

                    if !approved {
                        // Notify the client about the denied tool call.
                        protocol::server::send_tool_call(writer, &tc.id, &tc.name, &args_str)
                            .await?;

                        let msg = format!("Tool '{}' was denied by the user.", tc.name);
                        (msg, true)
                    } else {
                        // User approved — proceed with execution.
                        protocol::server::send_tool_call(writer, &tc.id, &tc.name, &args_str)
                            .await?;

                        if tools::is_user_prompt_tool(&tc.name) {
                            execute_user_prompt(writer, &tc.id, &tc.arguments, user_prompt_rx).await
                        } else if tools::is_dom_query_tool(&tc.name) {
                            execute_dom_query(writer, &tc.id, &tc.arguments, dom_query_rx).await
                        } else {
                            tool_executor::execute_tool_by_type(
                                &tc.name,
                                &tc.arguments,
                                workspace_dir,
                                vault,
                                skill_mgr,
                            )
                            .await
                        }
                    }
                }
                tools::ToolPermission::Allow => {
                    // Notify the client about the tool call.
                    protocol::server::send_tool_call(writer, &tc.id, &tc.name, &args_str).await?;

                    // Execute the tool.
                    if tools::is_user_prompt_tool(&tc.name) {
                        execute_user_prompt(writer, &tc.id, &tc.arguments, user_prompt_rx).await
                    } else if tools::is_dom_query_tool(&tc.name) {
                        execute_dom_query(writer, &tc.id, &tc.arguments, dom_query_rx).await
                    } else {
                        tool_executor::execute_tool_by_type(
                            &tc.name,
                            &tc.arguments,
                            workspace_dir,
                            vault,
                            skill_mgr,
                        )
                        .await
                    }
                }
            };

            // Sanitize the output (truncate large outputs, warn about garbage).
            let mut output = tools::sanitize_tool_output(output);

            // Intercept thread update markers and apply them
            if output.starts_with(tools::THREAD_UPDATE_MARKER) {
                let json_str = &output[tools::THREAD_UPDATE_MARKER.len()..];
                if let Ok(update) = serde_json::from_str::<serde_json::Value>(json_str) {
                    let action = update.get("action").and_then(|v| v.as_str()).unwrap_or("");
                    match action {
                        "set_description" => {
                            if let Some(description) =
                                update.get("description").and_then(|v| v.as_str())
                            {
                                thread_mgr.set_foreground_description(description);
                                output = format!("Thread description set to: {}", description);
                                send_threads_update(writer, thread_mgr, task_mgr, None).await?;
                            }
                        }
                        "set_caption" => {
                            if let Some(caption) = update.get("caption").and_then(|v| v.as_str()) {
                                if let Some(fg_id) = thread_mgr.foreground_id() {
                                    thread_mgr.rename(fg_id, caption);
                                    output = format!("Thread caption set to: {}", caption);
                                    send_threads_update(writer, thread_mgr, task_mgr, None).await?;
                                    let _ = thread_mgr.save_to_file(threads_path);
                                } else {
                                    output = "No active thread to caption.".to_string();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Tools that modify session state should trigger sidebar update
            const SESSION_TOOLS: &[&str] = &["sessions_spawn", "sessions_send", "subagents"];
            if SESSION_TOOLS.contains(&tc.name.as_str()) && !is_error {
                send_threads_update(writer, thread_mgr, task_mgr, None).await?;
            }

            // Notify the client about the result.
            protocol::server::send_tool_result(writer, &tc.id, &tc.name, &output, is_error).await?;

            // Record tool call completion event
            if let Some(obs) = observer {
                obs.record_event(&ObserverEvent::ToolCall {
                    tool: tc.name.clone(),
                    duration: tool_start.elapsed(),
                    success: !is_error,
                });
            }

            tool_results.push(ToolCallResult {
                id: tc.id.clone(),
                name: tc.name.clone(),
                output,
                is_error,
            });
        }

        // ── Remap non-unique or colliding tool call IDs ──
        // Two cases require remapping:
        // 1. IDs matching the genai fallback pattern ("call_0", "call_1")
        //    which are always non-unique across turns.
        // 2. IDs that collide with ones already in the conversation history
        //    (some adapters/proxies deterministically regenerate the same
        //    IDs across turns, or the model itself returns a previously-used
        //    ID).
        //
        // We remap BEFORE appending to the in-memory conversation so that
        // both the live tool loop and persisted history use unique IDs.
        for tc in &mut model_resp.tool_calls {
            if is_likely_non_unique_tool_id(&tc.id) || seen_tool_ids.contains(&tc.id) {
                let new_id = format!("toolu_{}", uuid::Uuid::new_v4().as_simple());
                tc.id = new_id;
            }
            seen_tool_ids.insert(tc.id.clone());
        }
        // Match tool_results to tool_calls POSITIONALLY: since the tool
        // execution loop iterates model_resp.tool_calls in order, the Nth
        // result corresponds to the Nth tool_call. A HashMap lookup would
        // fail when multiple tool_calls share the same original ID (the
        // last insert overwrites earlier entries, causing all matching
        // results to receive the same remapped ID).
        for (idx, tr) in tool_results.iter_mut().enumerate() {
            if idx < model_resp.tool_calls.len() {
                tr.id = model_resp.tool_calls[idx].id.clone();
            }
        }

        // ── Append assistant + tool-result messages to conversation ──
        // The model's response (possibly with text + tool calls) becomes
        // an assistant message, and each tool result becomes a tool message.
        // IDs have already been remapped above, so no duplicates enter the
        // in-memory conversation.
        providers::append_tool_round(
            &resolved.provider,
            &mut resolved.messages,
            &model_resp,
            &tool_results,
        );

        // ── Persist this turn to the foreground thread's history ──
        // We store a normalized form (Vec<{id,name,arguments}>) so any
        // client can faithfully replay this round regardless of which
        // provider produced it.
        if let Some(thread) = thread_mgr.foreground_mut() {
            let normalized: Vec<serde_json::Value> = model_resp
                .tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "name": tc.name,
                        "arguments": tc.arguments,
                    })
                })
                .collect();
            thread.add_assistant_with_tool_calls(
                model_resp.text.clone(),
                serde_json::Value::Array(normalized),
            );
            for tr in &tool_results {
                thread.add_tool_result(tr.id.clone(), tr.output.clone());
            }
        }
        let _ = thread_mgr.save_to_file(threads_path);
    }

    // If we exhausted all rounds, send what we have and stop.
    let traced = errors::GatewayError::ToolLoopExhausted {
        rounds: MAX_TOOL_ROUNDS,
    }
    .into_traced();
    let _ = errors::handle(
        traced,
        writer,
        &mut resolved,
        &mut original_api_key,
        vault,
        credential_rx,
        tool_cancel,
    )
    .await?;
    Ok(())
}

/// Detect tool call IDs that are likely non-unique across turns.
///
/// Some OpenAI-compatible streaming adapters don't provide proper tool call IDs,
/// causing genai to fall back to `"call_0"`, `"call_1"`, etc. These collide
/// across assistant turns and violate the Anthropic API requirement that all
/// `tool_use` IDs in a conversation be unique.
fn is_likely_non_unique_tool_id(id: &str) -> bool {
    if id.is_empty() {
        return true;
    }
    // Match the genai fallback pattern: "call_" followed by a small number.
    if let Some(suffix) = id.strip_prefix("call_") {
        if let Ok(n) = suffix.parse::<u32>() {
            // Genuine OpenAI IDs are "call_" + 24 alphanumeric chars (e.g.
            // "call_abc123def456ghi789jkl"). The genai fallback is "call_" +
            // a small integer. Heuristic: if the numeric suffix is < 100,
            // it's almost certainly the fallback, not a real ID.
            return n < 100;
        }
    }
    false
}

/// Extract all tool_use IDs present in a conversation message array.
///
/// Parses assistant messages encoded in the canonical `assistant_tools`
/// envelope and collects their `tool_calls[].id` fields. This allows us
/// to detect collisions when a new model response produces an ID that
/// already exists in the replayed history.
fn collect_existing_tool_ids(
    messages: &[rustyclaw_core::gateway::ChatMessage],
) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    for msg in messages {
        if msg.role != "assistant" {
            continue;
        }
        // Try to parse the canonical envelope
        if let Ok(env) = serde_json::from_str::<serde_json::Value>(&msg.content) {
            if env.get("__rustyclaw_kind").and_then(|v| v.as_str()) == Some("assistant_tools") {
                if let Some(calls) = env.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in calls {
                        if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                            if !id.is_empty() {
                                ids.insert(id.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    ids
}

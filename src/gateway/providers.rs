use anyhow::{Context, Result};
use futures_util::SinkExt;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use crate::metrics;
use crate::retry::{RetryAttempt, RetryPolicy, classify_reqwest_result, retry_with_backoff};
use crate::secret::{ExposeSecret, SecretString};
use super::types::{
    ChatMessage, CopilotSession, ModelContext, ModelResponse, ParsedToolCall, ProviderRequest,
    ProbeResult, ToolCallResult,
};
use super::WsWriter;
use crate::providers;
use crate::tools;

// ── Connection retry helper ─────────────────────────────────────────────────

/// Send an HTTP request with automatic retry on connection errors.
///
/// On the first attempt, uses the provided client (which tries both IPv4
/// and IPv6 per OS defaults).  If that fails with a connection error
/// (e.g. IPv6 unreachable), retries with exponential backoff and jitter.
/// If it still fails with a connect error, attempts one final IPv4-only
/// fallback request.
pub async fn send_with_retry(
    builder: reqwest::RequestBuilder,
    provider: &str,
) -> Result<reqwest::Response> {
    let Some(template) = builder.try_clone() else {
        // Request body is not cloneable (streaming) — can't safely retry.
        return builder.send().await.context("HTTP request failed");
    };

    let policy = RetryPolicy::http_default();

    let result = retry_with_backoff(
        &policy,
        |_attempt| {
            let req = template
                .try_clone()
                .expect("retry template should remain cloneable");
            async move { req.send().await }
        },
        classify_reqwest_result,
        |RetryAttempt {
             attempt,
             delay,
             reason,
         }| {
            eprintln!(
                "[Gateway] transient provider error ({}) on attempt {}, retrying in {:?}",
                reason.as_str(),
                attempt,
                delay,
            );
            metrics::record_retry(provider, reason.as_str(), delay);
        },
    )
    .await;

    match result {
        Ok(resp) => Ok(resp),
        Err(err) if err.is_connect() => {
            eprintln!(
                "[Gateway] connection failed after retries ({}), trying IPv4-only fallback…",
                err
            );
            let ipv4_client = reqwest::Client::builder()
                .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
                .build()
                .context("Failed to build IPv4 client")?;
            let request = template
                .build()
                .context("Failed to rebuild request for IPv4 fallback")?;
            ipv4_client
                .execute(request)
                .await
                .context("IPv4 fallback failed")
        }
        Err(err) => Err(err).context("HTTP request failed after retries"),
    }
}

// ── Streaming helpers ───────────────────────────────────────────────────────

/// Send a single `{"type": "chunk", "delta": "..."}` frame.
pub async fn send_chunk(writer: &mut WsWriter, delta: &str) -> Result<()> {
    let frame = json!({ "type": "chunk", "delta": delta });
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send chunk frame")
}

/// Send a `{"type": "thinking_start"}` frame to indicate extended thinking has begun.
pub async fn send_thinking_start(writer: &mut WsWriter) -> Result<()> {
    let frame = json!({ "type": "thinking_start" });
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send thinking_start frame")
}

/// Send a `{"type": "thinking_delta", "delta": "..."}` frame for streaming thinking content.
pub async fn send_thinking_delta(writer: &mut WsWriter, delta: &str) -> Result<()> {
    let frame = json!({ "type": "thinking_delta", "delta": delta });
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send thinking_delta frame")
}

/// Send a `{"type": "thinking_end"}` frame to indicate extended thinking has finished.
pub async fn send_thinking_end(writer: &mut WsWriter, summary: Option<&str>) -> Result<()> {
    let mut frame = json!({ "type": "thinking_end" });
    if let Some(s) = summary {
        frame["summary"] = json!(s);
    }
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send thinking_end frame")
}

/// Send the `{"type": "response_done"}` sentinel frame.
pub async fn send_response_done(writer: &mut WsWriter) -> Result<()> {
    let frame = json!({ "type": "response_done", "ok": true });
    writer
        .send(Message::Text(frame.to_string().into()))
        .await
        .context("Failed to send response_done frame")
}

/// Attach GitHub-Copilot-required IDE headers to a request builder.
///
/// Uses VS Code / Copilot Chat identifiers that GitHub's API recognizes.
/// The `messages` slice is used to determine whether this is a user-initiated
/// or agent-initiated request (for the `X-Initiator` header).
pub fn apply_copilot_headers(
    builder: reqwest::RequestBuilder,
    provider: &str,
    messages: &[ChatMessage],
) -> reqwest::RequestBuilder {
    if !providers::needs_copilot_session(provider) {
        return builder;
    }
    // Determine X-Initiator based on the last message role.
    // If the last message is from the user, it's user-initiated.
    // If the last message is from assistant/tool, it's agent-initiated.
    let is_agent_call = messages
        .last()
        .map(|m| m.role != "user")
        .unwrap_or(false);
    let x_initiator = if is_agent_call { "agent" } else { "user" };

    // GitHub Copilot requires recognized IDE headers.
    // Using VS Code / Copilot Chat identifiers that the API accepts.
    builder
        .header("User-Agent", "GitHubCopilotChat/0.35.0")
        .header("Editor-Version", "vscode/1.107.0")
        .header("Editor-Plugin-Version", "copilot-chat/0.35.0")
        .header("Copilot-Integration-Id", "vscode-chat")
        .header("Openai-Intent", "conversation-edits")
        .header("X-Initiator", x_initiator)
}

/// Merge an incoming chat request with the gateway's model context.
///
/// Fields present in the request take priority; missing fields fall back
/// to the gateway defaults.  Returns an error message string if a required
/// field cannot be resolved from either source.
pub fn resolve_request(
    req: super::types::ChatRequest,
    ctx: Option<&ModelContext>,
) -> std::result::Result<ProviderRequest, String> {
    let provider = req
        .provider
        .or_else(|| ctx.map(|c| c.provider.clone()))
        .ok_or_else(|| "No provider specified and gateway has no model configured".to_string())?;
    let model = req
        .model
        .or_else(|| ctx.map(|c| c.model.clone()))
        .ok_or_else(|| "No model specified and gateway has no model configured".to_string())?;
    let base_url = req
        .base_url
        .or_else(|| ctx.map(|c| c.base_url.clone()))
        .ok_or_else(|| "No base_url specified and gateway has no model configured".to_string())?;
    let api_key = req
        .api_key
        .map(SecretString::new)
        .or_else(|| ctx.and_then(|c| c.api_key.clone()));

    Ok(ProviderRequest {
        messages: req.messages,
        model,
        provider,
        base_url,
        api_key,
    })
}

/// Append the model's assistant turn and tool results to the conversation
/// so the next round has full context.
/// Append a tool round to the conversation history.
///
/// This adds:
/// 1. An assistant message with the model's text response and tool calls
/// 2. Tool result message(s) with the execution results
///
/// The format varies by provider but the logic is unified here.
pub fn append_tool_round(
    provider: &str,
    messages: &mut Vec<ChatMessage>,
    model_resp: &ModelResponse,
    results: &[ToolCallResult],
) {
    // Build assistant message content based on provider format
    let assistant_content = format_assistant_message(provider, model_resp);
    messages.push(ChatMessage::text("assistant", &assistant_content));

    // Build tool result message(s) based on provider format
    let result_messages = format_tool_results(provider, results);
    for (role, content) in result_messages {
        messages.push(ChatMessage::text(&role, &content));
    }
}

/// Format the assistant message containing text and tool calls.
fn format_assistant_message(provider: &str, model_resp: &ModelResponse) -> String {
    match provider {
        "anthropic" => {
            // Anthropic: array of content blocks (text + tool_use)
            let mut blocks = Vec::new();
            if !model_resp.text.trim().is_empty() {
                blocks.push(json!({ "type": "text", "text": model_resp.text }));
            }
            for tc in &model_resp.tool_calls {
                blocks.push(json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": tc.name,
                    "input": tc.arguments,
                }));
            }
            serde_json::to_string(&blocks).unwrap_or_default()
        }
        "google" => {
            // Google: array of parts (text + functionCall)
            let mut parts = Vec::new();
            if !model_resp.text.trim().is_empty() {
                parts.push(json!({ "text": model_resp.text }));
            }
            for tc in &model_resp.tool_calls {
                parts.push(json!({
                    "functionCall": { "name": tc.name, "args": tc.arguments }
                }));
            }
            serde_json::to_string(&parts).unwrap_or_default()
        }
        _ => {
            // OpenAI-compatible: object with role, content, tool_calls
            let tc_array: Vec<serde_json::Value> = model_resp
                .tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        }
                    })
                })
                .collect();

            json!({
                "role": "assistant",
                "content": if model_resp.text.trim().is_empty() { 
                    serde_json::Value::Null 
                } else { 
                    json!(model_resp.text) 
                },
                "tool_calls": tc_array,
            })
            .to_string()
        }
    }
}

/// Format tool results into message(s).
/// Returns Vec of (role, content) pairs.
fn format_tool_results(provider: &str, results: &[ToolCallResult]) -> Vec<(String, String)> {
    match provider {
        "anthropic" => {
            // Anthropic: single "user" message with array of tool_result blocks
            let blocks: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "type": "tool_result",
                        "tool_use_id": r.id,
                        "content": r.output,
                        "is_error": r.is_error,
                    })
                })
                .collect();
            vec![("user".to_string(), serde_json::to_string(&blocks).unwrap_or_default())]
        }
        "google" => {
            // Google: single "user" message with array of functionResponse parts
            let parts: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "functionResponse": {
                            "name": r.name,
                            "response": { "content": r.output, "is_error": r.is_error }
                        }
                    })
                })
                .collect();
            vec![("user".to_string(), serde_json::to_string(&parts).unwrap_or_default())]
        }
        _ => {
            // OpenAI-compatible: one "tool" message per result
            results
                .iter()
                .map(|r| {
                    let content = json!({
                        "role": "tool",
                        "tool_call_id": r.id,
                        "content": r.output,
                    })
                    .to_string();
                    ("tool".to_string(), content)
                })
                .collect()
        }
    }
}

// ── Context compaction ──────────────────────────────────────────────────────

use super::helpers::estimate_tokens;

/// After compaction, we aim to keep this fraction of the window for fresh context.
const COMPACTION_TARGET: f64 = 0.40;

/// Compact the conversation by summarizing older turns.
///
/// Strategy:
/// 1. Keep the system prompt (first message if role == "system").
/// 2. Keep the most recent turns that fit in COMPACTION_TARGET of the window.
/// 3. Ask the model to produce a concise summary of the middle (old) turns.
/// 4. Replace those old turns with a single assistant "summary" message.
///
/// This modifies `resolved.messages` in-place.
pub async fn compact_conversation(
    http: &reqwest::Client,
    resolved: &mut ProviderRequest,
    context_limit: usize,
    writer: &mut WsWriter,
) -> Result<()> {
    let msgs = &resolved.messages;
    if msgs.len() < 4 {
        // Too few messages to compact meaningfully.
        return Ok(());
    }

    // Separate system prompt from the rest.
    let has_system = msgs.first().is_some_and(|m| m.role == "system");
    let start_idx = if has_system { 1 } else { 0 };

    // Walk backwards to find how many recent turns fit in the target budget.
    let target_tokens = (context_limit as f64 * COMPACTION_TARGET) as usize;
    let mut tail_tokens = 0usize;
    let mut keep_from = msgs.len(); // index where "recent" messages start
    for i in (start_idx..msgs.len()).rev() {
        let msg_tokens = (msgs[i].role.len() + msgs[i].content.len()) / 3;
        if tail_tokens + msg_tokens > target_tokens {
            break;
        }
        tail_tokens += msg_tokens;
        keep_from = i;
    }

    // The middle section to summarize: everything between system and keep_from.
    if keep_from <= start_idx + 1 {
        // Nothing meaningful to summarize.
        return Ok(());
    }

    let old_turns = &msgs[start_idx..keep_from];

    // Build a summary prompt.
    let mut summary_text = String::from(
        "Summarize the following conversation turns into a concise context recap. \
         Preserve key facts, decisions, file paths, tool results, and user preferences. \
         Keep it under 500 words. Output only the summary, no preamble.\n\n",
    );
    for m in old_turns {
        // Truncate very large tool results to avoid blowing up the summary request.
        let content = if m.content.len() > 2000 {
            format!("{}… [truncated]", &m.content[..2000])
        } else {
            m.content.clone()
        };
        summary_text.push_str(&format!("[{}]: {}\n\n", m.role, content));
    }

    // Call the model to produce the summary (simple request, no tools).
    let summary_req = ProviderRequest {
        messages: vec![ChatMessage::text("user", &summary_text)],
        model: resolved.model.clone(),
        provider: resolved.provider.clone(),
        base_url: resolved.base_url.clone(),
        api_key: resolved.api_key.clone(),
    };

    let summary_result = if resolved.provider == "anthropic" {
        call_anthropic_with_tools(http, &summary_req, None).await
    } else if resolved.provider == "google" {
        call_google_with_tools(http, &summary_req).await
    } else {
        call_openai_with_tools(http, &summary_req).await
    };

    let summary = match summary_result {
        Ok(resp) if !resp.text.is_empty() => resp.text,
        Ok(_) => anyhow::bail!("Model returned empty summary"),
        Err(e) => anyhow::bail!("Summary request failed: {}", e),
    };

    // Rebuild messages: system + summary + recent turns.
    let mut new_messages = Vec::new();
    if has_system {
        new_messages.push(msgs[0].clone());
    }
    new_messages.push(ChatMessage::text(
        "assistant",
        &format!(
            "[Conversation summary — older messages were compacted to save context]\n\n{}",
            summary,
        ),
    ));
    new_messages.extend_from_slice(&msgs[keep_from..]);

    let old_count = msgs.len();
    let new_count = new_messages.len();
    let old_tokens = estimate_tokens(msgs);
    let new_tokens = estimate_tokens(&new_messages);

    resolved.messages = new_messages;

    // Notify the client.
    let info_frame = json!({
        "type": "info",
        "message": format!(
            "Context compacted: {} → {} messages (~{}k → ~{}k tokens)",
            old_count,
            new_count,
            old_tokens / 1000,
            new_tokens / 1000,
        ),
    });
    writer
        .send(Message::Text(info_frame.to_string().into()))
        .await
        .context("Failed to send compaction info frame")?;

    Ok(())
}

// ── Model connection probe ──────────────────────────────────────────────────

/// Validate the model connection by probing the provider.
///
/// The probe strategy differs by provider:
/// - **OpenAI-compatible**: `GET /models` — an auth-only check that does
///   not send a chat request, avoiding model-format mismatches.
/// - **Anthropic**: `POST /v1/messages` with `max_tokens: 1`.
/// - **Google Gemini**: `GET /models/{model}` metadata endpoint.
///
/// For Copilot providers the optional [`CopilotSession`] is used to
/// exchange the OAuth token for a session token before probing.
///
/// Returns a [`ProbeResult`] that lets the caller distinguish between
/// "fully ready", "connected with a warning", and "hard failure".
pub async fn validate_model_connection(
    http: &reqwest::Client,
    ctx: &ModelContext,
    copilot_session: Option<&CopilotSession>,
) -> ProbeResult {
    // Resolve the bearer token (session token for Copilot, raw key otherwise).
    let effective_key = match super::auth::resolve_bearer_token(
        http,
        &ctx.provider,
        ctx.api_key.as_ref().map(|v| v.expose_secret()),
        copilot_session,
    )
    .await
    {
        Ok(k) => k,
        Err(err) => {
            return ProbeResult::AuthError {
                detail: format!("Token exchange failed: {}", err),
            };
        }
    };

    let result: Result<reqwest::Response> = if ctx.provider == "anthropic" {
        // Anthropic has no /models list endpoint — use a minimal chat.
        let url = format!("{}/v1/messages", ctx.base_url.trim_end_matches('/'));
        let body = json!({
            "model": ctx.model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "Hi"}],
        });
        let builder = http.post(&url)
            .header(
                "x-api-key",
                ctx.api_key
                    .as_ref()
                    .map(|v| v.expose_secret())
                    .unwrap_or(""),
            )
            .header("anthropic-version", "2023-06-01")
            .json(&body);
        send_with_retry(builder, &ctx.provider).await
    } else if ctx.provider == "google" {
        // Google: check the model metadata endpoint (no chat needed).
        let key = ctx
            .api_key
            .as_ref()
            .map(|v| v.expose_secret())
            .unwrap_or("");
        let url = format!(
            "{}/models/{}?key={}",
            ctx.base_url.trim_end_matches('/'),
            ctx.model,
            key,
        );
        send_with_retry(http.get(&url), &ctx.provider).await
    } else {
        // OpenAI-compatible: GET /models — lightweight auth check.
        let url = format!("{}/models", ctx.base_url.trim_end_matches('/'));
        let mut builder = http.get(&url);
        if let Some(ref key) = effective_key {
            builder = builder.bearer_auth(key);
        }
        builder = apply_copilot_headers(builder, &ctx.provider, &[]);
        send_with_retry(builder, &ctx.provider).await
    };

    match result {
        Ok(resp) if resp.status().is_success() => ProbeResult::Ready,
        Ok(resp) => {
            let status = resp.status();
            let code = status.as_u16();
            let body = resp.text().await.unwrap_or_default();

            // Try to extract a human-readable error message from JSON.
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message").or(Some(e)))
                        .and_then(|m| m.as_str().map(String::from))
                })
                .unwrap_or(body);

            match code {
                401 | 403 => ProbeResult::AuthError {
                    detail: format!("{} — {}", status, detail),
                },
                // 400, 404, 422 etc — the server answered, auth is fine,
                // but something about the request/model wasn't accepted.
                // Chat may still work with the full request format.
                400..=499 => ProbeResult::Connected {
                    warning: format!("{} — {}", status, detail),
                },
                _ => ProbeResult::Unreachable {
                    detail: format!("{} — {}", status, detail),
                },
            }
        }
        Err(err) => ProbeResult::Unreachable {
            detail: err.to_string(),
        },
    }
}

// ── Provider-specific callers ───────────────────────────────────────────────

/// Parse SSE text that was already fully received (not streaming).
/// This handles cases where the response was buffered as text but contains SSE format.
fn consume_sse_text(text: &str) -> Result<serde_json::Value> {
    let mut content = String::new();
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();
    let mut finish_reason: Option<String> = None;
    let mut usage: Option<serde_json::Value> = None;
    let mut model = String::new();

    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if data.trim() == "[DONE]" {
                break;
            }

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                // Extract model name
                if let Some(m) = json.get("model").and_then(|v| v.as_str()) {
                    model = m.to_string();
                }

                // Extract usage if present
                if let Some(u) = json.get("usage") {
                    if !u.is_null() {
                        usage = Some(u.clone());
                    }
                }

                // Process choices
                if let Some(choices) = json.get("choices").and_then(|v| v.as_array()) {
                    for choice in choices {
                        if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                            finish_reason = Some(fr.to_string());
                        }

                        if let Some(delta) = choice.get("delta") {
                            if let Some(c) = delta.get("content").and_then(|v| v.as_str()) {
                                content.push_str(c);
                            }

                            if let Some(tc_array) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                                for tc in tc_array {
                                    let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                    while tool_calls.len() <= index {
                                        tool_calls.push(json!({
                                            "id": "",
                                            "type": "function",
                                            "function": { "name": "", "arguments": "" }
                                        }));
                                    }
                                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                        tool_calls[index]["id"] = json!(id);
                                    }
                                    if let Some(func) = tc.get("function") {
                                        if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                            tool_calls[index]["function"]["name"] = json!(name);
                                        }
                                        if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                                            let existing = tool_calls[index]["function"]["arguments"]
                                                .as_str()
                                                .unwrap_or("");
                                            tool_calls[index]["function"]["arguments"] =
                                                json!(format!("{}{}", existing, args));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Count incomplete tool calls for debugging
    let incomplete_count = tool_calls.iter().filter(|tc| {
        let id = tc["id"].as_str().unwrap_or("");
        let name = tc["function"]["name"].as_str().unwrap_or("");
        id.is_empty() || name.is_empty()
    }).count();

    // Filter out incomplete tool calls (missing id or name)
    let tool_calls: Vec<serde_json::Value> = tool_calls
        .into_iter()
        .filter(|tc| {
            let id = tc["id"].as_str().unwrap_or("");
            let name = tc["function"]["name"].as_str().unwrap_or("");
            !id.is_empty() && !name.is_empty()
        })
        .collect();

    // Log if we filtered any tool calls (for debugging stall issues)
    if incomplete_count > 0 {
        eprintln!(
            "[SSE] Filtered {} incomplete tool calls (text), {} remaining",
            incomplete_count,
            tool_calls.len()
        );
    }

    let mut message = json!({
        "role": "assistant",
        "content": if content.trim().is_empty() { serde_json::Value::Null } else { json!(content) }
    });

    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    let mut response = json!({
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason.unwrap_or_else(|| "stop".to_string())
        }]
    });

    if let Some(u) = usage {
        response["usage"] = u;
    }

    Ok(response)
}

/// Consume an SSE (Server-Sent Events) stream and reassemble it into
/// an OpenAI-compatible JSON response structure.
///
/// This handles the case where a provider returns a streaming response
/// even though we didn't request `"stream": true`.
async fn consume_sse_stream(resp: reqwest::Response) -> Result<serde_json::Value> {
    use futures_util::StreamExt;
    use std::time::Duration;
    use tokio::time::timeout;

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();

    // Accumulated response fields
    let mut content = String::new();
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();
    let mut finish_reason: Option<String> = None;
    let mut usage: Option<serde_json::Value> = None;
    let mut model = String::new();

    // Timeout for waiting on next chunk — if exceeded, assume stream is done
    let chunk_timeout = Duration::from_secs(30);

    'outer: loop {
        // Wait for next chunk with timeout
        let chunk_result = match timeout(chunk_timeout, stream.next()).await {
            Ok(Some(result)) => result,
            Ok(None) => {
                eprintln!("[SSE] Stream ended normally (None)");
                break 'outer;
            }
            Err(_) => {
                // Timeout — stream stalled, return what we have
                eprintln!("[SSE] Stream timeout after {}s", chunk_timeout.as_secs());
                break 'outer;
            }
        };

        let chunk = chunk_result.context("SSE stream read error")?;
        let chunk_str = String::from_utf8_lossy(&chunk);
        
        buffer.push_str(&chunk_str);

        // Process complete SSE events (terminated by double newline)
        while let Some(event_end) = buffer.find("\n\n") {
            let event = buffer[..event_end].to_string();
            buffer = buffer[event_end + 2..].to_string();

            for line in event.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data.trim() == "[DONE]" {
                        // Stream complete — exit all loops
                        eprintln!("[SSE] Received [DONE], content len={}, tool_calls={}", content.len(), tool_calls.len());
                        break 'outer;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        // Extract model name
                        if let Some(m) = json.get("model").and_then(|v| v.as_str()) {
                            model = m.to_string();
                        }

                        // Extract usage if present (usually in final chunk)
                        if let Some(u) = json.get("usage") {
                            if !u.is_null() {
                                usage = Some(u.clone());
                            }
                        }

                        // Track if this chunk signals completion
                        let mut should_exit = false;

                        // Process choices
                        if let Some(choices) = json.get("choices").and_then(|v| v.as_array()) {
                            for choice in choices {
                                // Extract delta content FIRST (before checking finish_reason)
                                if let Some(delta) = choice.get("delta") {
                                    // Text content
                                    if let Some(c) = delta.get("content").and_then(|v| v.as_str()) {
                                        content.push_str(c);
                                    }

                                    // Tool calls (streamed incrementally)
                                    if let Some(tc_array) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                                        for tc in tc_array {
                                            let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

                                            // Ensure tool_calls vec is big enough
                                            while tool_calls.len() <= index {
                                                tool_calls.push(json!({
                                                    "id": "",
                                                    "type": "function",
                                                    "function": { "name": "", "arguments": "" }
                                                }));
                                            }

                                            // Update tool call fields
                                            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                                tool_calls[index]["id"] = json!(id);
                                            }
                                            if let Some(func) = tc.get("function") {
                                                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                                    tool_calls[index]["function"]["name"] = json!(name);
                                                }
                                                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                                                    // Append to existing arguments
                                                    let existing = tool_calls[index]["function"]["arguments"]
                                                        .as_str()
                                                        .unwrap_or("");
                                                    tool_calls[index]["function"]["arguments"] =
                                                        json!(format!("{}{}", existing, args));
                                                }
                                            }
                                        }
                                    }
                                }

                                // Check finish_reason AFTER extracting delta data
                                if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                                    finish_reason = Some(fr.to_string());
                                    // Terminal reasons mean the model is done
                                    if fr == "stop" || fr == "tool_calls" || fr == "tool_use" || fr == "length" || fr == "end_turn" {
                                        eprintln!(
                                            "[SSE] finish_reason='{}', content len={}, tool_calls={}",
                                            fr, content.len(), tool_calls.len()
                                        );
                                        should_exit = true;
                                    }
                                }
                            }
                        }

                        // Exit after processing all data in this chunk
                        if should_exit {
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    // Count incomplete tool calls for debugging
    let incomplete_count = tool_calls.iter().filter(|tc| {
        let id = tc["id"].as_str().unwrap_or("");
        let name = tc["function"]["name"].as_str().unwrap_or("");
        id.is_empty() || name.is_empty()
    }).count();

    // Filter out incomplete tool calls (missing id or name)
    let tool_calls: Vec<serde_json::Value> = tool_calls
        .into_iter()
        .filter(|tc| {
            let id = tc["id"].as_str().unwrap_or("");
            let name = tc["function"]["name"].as_str().unwrap_or("");
            !id.is_empty() && !name.is_empty()
        })
        .collect();

    // Log if we filtered any tool calls (for debugging stall issues)
    if incomplete_count > 0 {
        eprintln!(
            "[SSE] Filtered {} incomplete tool calls (stream), {} remaining",
            incomplete_count,
            tool_calls.len()
        );
    }

    // Build a standard OpenAI-style response object
    let mut message = json!({
        "role": "assistant",
        "content": if content.trim().is_empty() { serde_json::Value::Null } else { json!(content) }
    });

    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    let mut response = json!({
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason.unwrap_or_else(|| "stop".to_string())
        }]
    });

    if let Some(u) = usage {
        response["usage"] = u;
    }

    Ok(response)
}

/// Call an OpenAI-compatible `/chat/completions` endpoint (non-streaming)
/// with tool definitions.  Returns structured text + tool calls.
pub async fn call_openai_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
) -> Result<ModelResponse> {
    let url = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));

    // Build the messages array.  Most messages are simple role+content,
    // but tool-loop continuation messages have structured JSON content
    // that must be sent as raw objects rather than string-escaped.
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            // Try to parse content as JSON first (for assistant messages
            // with tool_calls and tool-result messages).
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_object() && parsed.get("role").is_some() {
                    return parsed;
                }
            }
            json!({ "role": m.role, "content": m.content })
        })
        .collect();

    let tool_defs = tools::tools_openai();

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !tool_defs.is_empty() {
        body["tools"] = json!(tool_defs);
    }

    let mut builder = http.post(&url).json(&body);
    if let Some(ref key) = req.api_key {
        builder = builder.bearer_auth(key.expose_secret());
    }
    builder = apply_copilot_headers(builder, &req.provider, &req.messages);

    let resp = send_with_retry(builder, &req.provider).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Provider returned {} — {}", status, text);
    }

    // Check if the server returned a streaming response (SSE) despite us
    // not requesting one.  Some providers (e.g. GitHub Copilot) may force
    // streaming.  If so, consume the SSE stream and reassemble the response.
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Detect SSE by content-type (may include charset, e.g., "text/event-stream; charset=utf-8")
    let data: serde_json::Value = if content_type.contains("text/event-stream") {
        // Server is streaming — parse SSE events.
        consume_sse_stream(resp).await?
    } else {
        // Normal JSON response — but check if it actually looks like SSE
        let text = resp.text().await.context("Failed to read response body")?;
        
        if text.trim_start().starts_with("data:") {
            // Looks like SSE despite content-type — parse it
            consume_sse_text(&text)?
        } else {
            serde_json::from_str(&text).context("Invalid JSON from provider")?
        }
    };

    let choice = &data["choices"][0];
    let message = &choice["message"];

    let mut result = ModelResponse::default();

    // Extract finish_reason
    if let Some(fr) = choice["finish_reason"].as_str() {
        result.finish_reason = Some(fr.to_string());
    }

    // Extract text content (ignore whitespace-only content to avoid API errors).
    if let Some(text) = message["content"].as_str() {
        if !text.trim().is_empty() {
            result.text = text.to_string();
        }
    }

    // Extract tool calls (skip incomplete ones with empty id or name).
    if let Some(tc_array) = message["tool_calls"].as_array() {
        for tc in tc_array {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            
            // Skip tool calls with missing id or name
            if id.is_empty() || name.is_empty() {
                continue;
            }
            
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let arguments = serde_json::from_str(args_str).unwrap_or(json!({}));
            result.tool_calls.push(ParsedToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    // Extract token usage if present.
    if let Some(usage) = data.get("usage") {
        result.prompt_tokens = usage["prompt_tokens"].as_u64();
        result.completion_tokens = usage["completion_tokens"].as_u64();
    }

    Ok(result)
}

/// Call the Anthropic Messages API with tool definitions.
/// 
/// When `writer` is provided, streams thinking and text deltas to the TUI
/// in real-time. When `None`, operates in batch mode (for internal calls
/// like context compaction).
///
/// Extended thinking is automatically enabled for supported models when
/// the model name contains "opus" or "sonnet" and the request appears
/// complex enough to benefit from reasoning.
pub async fn call_anthropic_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
    mut writer: Option<&mut WsWriter>,
) -> Result<ModelResponse> {
    use futures_util::StreamExt;

    let url = format!("{}/v1/messages", req.base_url.trim_end_matches('/'));

    let system = req
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    // Build messages.  Tool-loop continuation messages have structured
    // JSON content (content blocks) that must be sent as arrays.
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            // Try to parse content as a JSON array (content blocks).
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_array() {
                    return json!({ "role": m.role, "content": parsed });
                }
            }
            json!({ "role": m.role, "content": m.content })
        })
        .collect();

    let tool_defs = tools::tools_anthropic();

    // Use streaming when we have a writer to forward chunks to
    let use_streaming = writer.is_some();
    
    // Increase max_tokens when streaming to allow for longer responses
    let max_tokens = if use_streaming { 16384 } else { 4096 };

    let mut body = json!({
        "model": req.model,
        "max_tokens": max_tokens,
        "messages": messages,
        "stream": use_streaming,
    });
    
    if !system.is_empty() {
        body["system"] = serde_json::Value::String(system);
    }
    if !tool_defs.is_empty() {
        body["tools"] = json!(tool_defs);
    }

    // Send immediate "waiting" indicator BEFORE the HTTP request
    // This is where the model processing time is spent
    if let Some(ref mut w) = writer {
        let waiting_frame = json!({ "type": "stream_start" });
        let _ = w.send(Message::Text(waiting_frame.to_string().into())).await;
    }

    let api_key = req
        .api_key
        .as_ref()
        .map(|v| v.expose_secret())
        .unwrap_or("");
    let builder = http
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body);
    let resp = send_with_retry(builder, &req.provider).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic returned {} — {}", status, text);
    }

    // Non-streaming path (for internal calls like compaction)
    if !use_streaming {
        let data: serde_json::Value = resp.json().await.context("Invalid JSON from Anthropic")?;
        return parse_anthropic_response(&data);
    }

    // Streaming path — parse SSE and forward to TUI
    let writer = writer.unwrap();
    
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    
    // Accumulated response
    let mut result = ModelResponse::default();
    let mut current_tool_index = 0;
    let mut in_thinking_block = false;
    let mut thinking_content = String::new();
    let mut tool_args_buffer: std::collections::HashMap<usize, String> = std::collections::HashMap::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("Stream read error")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(event_end) = buffer.find("\n\n") {
            let event = buffer[..event_end].to_string();
            buffer = buffer[event_end + 2..].to_string();

            let mut event_type = String::new();
            let mut event_data = String::new();

            for line in event.lines() {
                if let Some(typ) = line.strip_prefix("event: ") {
                    event_type = typ.to_string();
                } else if let Some(data) = line.strip_prefix("data: ") {
                    event_data = data.to_string();
                }
            }

            // Debug: log event types we receive
            eprintln!("[Anthropic SSE] event_type='{}', data_len={}", event_type, event_data.len());

            if event_data.is_empty() {
                continue;
            }

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&event_data) {
                match event_type.as_str() {
                    "message_start" => {
                        // Extract usage from message start if present
                        if let Some(usage) = json.get("message").and_then(|m| m.get("usage")) {
                            result.prompt_tokens = usage["input_tokens"].as_u64();
                        }
                    }
                    "content_block_start" => {
                        if let Some(block) = json.get("content_block") {
                            match block["type"].as_str() {
                                Some("thinking") => {
                                    // Extended thinking block started
                                    in_thinking_block = true;
                                    thinking_content.clear();
                                    let _ = send_thinking_start(writer).await;
                                }
                                Some("tool_use") => {
                                    let id = block["id"].as_str().unwrap_or("").to_string();
                                    let name = block["name"].as_str().unwrap_or("").to_string();
                                    current_tool_index = json["index"].as_u64().unwrap_or(0) as usize;
                                    
                                    // Initialize tool call
                                    result.tool_calls.push(ParsedToolCall {
                                        id,
                                        name,
                                        arguments: json!({}),
                                    });
                                    tool_args_buffer.insert(current_tool_index, String::new());
                                }
                                Some("text") => {
                                    // Regular text block - nothing special to do on start
                                }
                                _ => {}
                            }
                        }
                    }
                    "content_block_delta" => {
                        if let Some(delta) = json.get("delta") {
                            match delta["type"].as_str() {
                                Some("thinking_delta") => {
                                    // Extended thinking content streaming
                                    if let Some(thinking) = delta["thinking"].as_str() {
                                        thinking_content.push_str(thinking);
                                        let _ = send_thinking_delta(writer, thinking).await;
                                    }
                                }
                                Some("text_delta") => {
                                    if let Some(text) = delta["text"].as_str() {
                                        result.text.push_str(text);
                                        let _ = send_chunk(writer, text).await;
                                    }
                                }
                                Some("input_json_delta") => {
                                    if let Some(partial) = delta["partial_json"].as_str() {
                                        if let Some(buf) = tool_args_buffer.get_mut(&current_tool_index) {
                                            buf.push_str(partial);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "content_block_stop" => {
                        // A content block finished
                        if in_thinking_block {
                            in_thinking_block = false;
                            // Generate a brief summary from the thinking content
                            let summary = if thinking_content.len() > 100 {
                                let truncated = &thinking_content[..100];
                                if let Some(period_pos) = truncated.find(". ") {
                                    Some(&truncated[..=period_pos])
                                } else {
                                    Some(truncated)
                                }
                            } else if !thinking_content.is_empty() {
                                Some(thinking_content.as_str())
                            } else {
                                None
                            };
                            let _ = send_thinking_end(writer, summary).await;
                        }
                        
                        // Finalize tool call arguments
                        let block_index = json["index"].as_u64().unwrap_or(0) as usize;
                        if let Some(args_str) = tool_args_buffer.remove(&block_index) {
                            if !args_str.is_empty() {
                                if let Some(tc) = result.tool_calls.get_mut(block_index) {
                                    tc.arguments = serde_json::from_str(&args_str).unwrap_or(json!({}));
                                }
                            }
                        }
                    }
                    "message_delta" => {
                        // Extract stop_reason and usage from message delta
                        if let Some(delta) = json.get("delta") {
                            if let Some(sr) = delta["stop_reason"].as_str() {
                                result.finish_reason = Some(sr.to_string());
                            }
                        }
                        if let Some(usage) = json.get("usage") {
                            result.completion_tokens = usage["output_tokens"].as_u64();
                        }
                    }
                    "message_stop" => {
                        // Stream complete
                        return Ok(result);
                    }
                    "error" => {
                        let msg = json["error"]["message"]
                            .as_str()
                            .unwrap_or("Unknown error");
                        anyhow::bail!("Anthropic stream error: {}", msg);
                    }
                    _ => {
                        eprintln!("[Anthropic SSE] Unhandled event type: '{}'", event_type);
                    }
                }
            }
        }
    }

    // Debug: log final result
    eprintln!(
        "[Anthropic SSE] Stream complete: text_len={}, tool_calls={}, finish_reason={:?}",
        result.text.len(),
        result.tool_calls.len(),
        result.finish_reason
    );

    Ok(result)
}

/// Parse a non-streaming Anthropic response into ModelResponse.
fn parse_anthropic_response(data: &serde_json::Value) -> Result<ModelResponse> {
    let mut result = ModelResponse::default();

    // Extract stop_reason (Anthropic's equivalent of finish_reason)
    if let Some(sr) = data["stop_reason"].as_str() {
        result.finish_reason = Some(sr.to_string());
    }

    if let Some(content) = data["content"].as_array() {
        for block in content {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        if !result.text.is_empty() {
                            result.text.push('\n');
                        }
                        result.text.push_str(text);
                    }
                }
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or("").to_string();
                    let name = block["name"].as_str().unwrap_or("").to_string();
                    let arguments = block["input"].clone();
                    result.tool_calls.push(ParsedToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                _ => {}
            }
        }
    }

    // Extract token usage if present.
    if let Some(usage) = data.get("usage") {
        result.prompt_tokens = usage["input_tokens"].as_u64();
        result.completion_tokens = usage["output_tokens"].as_u64();
    }

    Ok(result)
}

/// Call Google Gemini with function declarations (non-streaming).
pub async fn call_google_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
) -> Result<ModelResponse> {
    let api_key = req
        .api_key
        .as_ref()
        .map(|v| v.expose_secret())
        .unwrap_or("");
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        req.base_url.trim_end_matches('/'),
        req.model,
        api_key,
    );

    let system = req
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    // Build contents.  Tool-loop continuation messages may have
    // structured JSON parts that need to be sent as arrays.
    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let role = if m.role == "assistant" { "model" } else { "user" };
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_array() {
                    return json!({ "role": role, "parts": parsed });
                }
            }
            json!({ "role": role, "parts": [{ "text": m.content }] })
        })
        .collect();

    let tool_defs = tools::tools_google();

    let mut body = json!({ "contents": contents });
    if !system.is_empty() {
        body["system_instruction"] = json!({ "parts": [{ "text": system }] });
    }
    if !tool_defs.is_empty() {
        body["tools"] = json!([{ "function_declarations": tool_defs }]);
    }

    let builder = http
        .post(&url)
        .json(&body);
    let resp = send_with_retry(builder, &req.provider).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Google returned {} — {}", status, text);
    }

    let data: serde_json::Value = resp.json().await.context("Invalid JSON from Google")?;

    let mut result = ModelResponse::default();

    // Extract finishReason from Google's format
    if let Some(fr) = data["candidates"][0]["finishReason"].as_str() {
        // Convert Google's format to OpenAI-style (STOP -> stop, etc.)
        result.finish_reason = Some(fr.to_lowercase());
    }

    if let Some(parts) = data["candidates"][0]["content"]["parts"].as_array() {
        for (i, part) in parts.iter().enumerate() {
            if let Some(text) = part["text"].as_str() {
                if !result.text.is_empty() {
                    result.text.push('\n');
                }
                result.text.push_str(text);
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc["name"].as_str().unwrap_or("").to_string();
                let arguments = fc["args"].clone();
                result.tool_calls.push(ParsedToolCall {
                    id: format!("google_call_{}", i),
                    name,
                    arguments,
                });
            }
        }
    }

    // Extract token usage if present.
    if let Some(usage) = data.get("usageMetadata") {
        result.prompt_tokens = usage["promptTokenCount"].as_u64();
        result.completion_tokens = usage["candidatesTokenCount"].as_u64();
    }

    Ok(result)
}

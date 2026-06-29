use std::collections::HashSet;

use anyhow::{Context, Result};
use serde_json::json;
use tracing::debug;

use rustyclaw_core::error_details::{ErrorLike, RequestDetails};
use rustyclaw_core::gateway::protocol::server;
use rustyclaw_core::gateway::transport::TransportWriter;
use rustyclaw_core::gateway::{
    ChatMessage, CopilotSession, ModelContext, ModelResponse, ParsedToolCall, ProbeResult,
    ProviderRequest, ToolCallResult,
};
use rustyclaw_core::providers;

// ── Connection retry helper ─────────────────────────────────────────────────

/// Send an HTTP request with automatic retry on connection errors.
///
/// On the first attempt, uses the provided client (which tries both IPv4
/// and IPv6 per OS defaults).  If that fails with a connection error
/// (e.g. IPv6 unreachable), retries once with an IPv4-only client.
///
/// This avoids hardcoding an IPv4 preference while still recovering from
/// broken IPv6 connectivity — a common issue with some providers.
pub async fn send_with_retry(builder: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    match builder.try_clone() {
        Some(cloned) => {
            match builder.send().await {
                Ok(resp) => Ok(resp),
                Err(e) if e.is_connect() => {
                    debug!(error = %e, "Connection failed, retrying with IPv4-only");
                    // Build an IPv4-only client for the retry
                    let ipv4_client = reqwest::Client::builder()
                        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
                        .build()
                        .context("Failed to build IPv4 client")?;
                    // Re-issue the request through the IPv4 client.
                    // try_clone gave us a copy of the original request builder,
                    // but it's bound to the original client.  We need to
                    // rebuild from the cloned builder's inner request.
                    // Unfortunately RequestBuilder::try_clone clones the
                    // builder but keeps the same client.  So we send via
                    // the clone which still uses the default client — not
                    // helpful.  Instead, we extract the Request and execute
                    // it on the new client.
                    let request = cloned.build().context("Failed to rebuild request")?;
                    ipv4_client
                        .execute(request)
                        .await
                        .context("IPv4 retry also failed")
                }
                Err(e) => Err(e).context("HTTP request failed"),
            }
        }
        None => {
            // Request body is not cloneable (streaming) — can't retry
            builder.send().await.context("HTTP request failed")
        }
    }
}

// ── Streaming helpers ───────────────────────────────────────────────────────

/// Send a single chunk frame as binary.
pub async fn send_chunk(writer: &mut dyn TransportWriter, delta: &str) -> Result<()> {
    server::send_chunk(writer, delta)
        .await
        .context("Failed to send chunk frame")
}

/// Send the response_done sentinel frame as binary.
pub async fn send_response_done(writer: &mut dyn TransportWriter) -> Result<()> {
    server::send_response_done(writer, true)
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
    let is_agent_call = messages.last().map(|m| m.role != "user").unwrap_or(false);
    let x_initiator = if is_agent_call { "agent" } else { "user" };

    // GitHub Copilot requires recognized IDE headers.
    // Using VS Code / Copilot Chat identifiers that the API accepts.
    builder
        .header("User-Agent", providers::COPILOT_API_USER_AGENT)
        .header("Editor-Version", providers::COPILOT_EDITOR_VERSION)
        .header(
            "Editor-Plugin-Version",
            providers::COPILOT_EDITOR_PLUGIN_VERSION,
        )
        .header("Copilot-Integration-Id", providers::COPILOT_INTEGRATION_ID)
        .header("Openai-Intent", "conversation-edits")
        .header("X-Initiator", x_initiator)
}

/// Merge an incoming chat request with the gateway's model context.
///
/// Fields present in the request take priority; missing fields fall back
/// to the gateway defaults.  Returns an error message string if a required
/// field cannot be resolved from either source.
pub fn resolve_request(
    req: rustyclaw_core::gateway::ChatRequest,
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
    let api_key = req.api_key.or_else(|| ctx.and_then(|c| c.api_key.clone()));

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

/// Encode the assistant turn (text + tool calls) into RustyClaw's canonical,
/// provider-agnostic envelope.
///
/// The encoding now lives in `rustyclaw-core` alongside the genai decoder, so
/// the on-wire contract has a single owner; this delegates to it. The
/// `_provider` argument is retained for call-site compatibility but no longer
/// affects the output.
fn format_assistant_message(_provider: &str, model_resp: &ModelResponse) -> String {
    providers::encode_assistant_message(model_resp)
}

/// Encode tool results into canonical `tool_result` messages — one `tool`
/// message per result. Provider-agnostic (see [`format_assistant_message`]).
fn format_tool_results(_provider: &str, results: &[ToolCallResult]) -> Vec<(String, String)> {
    results
        .iter()
        .map(|r| ("tool".to_string(), providers::encode_tool_result(r)))
        .collect()
}

/// Convert persisted [`ThreadMessage`]s into the [`ChatMessage`] wire form
/// expected by the provider.
///
/// Thread history stores assistant turns and tool results in a normalized,
/// provider-agnostic shape (text + `tool_calls` JSON on assistant messages;
/// `tool_call_id` on tool messages). The provider request builders, however,
/// expect tool-loop continuation messages to carry their structured payload
/// as a JSON-encoded `content` string (see [`format_assistant_message`] and
/// [`format_tool_results`]).
///
/// Without this re-encoding, an assistant message that issued tool calls
/// would be sent without its `tool_calls` field and the following tool
/// messages would be sent as plain text — causing OpenAI-compatible
/// providers to reject the request with:
/// `messages with role 'tool' must be a response to a preceding message
/// with 'tool_calls'`.
pub fn thread_history_to_chat_messages(
    provider: &str,
    history: &[rustyclaw_core::threads::ThreadMessage],
) -> Vec<ChatMessage> {
    use rustyclaw_core::threads::MessageRole;

    let mut out: Vec<ChatMessage> = Vec::with_capacity(history.len());
    // Track seen tool_use IDs across the entire conversation to detect and
    // fix duplicates. Some OpenAI-compatible adapters generate non-unique IDs
    // (e.g. "call_0", "call_1") per turn, which violates the Anthropic API's
    // requirement that tool_use IDs be unique across the full message array.
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut i = 0;
    while i < history.len() {
        let m = &history[i];
        match m.role {
            MessageRole::User => {
                out.push(ChatMessage::text("user", &m.content));
                i += 1;
            }
            MessageRole::System => {
                out.push(ChatMessage::text("system", &m.content));
                i += 1;
            }
            MessageRole::Assistant => {
                // Reconstruct ParsedToolCall list from the stored JSON.
                let tool_calls: Vec<ParsedToolCall> = m
                    .tool_calls
                    .as_ref()
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|tc| {
                                let id = tc.get("id")?.as_str()?.to_string();
                                let name = tc.get("name")?.as_str()?.to_string();
                                // Arguments might be stored as a JSON string (from wire
                                // protocol) or as a JSON object. Normalize to object.
                                let arguments = tc.get("arguments")
                                    .and_then(|v| {
                                        // If it's already an object, use it
                                        if v.is_object() || v.is_array() {
                                            Some(v.clone())
                                        } else if let Some(s) = v.as_str() {
                                            // If it's a string, try to parse it as JSON
                                            match serde_json::from_str(s) {
                                                Ok(parsed) => Some(parsed),
                                                Err(e) => {
                                                    tracing::warn!(
                                                        error = %e,
                                                        tool_id = %id,
                                                        tool_name = %name,
                                                        arguments_str = %s.chars().take(100).collect::<String>(),
                                                        "Failed to parse tool call arguments from string"
                                                    );
                                                    None
                                                }
                                            }
                                        } else {
                                            tracing::debug!(
                                                tool_id = %id,
                                                tool_name = %name,
                                                arguments_type = ?v,
                                                "Tool call arguments in unexpected format"
                                            );
                                            None
                                        }
                                    })
                                    .unwrap_or_else(|| json!({}));
                                Some(ParsedToolCall {
                                    id,
                                    name,
                                    arguments,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                if tool_calls.is_empty() {
                    // Plain assistant text turn.
                    out.push(ChatMessage::text("assistant", &m.content));
                    i += 1;
                    continue;
                }

                // Deduplicate tool call IDs: if an ID was already used in
                // an earlier assistant turn, generate a unique replacement.
                //
                // We track the final (deduplicated) IDs in order so that
                // tool_results can be matched POSITIONALLY — a HashMap
                // would incorrectly remap ALL results sharing the same
                // original ID to the same new ID when a single turn has
                // multiple tool_calls with identical IDs.
                let tool_calls: Vec<ParsedToolCall> = tool_calls
                    .into_iter()
                    .map(|mut tc| {
                        if seen_ids.contains(&tc.id) {
                            // Generate a unique ID by appending a disambiguator.
                            let mut new_id = format!("{}_{:x}", tc.id, seen_ids.len());
                            while seen_ids.contains(&new_id) {
                                new_id.push('x');
                            }
                            tracing::debug!(
                                old_id = %tc.id,
                                new_id = %new_id,
                                tool_name = %tc.name,
                                "Remapping duplicate tool_use ID"
                            );
                            tc.id = new_id;
                        }
                        seen_ids.insert(tc.id.clone());
                        tc
                    })
                    .collect();

                // Gather the contiguous run of following Tool results so we
                // can attach a name (Google needs it) when available.
                //
                // Match tool_results to tool_calls POSITIONALLY: since
                // dispatch stores results in the same order as the calls,
                // the Nth result corresponds to the Nth (deduplicated)
                // tool_call ID. This avoids the HashMap overwrite bug where
                // multiple tool_calls sharing the same original ID would
                // cause all their results to reference only the last remap.
                let mut j = i + 1;
                let mut results: Vec<ToolCallResult> = Vec::new();
                let mut result_index: usize = 0;
                while j < history.len() && matches!(history[j].role, MessageRole::Tool) {
                    let tm = &history[j];
                    // Use the deduplicated tool_call ID at the corresponding
                    // position, falling back to the stored tool_call_id.
                    let id = if result_index < tool_calls.len() {
                        tool_calls[result_index].id.clone()
                    } else {
                        tm.tool_call_id.clone().unwrap_or_default()
                    };
                    // Recover the tool name from the matching call.
                    let name = tool_calls
                        .iter()
                        .find(|tc| tc.id == id)
                        .map(|tc| tc.name.clone())
                        .unwrap_or_default();
                    results.push(ToolCallResult {
                        id,
                        name,
                        output: tm.content.clone(),
                        is_error: false,
                    });
                    result_index += 1;
                    j += 1;
                }

                // Synthesize a ModelResponse so we can reuse the existing
                // provider-aware formatters.
                let model_resp = ModelResponse {
                    text: m.content.clone(),
                    tool_calls,
                    ..Default::default()
                };
                let assistant_content = format_assistant_message(provider, &model_resp);
                out.push(ChatMessage::text("assistant", &assistant_content));

                for (role, content) in format_tool_results(provider, &results) {
                    out.push(ChatMessage::text(&role, &content));
                }

                i = j;
            }
            MessageRole::Tool => {
                // Orphan tool message (no preceding assistant tool_calls in
                // this run — likely a corrupted history). Drop it rather
                // than forwarding an unanchored tool message, which the
                // provider would reject outright.
                tracing::warn!(
                    target: "rustyclaw::gateway",
                    tool_call_id = ?m.tool_call_id,
                    "Dropping orphan tool message during history reconstruction"
                );
                i += 1;
            }
        }
    }
    out
}

// ── Context compaction ──────────────────────────────────────────────────────

use crate::helpers::estimate_tokens;

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
    writer: &mut dyn TransportWriter,
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

    // Ensure the kept tail doesn't start with orphaned tool results.
    // A "tool" message references a tool_use_id from the preceding "assistant"
    // message; if we split them, the model rejects the request.  Walk backward
    // to include the matching assistant turn.
    while keep_from > start_idx && msgs[keep_from].role == "tool" {
        keep_from -= 1;
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
            let mut boundary = 2000;
            while !m.content.is_char_boundary(boundary) {
                boundary -= 1;
            }
            format!("{}… [truncated]", &m.content[..boundary])
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

    let summary_result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        if resolved.provider == "anthropic" {
            call_anthropic_with_tools(http, &summary_req, None).await
        } else if resolved.provider == "google" {
            call_google_with_tools(http, &summary_req).await
        } else {
            call_openai_with_tools(http, &summary_req, None).await
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("compaction summary request timed out after 60s"))
    .and_then(|r| r);

    let summary = match summary_result {
        Ok(resp) if !resp.text.is_empty() => resp.text,
        Ok(_) => anyhow::bail!("Model returned empty summary"),
        Err(e) => anyhow::bail!("Summary request failed: {:#}", e),
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
    server::send_info(
        writer,
        &format!(
            "Context compacted: {} → {} messages (~{}k → ~{}k tokens)",
            old_count,
            new_count,
            old_tokens / 1000,
            new_tokens / 1000,
        ),
    )
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
///
/// On any failure path a structured `tracing::warn!` is emitted via
/// [`rustyclaw_core::error_details::RequestDetails`] so that JSON log output
/// carries the request method, URL, status, redacted request/response
/// headers, and body excerpt as named fields — rather than a single
/// pre-formatted string.  The wire-protocol [`ProbeResult`] strings
/// remain unchanged for compatibility with TUI/CLI clients.
pub async fn validate_model_connection(
    http: &reqwest::Client,
    ctx: &ModelContext,
    copilot_session: Option<&CopilotSession>,
) -> ProbeResult {
    // Resolve the bearer token (session token for Copilot, raw key otherwise).
    let effective_key = match crate::auth::resolve_bearer_token(
        http,
        &ctx.provider,
        ctx.api_key.as_deref(),
        copilot_session,
    )
    .await
    {
        Ok(k) => k,
        Err(err) => {
            // The inner error from `resolve_bearer_token` (in particular
            // the Copilot session-exchange path via
            // `providers::exchange_copilot_session`) already carries a
            // populated `RequestDetails` with method/url/status/headers/
            // body when applicable.  Emit a warn with that structured
            // context preserved, plus the provider id, rather than
            // overwriting the URL field with a synthetic placeholder.
            let wrapped = anyhow_tracing::Error::from(err)
                .context("Token exchange failed")
                .with_field("provider", &ctx.provider);
            tracing::warn!(
                target: "rustyclaw::providers",
                provider = %ctx.provider,
                error = %wrapped,
                "Token exchange failed during model probe",
            );
            return ProbeResult::AuthError {
                detail: format_probe_error(&wrapped),
            };
        }
    };

    // Per-branch probe: build the request, capture the structured
    // request snapshot (method/url/headers/bearer) before issuing it,
    // and run it.  Each branch returns the response (or send error)
    // along with the snapshot it built.
    let (mut details, result) = if ctx.provider == "anthropic" {
        let base = ctx.base_url.trim_end_matches('/');
        let url = if base.ends_with("/v1") {
            format!("{}/messages", base)
        } else {
            format!("{}/v1/messages", base)
        };
        let body = json!({
            "model": ctx.model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "Hi"}],
        });
        let api_key = ctx.api_key.as_deref().unwrap_or("");
        let details = RequestDetails::new("probe.anthropic", "POST", url.clone())
            .with_provider(&ctx.provider)
            .with_request_headers([
                ("x-api-key", "<redacted>"),
                ("anthropic-version", "2023-06-01"),
                ("content-type", "application/json"),
            ])
            .with_bearer(Some(api_key));
        let builder = http
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body);
        (details, send_with_retry(builder).await)
    } else if ctx.provider == "google" {
        // Google: check the model metadata endpoint (no chat needed).
        let key = ctx.api_key.as_deref().unwrap_or("");
        let url = format!(
            "{}/models/{}?key={}",
            ctx.base_url.trim_end_matches('/'),
            ctx.model,
            key,
        );
        let public_url = format!(
            "{}/models/{}?key=<redacted>",
            ctx.base_url.trim_end_matches('/'),
            ctx.model,
        );
        let details = RequestDetails::new("probe.google", "GET", public_url)
            .with_provider(&ctx.provider)
            .with_bearer(Some(key));
        (details, send_with_retry(http.get(&url)).await)
    } else {
        // OpenAI-compatible: GET /models — lightweight auth check.
        let url = format!("{}/models", ctx.base_url.trim_end_matches('/'));
        let mut details = RequestDetails::new("probe.openai_compatible", "GET", url.clone())
            .with_provider(&ctx.provider)
            .with_bearer(effective_key.as_deref());
        if effective_key.is_some() {
            details = details.with_request_headers([("Authorization", "Bearer <redacted>")]);
        }
        let mut builder = http.get(&url);
        if let Some(ref key) = effective_key {
            builder = builder.bearer_auth(key);
        }
        builder = apply_copilot_headers(builder, &ctx.provider, &[]);
        (details, send_with_retry(builder).await)
    };

    match result {
        Ok(resp) if resp.status().is_success() => ProbeResult::Ready,
        Ok(resp) => {
            let status = resp.status();
            let code = status.as_u16();
            let response_headers = resp.headers().clone();
            let body = resp.text().await.unwrap_or_default();

            details = details
                .with_response(code, &response_headers)
                .with_body(&body);

            // Build a structured anyhow_tracing error capturing the HTTP
            // response so the trace below carries method/url/status/
            // headers/body as named fields rather than a single
            // pre-rendered string.
            let truncated = providers::truncate_for_error(&body);
            let err = anyhow_tracing::anyhow!(
                "{} {} returned HTTP {} — body: {}",
                details.method,
                details.url,
                status,
                truncated
            );

            // Try to extract a human-readable error message from JSON for
            // the wire-protocol summary.  The full body is preserved on
            // the structured field above.
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message").or(Some(e)))
                        .and_then(|m| m.as_str().map(String::from))
                })
                .unwrap_or(body);

            match code {
                401 | 403 => {
                    let _ = details.emit_warning(err);
                    ProbeResult::AuthError {
                        detail: format!("{} — {}", status, detail),
                    }
                }
                // 400, 404, 422 etc — the server answered, auth is fine,
                // but something about the request/model wasn't accepted.
                // Chat may still work with the full request format.
                400..=499 => {
                    let _ = details.emit_warning(err);
                    ProbeResult::Connected {
                        warning: format!("{} — {}", status, detail),
                    }
                }
                _ => {
                    let _ = details.emit_warning(err);
                    ProbeResult::Unreachable {
                        detail: format!("{} — {}", status, detail),
                    }
                }
            }
        }
        Err(err) => {
            let wrapped = anyhow_tracing::Error::from(err)
                .context(format!("{} {} failed", details.method, details.url));
            let wrapped = details.emit_warning(wrapped);
            ProbeResult::Unreachable {
                detail: format_probe_error(&wrapped),
            }
        }
    }
}

/// Render an `anyhow_tracing::Error` as a single-line wire-protocol
/// detail string.  The full structured fields are emitted via
/// `tracing::warn!` separately by [`RequestDetails::emit_warning`]; this
/// helper produces a concise human-readable summary suitable for a
/// status frame's `detail` field.
fn format_probe_error(err: &anyhow_tracing::Error) -> String {
    // Filter out empty causes so a stray `.context("")` or empty top-level
    // message doesn't produce something like ": some cause".
    let chain: Vec<String> = err
        .cause_chain()
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect();
    if chain.is_empty() {
        // Last-resort fallback: use the full Display, which includes the
        // anyhow-tracing field list.  Better than returning an empty
        // string to the wire protocol.
        let s = err.to_string();
        if s.trim().is_empty() {
            "unknown error".to_string()
        } else {
            s
        }
    } else {
        chain.join(": ")
    }
}

// The genai-backed provider dispatch lives in `rustyclaw-core` so the gateway
// and client crates share one genai instance. Re-export the call surface here
// so existing `providers::call_*` call sites resolve unchanged.
pub use rustyclaw_core::providers::{
    call_anthropic_with_tools, call_google_with_tools, call_openai_with_tools,
};

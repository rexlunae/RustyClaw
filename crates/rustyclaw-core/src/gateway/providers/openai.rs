//! OpenAI provider integration — Chat Completions and Responses APIs,
//! plus the shared SSE streaming consumers.

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{debug, trace, warn};

use super::super::protocol::server;
use super::super::transport::TransportWriter;
use super::super::types::{ChatMessage, ModelResponse, ParsedToolCall, ProviderRequest};
use super::{
    apply_copilot_headers, find_event_boundary, provider_error, send_chunk, send_with_retry,
};
use crate::tools;

// ── OpenAI Responses API ─────────────────────────────────────────────────────

/// Convert our internal `ChatMessage` slice into the `input` array expected by
/// the OpenAI Responses API (`POST /v1/responses`).
///
/// Handles all four cases that appear in multi-turn tool loops:
/// * `system`/`user` — passed through unchanged.
/// * `assistant` with plain text — passed through unchanged.
/// * `assistant` stored as JSON with `tool_calls` (Chat Completions format)
///   — each tool call becomes a `{"type":"function_call",...}` item.
/// * `tool` stored as JSON with `tool_call_id`/`content`
///   — converted to `{"type":"function_call_output",...}`.
fn messages_to_responses_input(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut input: Vec<serde_json::Value> = Vec::with_capacity(messages.len());

    for msg in messages {
        match msg.role.as_str() {
            "system" | "user" => {
                input.push(json!({"role": msg.role, "content": msg.content}));
            }
            "assistant" => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                    if let Some(tool_calls) = parsed.get("tool_calls").and_then(|v| v.as_array()) {
                        // Emit any preceding text content.
                        let text = parsed
                            .get("content")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty());
                        if let Some(text) = text {
                            input.push(json!({"role": "assistant", "content": text}));
                        }
                        // One function_call item per tool call.
                        for tc in tool_calls {
                            let call_id = tc["id"].as_str().unwrap_or("");
                            let name = tc["function"]["name"].as_str().unwrap_or("");
                            let arguments = tc["function"]["arguments"].as_str().unwrap_or("{}");
                            input.push(json!({
                                "type": "function_call",
                                "call_id": call_id,
                                "name": name,
                                "arguments": arguments,
                            }));
                        }
                        continue;
                    }
                    // JSON-wrapped plain assistant message (has "role" key).
                    if parsed.get("role").is_some() {
                        let text = parsed["content"].as_str().unwrap_or("");
                        input.push(json!({"role": "assistant", "content": text}));
                        continue;
                    }
                }
                input.push(json!({"role": "assistant", "content": msg.content}));
            }
            "tool" => {
                // {"role":"tool","tool_call_id":"call_xxx","content":"result"}
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                    let call_id = parsed["tool_call_id"].as_str().unwrap_or("");
                    let output = parsed["content"].as_str().unwrap_or("");
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": output,
                    }));
                } else {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": "",
                        "output": msg.content,
                    }));
                }
            }
            _ => {
                input.push(json!({"role": msg.role, "content": msg.content}));
            }
        }
    }

    input
}

/// Consume a streaming Responses API SSE response and return a `ModelResponse`.
async fn consume_responses_api_sse(
    resp: reqwest::Response,
    mut writer: Option<&mut dyn TransportWriter>,
) -> Result<ModelResponse> {
    use futures_util::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut result = ModelResponse::default();
    // item_id → (call_id, name, accumulated_arguments)
    let mut func_calls: std::collections::HashMap<String, (String, String, String)> =
        std::collections::HashMap::new();
    let mut stream_started = false;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("Responses API stream read error")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some((event_end, skip)) = find_event_boundary(&buffer) {
            let event = buffer[..event_end].to_string();
            buffer = buffer[event_end + skip..].to_string();

            let mut event_type = String::new();
            let mut event_data = String::new();

            for line in event.lines() {
                if let Some(t) = line.strip_prefix("event: ") {
                    event_type = t.to_string();
                } else if let Some(d) = line.strip_prefix("data: ") {
                    event_data = d.to_string();
                }
            }

            if event_data.is_empty() || event_data == "[DONE]" {
                continue;
            }

            let Ok(json) = serde_json::from_str::<serde_json::Value>(&event_data) else {
                continue;
            };

            trace!(event_type = %event_type, "Responses API SSE event");

            match event_type.as_str() {
                "response.output_text.delta" => {
                    let delta = json["delta"].as_str().unwrap_or("");
                    if !delta.is_empty() {
                        if !stream_started {
                            if let Some(w) = writer.as_deref_mut() {
                                server::send_stream_start(w).await?;
                            }
                            stream_started = true;
                        }
                        result.text.push_str(delta);
                        if let Some(w) = writer.as_deref_mut() {
                            send_chunk(w, delta).await?;
                        }
                    }
                }
                "response.output_item.added" => {
                    let item = &json["item"];
                    if item["type"].as_str() == Some("function_call") {
                        let item_id = item["id"].as_str().unwrap_or("").to_string();
                        let call_id = item["call_id"].as_str().unwrap_or("").to_string();
                        let name = item["name"].as_str().unwrap_or("").to_string();
                        func_calls.insert(item_id, (call_id, name, String::new()));
                    }
                }
                "response.function_call_arguments.delta" => {
                    let item_id = json["item_id"].as_str().unwrap_or("");
                    let delta = json["delta"].as_str().unwrap_or("");
                    if let Some(entry) = func_calls.get_mut(item_id) {
                        entry.2.push_str(delta);
                    }
                }
                "response.output_item.done" => {
                    let item = &json["item"];
                    if item["type"].as_str() == Some("function_call") {
                        let item_id = item["id"].as_str().unwrap_or("");
                        if let Some((call_id, name, args)) = func_calls.remove(item_id) {
                            let final_args = item["arguments"].as_str().unwrap_or(args.as_str());
                            let arguments = serde_json::from_str(final_args).unwrap_or(json!({}));
                            if !call_id.is_empty() && !name.is_empty() {
                                result.tool_calls.push(ParsedToolCall {
                                    id: call_id,
                                    name,
                                    arguments,
                                });
                            }
                        }
                    }
                }
                "response.completed" => {
                    let usage = &json["response"]["usage"];
                    result.prompt_tokens = usage["input_tokens"].as_u64();
                    result.completion_tokens = usage["output_tokens"].as_u64();
                    result.finish_reason = Some(
                        json["response"]["status"]
                            .as_str()
                            .unwrap_or("stop")
                            .to_string(),
                    );
                }
                "response.failed" | "error" => {
                    let msg = json["error"]["message"]
                        .as_str()
                        .or_else(|| json["message"].as_str())
                        .unwrap_or("Unknown error from Responses API");
                    anyhow::bail!("Responses API error: {}", msg);
                }
                _ => {}
            }
        }
    }

    Ok(result)
}

/// Call the OpenAI Responses API (`POST .../responses`) with tool support.
///
/// This is used automatically as a fallback when a model refuses `/chat/completions`
/// with `code: "unsupported_api_for_model"` (e.g. `gpt-5.5`).
pub async fn call_openai_responses_api(
    http: &reqwest::Client,
    req: &ProviderRequest,
    writer: Option<&mut dyn TransportWriter>,
) -> Result<ModelResponse> {
    let url = format!("{}/responses", req.base_url.trim_end_matches('/'));

    let input = messages_to_responses_input(&req.messages);

    let tool_defs = if std::env::var("RUSTYCLAW_SKIP_TOOLS").is_ok() {
        vec![]
    } else {
        tools::tools_openai()
    };

    let mut body = json!({
        "model": req.model,
        "input": input,
        "stream": true,
    });
    if !tool_defs.is_empty() {
        body["tools"] = json!(tool_defs);
    }

    let mut builder = http.post(&url).json(&body);
    if let Some(ref key) = req.api_key {
        builder = builder.bearer_auth(key);
    }
    builder = apply_copilot_headers(builder, &req.provider, &req.messages);

    let resp = send_with_retry(builder).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(provider_error("Provider (Responses API)", status, &text));
    }

    consume_responses_api_sse(resp, writer).await
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

                            if let Some(tc_array) =
                                delta.get("tool_calls").and_then(|v| v.as_array())
                            {
                                for tc in tc_array {
                                    let index =
                                        tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0)
                                            as usize;
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
                                        if let Some(name) =
                                            func.get("name").and_then(|v| v.as_str())
                                        {
                                            tool_calls[index]["function"]["name"] = json!(name);
                                        }
                                        if let Some(args) =
                                            func.get("arguments").and_then(|v| v.as_str())
                                        {
                                            let existing =
                                                tool_calls[index]["function"]["arguments"]
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
    let incomplete_count = tool_calls
        .iter()
        .filter(|tc| {
            let id = tc["id"].as_str().unwrap_or("");
            let name = tc["function"]["name"].as_str().unwrap_or("");
            id.is_empty() || name.is_empty()
        })
        .count();

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
        debug!(
            incomplete = incomplete_count,
            remaining = tool_calls.len(),
            "Filtered incomplete tool calls"
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
async fn consume_sse_stream(
    resp: reqwest::Response,
    mut writer: Option<&mut dyn TransportWriter>,
) -> Result<serde_json::Value> {
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

    if let Some(w) = writer.as_deref_mut() {
        server::send_stream_start(w).await?;
    }

    // Timeout for waiting on next chunk — if exceeded, assume stream is done
    let chunk_timeout = Duration::from_secs(30);

    'outer: loop {
        // Wait for next chunk with timeout
        let chunk_result = match timeout(chunk_timeout, stream.next()).await {
            Ok(Some(result)) => result,
            Ok(None) => {
                trace!("SSE stream ended normally");
                break 'outer;
            }
            Err(_) => {
                // Timeout — stream stalled, return what we have
                warn!(timeout_secs = chunk_timeout.as_secs(), "SSE stream timeout");
                break 'outer;
            }
        };

        let chunk = chunk_result.context("SSE stream read error")?;
        let chunk_str = String::from_utf8_lossy(&chunk);

        buffer.push_str(&chunk_str);

        // Process complete SSE events (terminated by a blank line).
        // Handle both Unix (\n\n) and Windows (\r\n\r\n) line endings.
        while let Some((event_end, skip)) = find_event_boundary(&buffer) {
            let event = buffer[..event_end].to_string();
            buffer = buffer[event_end + skip..].to_string();

            for line in event.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data.trim() == "[DONE]" {
                        // Stream complete — exit all loops
                        trace!(
                            content_len = content.len(),
                            tool_calls = tool_calls.len(),
                            "SSE received [DONE]"
                        );
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
                                        if let Some(w) = writer.as_deref_mut() {
                                            send_chunk(w, c).await?;
                                        }
                                    }

                                    // Tool calls (streamed incrementally)
                                    if let Some(tc_array) =
                                        delta.get("tool_calls").and_then(|v| v.as_array())
                                    {
                                        for tc in tc_array {
                                            let index = tc
                                                .get("index")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0)
                                                as usize;

                                            // Ensure tool_calls vec is big enough
                                            while tool_calls.len() <= index {
                                                tool_calls.push(json!({
                                                    "id": "",
                                                    "type": "function",
                                                    "function": { "name": "", "arguments": "" }
                                                }));
                                            }

                                            // Update tool call fields
                                            if let Some(id) = tc.get("id").and_then(|v| v.as_str())
                                            {
                                                tool_calls[index]["id"] = json!(id);
                                            }
                                            if let Some(func) = tc.get("function") {
                                                if let Some(name) =
                                                    func.get("name").and_then(|v| v.as_str())
                                                {
                                                    tool_calls[index]["function"]["name"] =
                                                        json!(name);
                                                }
                                                if let Some(args) =
                                                    func.get("arguments").and_then(|v| v.as_str())
                                                {
                                                    // Append to existing arguments
                                                    let existing =
                                                        tool_calls[index]["function"]["arguments"]
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
                                if let Some(fr) =
                                    choice.get("finish_reason").and_then(|v| v.as_str())
                                {
                                    finish_reason = Some(fr.to_string());
                                    // Terminal reasons mean the model is done
                                    if fr == "stop"
                                        || fr == "tool_calls"
                                        || fr == "tool_use"
                                        || fr == "length"
                                        || fr == "end_turn"
                                    {
                                        trace!(
                                            finish_reason = fr,
                                            content_len = content.len(),
                                            tool_calls = tool_calls.len(),
                                            "SSE stream completed"
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
    let incomplete_count = tool_calls
        .iter()
        .filter(|tc| {
            let id = tc["id"].as_str().unwrap_or("");
            let name = tc["function"]["name"].as_str().unwrap_or("");
            id.is_empty() || name.is_empty()
        })
        .count();

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
        debug!(
            incomplete = incomplete_count,
            remaining = tool_calls.len(),
            "Filtered incomplete tool calls from stream"
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
    mut writer: Option<&mut dyn TransportWriter>,
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

    // Skip tool definitions when SKIP_TOOLS env var is set (reduces prompt size)
    let tool_defs = if std::env::var("RUSTYCLAW_SKIP_TOOLS").is_ok() {
        vec![]
    } else {
        tools::tools_openai()
    };

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": 16384,
        "stream": true,
    });
    // stream_options is an OpenAI extension — only include for providers
    // known to support it.  Copilot and other proxies may reject or
    // mishandle unrecognised fields.
    if !crate::providers::needs_copilot_session(&req.provider) {
        body["stream_options"] = json!({ "include_usage": true });
    }
    if !tool_defs.is_empty() {
        body["tools"] = json!(tool_defs);
    }

    let mut builder = http.post(&url).json(&body);
    if let Some(ref key) = req.api_key {
        builder = builder.bearer_auth(key);
    }
    builder = apply_copilot_headers(builder, &req.provider, &req.messages);

    let resp = send_with_retry(builder).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        // Transparently retry via the Responses API for models that only
        // support that endpoint (e.g. gpt-5.5).
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if json["error"]["code"].as_str() == Some("unsupported_api_for_model") {
                debug!(
                    model = %req.model,
                    "Model requires Responses API; retrying via /responses"
                );
                return call_openai_responses_api(http, req, writer).await;
            }
        }

        return Err(provider_error("Provider", status, &text));
    }

    // Check if the server returned a streaming response (SSE) despite us
    // not requesting one.  Some providers (e.g. GitHub Copilot) may force
    // streaming.  If so, consume the SSE stream and reassemble the response.
    let is_sse_response = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|content_type| content_type.contains("text/event-stream"));

    // Detect SSE by content-type (may include charset, e.g., "text/event-stream; charset=utf-8")
    let data: serde_json::Value = if is_sse_response {
        // Server is streaming — parse SSE events.
        consume_sse_stream(resp, writer.take()).await?
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

    if let Some(w) = writer
        && !result.text.is_empty()
        && !is_sse_response
    {
        server::send_stream_start(w).await?;
        send_chunk(w, &result.text).await?;
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

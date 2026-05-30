//! Anthropic Messages API provider integration.

use anyhow::{Context, Result};
use serde_json::json;
use tracing::trace;

use super::super::protocol::server;
use super::super::transport::TransportWriter;
use super::super::types::{ModelResponse, ParsedToolCall, ProviderRequest};
use super::{
    provider_error, send_chunk, send_thinking_delta, send_thinking_end, send_thinking_start,
    send_with_retry,
};
use crate::tools;

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
    mut writer: Option<&mut dyn TransportWriter>,
) -> Result<ModelResponse> {
    use futures_util::StreamExt;

    // Build the Anthropic messages URL.  The static base_url is
    // "https://api.anthropic.com" (no /v1), but the user's config may
    // have saved it with a trailing /v1.  Handle both to avoid a
    // double "/v1/v1/messages" path that 404s.
    let base = req.base_url.trim_end_matches('/');
    let url = if base.ends_with("/v1") {
        format!("{}/messages", base)
    } else {
        format!("{}/v1/messages", base)
    };

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

    // Skip tool definitions when SKIP_TOOLS env var is set (reduces prompt size)
    let tool_defs = if std::env::var("RUSTYCLAW_SKIP_TOOLS").is_ok() {
        vec![]
    } else {
        tools::tools_anthropic()
    };

    // Use streaming when we have a writer to forward chunks to
    let use_streaming = writer.is_some();

    // Allow generous output length to avoid truncation on long responses
    let max_tokens = 16384;

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
        server::send_stream_start(*w).await?;
    }

    let api_key = req.api_key.as_deref().unwrap_or("");
    let builder = http
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body);
    let resp = send_with_retry(builder).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(provider_error("Anthropic", status, &text));
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
    let mut tool_args_buffer: std::collections::HashMap<usize, String> =
        std::collections::HashMap::new();

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
            trace!(event_type = %event_type, data_len = event_data.len(), "Anthropic SSE event");

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
                                    current_tool_index =
                                        json["index"].as_u64().unwrap_or(0) as usize;

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
                                        if let Some(buf) =
                                            tool_args_buffer.get_mut(&current_tool_index)
                                        {
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
                                    tc.arguments =
                                        serde_json::from_str(&args_str).unwrap_or(json!({}));
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
                        let msg = json["error"]["message"].as_str().unwrap_or("Unknown error");
                        anyhow::bail!("Anthropic stream error: {}", msg);
                    }
                    _ => {
                        trace!(event_type = %event_type, "Unhandled Anthropic SSE event type");
                    }
                }
            }
        }
    }

    // Debug: log final result
    trace!(
        text_len = result.text.len(),
        tool_calls = result.tool_calls.len(),
        finish_reason = ?result.finish_reason,
        "Anthropic SSE stream complete"
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

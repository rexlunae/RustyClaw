//! Streaming provider support for OpenAI-compatible and Anthropic APIs.
//!
//! This module adds SSE (Server-Sent Events) streaming to provider calls,
//! allowing real-time token delivery to the TUI.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

/// A streaming chunk from the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamChunk {
    /// Text content delta
    Text(String),
    /// Extended thinking started (Anthropic)
    ThinkingStart,
    /// Extended thinking content delta (Anthropic)
    ThinkingDelta(String),
    /// Extended thinking finished, includes summary if provided
    ThinkingEnd { summary: Option<String> },
    /// Tool call started
    ToolCallStart {
        index: usize,
        id: String,
        name: String,
    },
    /// Tool call arguments delta
    ToolCallDelta { index: usize, arguments: String },
    /// Stream finished
    Done,
    /// Error occurred
    Error(String),
}

/// Request parameters for streaming calls
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub messages: Vec<StreamMessage>,
    pub tools: Vec<serde_json::Value>,
    /// Budget tokens for extended thinking (Anthropic only)
    pub thinking_budget: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct StreamMessage {
    pub role: String,
    pub content: String,
}

/// Call OpenAI-compatible endpoint with streaming.
/// Sends chunks to the provided channel.
pub async fn call_openai_streaming(
    http: &reqwest::Client,
    req: &StreamRequest,
    tx: mpsc::Sender<StreamChunk>,
) -> Result<()> {
    let url = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));

    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_object() && parsed.get("role").is_some() {
                    return parsed;
                }
            }
            json!({ "role": m.role, "content": m.content })
        })
        .collect();

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
    });

    if !req.tools.is_empty() {
        body["tools"] = json!(req.tools);
    }

    let mut builder = http.post(&url).json(&body);
    if let Some(ref key) = req.api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder.send().await.context("HTTP request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let _ = tx.send(StreamChunk::Error(format!("{} — {}", status, text))).await;
        return Ok(());
    }

    // Parse SSE stream
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args)

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("Stream read error")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete SSE events
        while let Some(event_end) = buffer.find("\n\n") {
            let event = buffer[..event_end].to_string();
            buffer = buffer[event_end + 2..].to_string();

            for line in event.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        let _ = tx.send(StreamChunk::Done).await;
                        return Ok(());
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        // Extract content delta
                        if let Some(delta) = json["choices"][0]["delta"].as_object() {
                            // Text content
                            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                let _ = tx.send(StreamChunk::Text(content.to_string())).await;
                            }

                            // Tool calls
                            if let Some(tc_array) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                for tc in tc_array {
                                    let index = tc["index"].as_u64().unwrap_or(0) as usize;
                                    
                                    // Ensure tool_calls vec is big enough
                                    while tool_calls.len() <= index {
                                        tool_calls.push((String::new(), String::new(), String::new()));
                                    }

                                    // Tool call start
                                    if let Some(id) = tc["id"].as_str() {
                                        tool_calls[index].0 = id.to_string();
                                    }
                                    if let Some(func) = tc.get("function") {
                                        if let Some(name) = func["name"].as_str() {
                                            tool_calls[index].1 = name.to_string();
                                            let _ = tx.send(StreamChunk::ToolCallStart {
                                                index,
                                                id: tool_calls[index].0.clone(),
                                                name: name.to_string(),
                                            }).await;
                                        }
                                        if let Some(args) = func["arguments"].as_str() {
                                            tool_calls[index].2.push_str(args);
                                            let _ = tx.send(StreamChunk::ToolCallDelta {
                                                index,
                                                arguments: args.to_string(),
                                            }).await;
                                        }
                                    }
                                }
                            }
                        }

                        // Check for finish reason
                        if let Some(finish) = json["choices"][0]["finish_reason"].as_str() {
                            if finish == "stop" || finish == "tool_calls" {
                                let _ = tx.send(StreamChunk::Done).await;
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    let _ = tx.send(StreamChunk::Done).await;
    Ok(())
}

/// Call Anthropic endpoint with streaming.
///
/// Supports extended thinking via the `thinking_budget` field in the request.
/// When thinking is enabled, sends `ThinkingStart`, `ThinkingDelta`, and
/// `ThinkingEnd` chunks so the TUI can display a thinking indicator.
pub async fn call_anthropic_streaming(
    http: &reqwest::Client,
    req: &StreamRequest,
    tx: mpsc::Sender<StreamChunk>,
) -> Result<()> {
    let url = format!("{}/v1/messages", req.base_url.trim_end_matches('/'));

    let system = req
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_array() {
                    return json!({ "role": m.role, "content": parsed });
                }
            }
            json!({ "role": m.role, "content": m.content })
        })
        .collect();

    // Determine max_tokens based on whether thinking is enabled
    // Extended thinking requires higher max_tokens to accommodate thinking + response
    let max_tokens = if req.thinking_budget.is_some() {
        16384 // Allow room for thinking + response
    } else {
        4096
    };

    let mut body = json!({
        "model": req.model,
        "max_tokens": max_tokens,
        "messages": messages,
        "stream": true,
    });

    if !system.is_empty() {
        body["system"] = serde_json::Value::String(system);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(req.tools);
    }

    // Add thinking configuration if budget is specified
    if let Some(budget) = req.thinking_budget {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": budget
        });
    }

    let api_key = req.api_key.as_deref().unwrap_or("");
    let resp = http
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .context("HTTP request to Anthropic failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let _ = tx.send(StreamChunk::Error(format!("{} — {}", status, text))).await;
        return Ok(());
    }

    // Parse Anthropic SSE stream
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut current_tool_index = 0;
    let mut in_thinking_block = false;
    let mut thinking_content = String::new();

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

            if event_data.is_empty() {
                continue;
            }

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&event_data) {
                match event_type.as_str() {
                    "content_block_start" => {
                        if let Some(block) = json.get("content_block") {
                            match block["type"].as_str() {
                                Some("thinking") => {
                                    // Extended thinking block started
                                    in_thinking_block = true;
                                    thinking_content.clear();
                                    let _ = tx.send(StreamChunk::ThinkingStart).await;
                                }
                                Some("tool_use") => {
                                    let id = block["id"].as_str().unwrap_or("").to_string();
                                    let name = block["name"].as_str().unwrap_or("").to_string();
                                    current_tool_index = json["index"].as_u64().unwrap_or(0) as usize;
                                    let _ = tx.send(StreamChunk::ToolCallStart {
                                        index: current_tool_index,
                                        id,
                                        name,
                                    }).await;
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
                                        let _ = tx.send(StreamChunk::ThinkingDelta(thinking.to_string())).await;
                                    }
                                }
                                Some("text_delta") => {
                                    if let Some(text) = delta["text"].as_str() {
                                        let _ = tx.send(StreamChunk::Text(text.to_string())).await;
                                    }
                                }
                                Some("input_json_delta") => {
                                    if let Some(partial) = delta["partial_json"].as_str() {
                                        let _ = tx.send(StreamChunk::ToolCallDelta {
                                            index: current_tool_index,
                                            arguments: partial.to_string(),
                                        }).await;
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
                            // (first ~100 chars or first sentence, whichever is shorter)
                            let summary = if thinking_content.len() > 100 {
                                let truncated = &thinking_content[..100];
                                if let Some(period_pos) = truncated.find(". ") {
                                    Some(truncated[..=period_pos].to_string())
                                } else {
                                    Some(format!("{}...", truncated))
                                }
                            } else if !thinking_content.is_empty() {
                                Some(thinking_content.clone())
                            } else {
                                None
                            };
                            let _ = tx.send(StreamChunk::ThinkingEnd { summary }).await;
                        }
                    }
                    "message_stop" => {
                        let _ = tx.send(StreamChunk::Done).await;
                        return Ok(());
                    }
                    "error" => {
                        let msg = json["error"]["message"]
                            .as_str()
                            .unwrap_or("Unknown error");
                        let _ = tx.send(StreamChunk::Error(msg.to_string())).await;
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = tx.send(StreamChunk::Done).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_chunk_serialization() {
        let chunk = StreamChunk::Text("hello".to_string());
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("Text"));
        assert!(json.contains("hello"));
    }

    #[test]
    fn test_thinking_chunk_serialization() {
        let start = StreamChunk::ThinkingStart;
        let json = serde_json::to_string(&start).unwrap();
        assert!(json.contains("ThinkingStart"));

        let delta = StreamChunk::ThinkingDelta("analyzing...".to_string());
        let json = serde_json::to_string(&delta).unwrap();
        assert!(json.contains("ThinkingDelta"));
        assert!(json.contains("analyzing"));

        let end = StreamChunk::ThinkingEnd { summary: Some("Done thinking".to_string()) };
        let json = serde_json::to_string(&end).unwrap();
        assert!(json.contains("ThinkingEnd"));
        assert!(json.contains("Done thinking"));
    }

    #[test]
    fn test_stream_request_creation() {
        let req = StreamRequest {
            provider: "openai".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_key: Some("test-key".to_string()),
            model: "gpt-4".to_string(),
            messages: vec![StreamMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            tools: vec![],
            thinking_budget: None,
        };
        assert_eq!(req.model, "gpt-4");
    }

    #[test]
    fn test_stream_request_with_thinking() {
        let req = StreamRequest {
            provider: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: Some("test-key".to_string()),
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![StreamMessage {
                role: "user".to_string(),
                content: "Think about this deeply".to_string(),
            }],
            tools: vec![],
            thinking_budget: Some(10000),
        };
        assert_eq!(req.thinking_budget, Some(10000));
    }
}

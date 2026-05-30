//! Google Gemini provider integration.

use anyhow::{Context, Result};
use serde_json::json;

use super::super::types::{ModelResponse, ParsedToolCall, ProviderRequest};
use super::{provider_error, send_with_retry};
use crate::tools;

/// Call Google Gemini with function declarations (non-streaming).
pub async fn call_google_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
) -> Result<ModelResponse> {
    let api_key = req.api_key.as_deref().unwrap_or("");
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
            let role = if m.role == "assistant" {
                "model"
            } else {
                "user"
            };
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&m.content) {
                if parsed.is_array() {
                    return json!({ "role": role, "parts": parsed });
                }
            }
            json!({ "role": role, "parts": [{ "text": m.content }] })
        })
        .collect();

    // Skip tool definitions when SKIP_TOOLS env var is set (reduces prompt size)
    let tool_defs = if std::env::var("RUSTYCLAW_SKIP_TOOLS").is_ok() {
        vec![]
    } else {
        tools::tools_google()
    };

    let mut body = json!({ "contents": contents });
    if !system.is_empty() {
        body["system_instruction"] = json!({ "parts": [{ "text": system }] });
    }
    if !tool_defs.is_empty() {
        body["tools"] = json!([{ "function_declarations": tool_defs }]);
    }

    let builder = http.post(&url).json(&body);
    let resp = send_with_retry(builder).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(provider_error("Google", status, &text));
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

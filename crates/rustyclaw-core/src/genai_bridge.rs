//! Type conversions between genai and RustyClaw gateway types.
//!
//! This module bridges the genai crate's types with RustyClaw's internal
//! protocol types, allowing gradual migration from hand-rolled provider
//! code to the genai abstraction.

use crate::gateway::protocol::types::{ChatMessage as RcChatMessage, ModelResponse, ParsedToolCall};
use genai::chat::{
    ChatMessage as GenaiChatMessage, ChatResponse, ChatStreamEvent, MessageContent, StreamEnd, Tool,
    ToolCall, ToolResponse,
};

// ── Message Conversions ─────────────────────────────────────────────────────

/// Convert a JSON tool-calls array (OpenAI format) to a `Vec<genai::chat::ToolCall>`.
///
/// Handles the OpenAI / OpenAI-compatible format:
/// ```json
/// [{ "id": "...", "type": "function", "function": { "name": "...", "arguments": "{...}" } }]
/// ```
/// `arguments` may be a JSON-encoded string **or** a JSON object — both are normalised to
/// a `serde_json::Value`.
fn json_to_genai_tool_calls(tool_calls_json: &serde_json::Value) -> Vec<ToolCall> {
    let array = match tool_calls_json.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    array
        .iter()
        .filter_map(|tc| {
            // OpenAI format: { "id": "...", "function": { "name": "...", "arguments": "..." } }
            let call_id = tc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let (fn_name, fn_arguments) = if let Some(func) = tc.get("function") {
                let name = func
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // `arguments` may be a pre-serialised JSON string or already an object.
                let raw = func
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let parsed = if let Some(s) = raw.as_str() {
                    serde_json::from_str(s).unwrap_or(serde_json::Value::Null)
                } else {
                    raw
                };
                (name, parsed)
            } else {
                // Flat format: { "name": "...", "arguments": { … } }
                let name = tc
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = tc
                    .get("arguments")
                    .or_else(|| tc.get("parameters"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                (name, args)
            };
            Some(ToolCall {
                call_id,
                fn_name,
                fn_arguments,
                thought_signatures: None,
            })
        })
        .collect()
}

/// Convert RustyClaw ChatMessage to genai ChatMessage.
///
/// Handles four roles:
/// - `system` → `GenaiChatMessage::system`
/// - `user`   → `GenaiChatMessage::user`
/// - `assistant` → `GenaiChatMessage::assistant` **or** `assistant_tool_calls_with_thoughts`
///   when `tool_calls` is present.
/// - `tool` (tool result) → `GenaiChatMessage::user(ToolResponse { call_id, content })`
pub fn rc_to_genai_message(msg: &RcChatMessage) -> GenaiChatMessage {
    match msg.role.as_str() {
        "system" => GenaiChatMessage::system(&msg.content),
        "user" => GenaiChatMessage::user(&msg.content),
        "assistant" => {
            // If the message carries tool calls, represent them properly.
            if let Some(tc_json) = &msg.tool_calls {
                let tool_calls = json_to_genai_tool_calls(tc_json);
                if !tool_calls.is_empty() {
                    return GenaiChatMessage::assistant_tool_calls_with_thoughts(
                        tool_calls,
                        vec![],
                    );
                }
            }
            GenaiChatMessage::assistant(&msg.content)
        }
        // Tool result: genai uses a ToolResponse carried inside a user message.
        "tool" => {
            let call_id = msg.tool_call_id.clone().unwrap_or_default();
            let tr = ToolResponse {
                call_id,
                content: msg.content.clone(),
            };
            GenaiChatMessage::user(tr)
        }
        _ => GenaiChatMessage::user(&msg.content),
    }
}

/// Convert a slice of RustyClaw messages to genai messages.
pub fn rc_to_genai_messages(messages: &[RcChatMessage]) -> Vec<GenaiChatMessage> {
    messages.iter().map(rc_to_genai_message).collect()
}

// ── Tool Conversions ────────────────────────────────────────────────────────

/// Convert a single JSON tool definition to a genai [`Tool`].
///
/// Handles two common formats:
///
/// * **OpenAI / OpenAI-compatible**
///   ```json
///   { "type": "function", "function": { "name": "...", "description": "...", "parameters": { … } } }
///   ```
/// * **Flat** (Anthropic `input_schema` or Google `parameters`)
///   ```json
///   { "name": "...", "description": "...", "parameters": { … } }
///   { "name": "...", "description": "...", "input_schema": { … } }
///   ```
pub fn json_value_to_genai_tool(value: &serde_json::Value) -> Tool {
    // OpenAI / OpenAI-compatible: nested under "function" key.
    if let Some(func) = value.get("function") {
        let name = func
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let mut tool = Tool::new(name);
        if let Some(desc) = func.get("description").and_then(|v| v.as_str()) {
            tool = tool.with_description(desc);
        }
        if let Some(params) = func.get("parameters") {
            tool = tool.with_schema(params.clone());
        }
        return tool;
    }

    // Flat format: name/description at top level, schema under "parameters" or "input_schema".
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let mut tool = Tool::new(name);
    if let Some(desc) = value.get("description").and_then(|v| v.as_str()) {
        tool = tool.with_description(desc);
    }
    let schema = value
        .get("parameters")
        .or_else(|| value.get("input_schema"));
    if let Some(schema) = schema {
        tool = tool.with_schema(schema.clone());
    }
    tool
}

/// Convert a slice of JSON tool definitions to a `Vec<genai::chat::Tool>`.
pub fn json_tools_to_genai(tools: &[serde_json::Value]) -> Vec<Tool> {
    tools.iter().map(json_value_to_genai_tool).collect()
}

// ── Response Conversions ────────────────────────────────────────────────────

/// Convert genai ChatResponse to RustyClaw ModelResponse.
pub fn genai_to_rc_response(resp: &ChatResponse) -> ModelResponse {
    let mut result = ModelResponse::default();

    // Extract text content using the API
    result.text = resp.content.joined_texts().unwrap_or_default();

    // Extract tool calls using the method
    let tool_calls = resp.tool_calls();
    result.tool_calls = tool_calls
        .into_iter()
        .map(|tc| ParsedToolCall {
            id: tc.call_id.clone(),
            name: tc.fn_name.clone(),
            arguments: tc.fn_arguments.clone(),
        })
        .collect();

    // Extract usage
    result.prompt_tokens = resp.usage.prompt_tokens.map(|t| t as u64);
    result.completion_tokens = resp.usage.completion_tokens.map(|t| t as u64);

    // Set finish reason based on whether there are tool calls
    if !result.tool_calls.is_empty() {
        result.finish_reason = Some("tool_calls".to_string());
    } else {
        result.finish_reason = Some("stop".to_string());
    }

    result
}

/// Extract text from genai MessageContent.
fn extract_text_content(content: &MessageContent) -> String {
    content.joined_texts().unwrap_or_default()
}

/// Extract tool calls from MessageContent.
fn extract_tool_calls(content: &MessageContent) -> Vec<ParsedToolCall> {
    content
        .tool_calls()
        .into_iter()
        .map(|tc| ParsedToolCall {
            id: tc.call_id.clone(),
            name: tc.fn_name.clone(),
            arguments: tc.fn_arguments.clone(),
        })
        .collect()
}

// ── Stream Event Conversions ────────────────────────────────────────────────

/// Event types that can come from a chat stream, mapped to RustyClaw concepts.
#[derive(Debug)]
pub enum StreamEvent {
    /// Stream has started.
    Start,
    /// Text content chunk.
    TextChunk(String),
    /// Reasoning/thinking content chunk.
    ThinkingChunk(String),
    /// Tool call is being streamed.
    ToolCallChunk {
        id: String,
        name: String,
        arguments_delta: String,
    },
    /// Stream has ended with final response data.
    End { response: ModelResponse },
}

/// Convert a genai ChatStreamEvent to our StreamEvent.
pub fn genai_stream_event_to_rc(event: ChatStreamEvent) -> StreamEvent {
    match event {
        ChatStreamEvent::Start => StreamEvent::Start,
        ChatStreamEvent::Chunk(chunk) => StreamEvent::TextChunk(chunk.content),
        ChatStreamEvent::ReasoningChunk(chunk) => StreamEvent::ThinkingChunk(chunk.content),
        ChatStreamEvent::ThoughtSignatureChunk(chunk) => StreamEvent::ThinkingChunk(chunk.content),
        ChatStreamEvent::ToolCallChunk(tc) => StreamEvent::ToolCallChunk {
            id: tc.tool_call.call_id,
            name: tc.tool_call.fn_name,
            arguments_delta: tc.tool_call.fn_arguments.to_string(),
        },
        ChatStreamEvent::End(end) => {
            let response = stream_end_to_model_response(&end);
            StreamEvent::End { response }
        }
    }
}

/// Convert a StreamEnd to ModelResponse.
pub fn stream_end_to_model_response(end: &StreamEnd) -> ModelResponse {
    let mut response = ModelResponse::default();

    // Extract accumulated content
    if let Some(ref content) = end.captured_content {
        response.text = extract_text_content(content);
        response.tool_calls = extract_tool_calls(content);
    }

    // Extract usage
    if let Some(ref usage) = end.captured_usage {
        response.prompt_tokens = usage.prompt_tokens.map(|t| t as u64);
        response.completion_tokens = usage.completion_tokens.map(|t| t as u64);
    }

    // Set finish reason
    if !response.tool_calls.is_empty() {
        response.finish_reason = Some("tool_calls".to_string());
    } else {
        response.finish_reason = Some("stop".to_string());
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rc_to_genai_user_message() {
        let msg = RcChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            ..Default::default()
        };
        let _genai_msg = rc_to_genai_message(&msg);
        // Just verify it doesn't panic
    }

    #[test]
    fn test_extract_text_content_empty() {
        let content = MessageContent::default();
        assert_eq!(extract_text_content(&content), "");
    }

    #[test]
    fn test_extract_text_content_single() {
        let content = MessageContent::from_text("Hello world");
        assert_eq!(extract_text_content(&content), "Hello world");
    }

    #[test]
    fn test_rc_to_genai_tool_result_message() {
        let msg = RcChatMessage {
            role: "tool".to_string(),
            content: "42°C".to_string(),
            tool_call_id: Some("call_abc".to_string()),
            ..Default::default()
        };
        // Should not panic; genai encodes this as a user message carrying a ToolResponse.
        let _genai_msg = rc_to_genai_message(&msg);
    }

    #[test]
    fn test_rc_to_genai_assistant_with_tool_calls() {
        use serde_json::json;
        let tool_calls_json = json!([{
            "id": "call_1",
            "type": "function",
            "function": {
                "name": "get_weather",
                "arguments": "{\"city\": \"London\"}"
            }
        }]);
        let msg = RcChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(tool_calls_json),
            ..Default::default()
        };
        // Should use assistant_tool_calls_with_thoughts, not assistant("").
        let _genai_msg = rc_to_genai_message(&msg);
    }

    #[test]
    fn test_json_to_genai_tool_calls_openai_format() {
        use serde_json::json;
        let tc_json = json!([{
            "id": "call_xyz",
            "type": "function",
            "function": {
                "name": "search",
                "arguments": "{\"query\": \"Rust\"}"
            }
        }]);
        let calls = json_to_genai_tool_calls(&tc_json);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_id, "call_xyz");
        assert_eq!(calls[0].fn_name, "search");
        assert_eq!(calls[0].fn_arguments["query"], "Rust");
    }

    #[test]
    fn test_json_to_genai_tool_calls_empty() {
        use serde_json::json;
        let calls = json_to_genai_tool_calls(&json!([]));
        assert!(calls.is_empty());
        let calls = json_to_genai_tool_calls(&json!(null));
        assert!(calls.is_empty());
    }
}

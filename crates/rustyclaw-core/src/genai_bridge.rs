//! Type conversions between genai and RustyClaw gateway types.
//!
//! This module bridges the genai crate's types with RustyClaw's internal
//! protocol types, allowing gradual migration from hand-rolled provider
//! code to the genai abstraction.

use crate::gateway::protocol::types::{ChatMessage as RcChatMessage, ModelResponse, ParsedToolCall};
use genai::chat::{
    ChatMessage as GenaiChatMessage, ChatResponse, ChatStreamEvent, MessageContent,
    StreamEnd,
};

// ── Message Conversions ─────────────────────────────────────────────────────

/// Convert RustyClaw ChatMessage to genai ChatMessage.
pub fn rc_to_genai_message(msg: &RcChatMessage) -> GenaiChatMessage {
    match msg.role.as_str() {
        "system" => GenaiChatMessage::system(&msg.content),
        "user" => GenaiChatMessage::user(&msg.content),
        "assistant" => GenaiChatMessage::assistant(&msg.content),
        // For tool results, genai uses a different structure
        "tool" => {
            // Tool messages in RustyClaw have tool_call_id in a specific format
            // For now, pass as user message (genai handles tool results differently)
            GenaiChatMessage::user(&msg.content)
        }
        _ => GenaiChatMessage::user(&msg.content),
    }
}

/// Convert a slice of RustyClaw messages to genai messages.
pub fn rc_to_genai_messages(messages: &[RcChatMessage]) -> Vec<GenaiChatMessage> {
    messages.iter().map(rc_to_genai_message).collect()
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
}

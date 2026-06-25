//! genai-backed provider dispatch.
//!
//! This is the shared multi-provider chat backend, built on the [`genai`]
//! crate (request building, tool calling, SSE streaming). It lives in
//! `rustyclaw-core` so every crate that depends on core — the gateway and the
//! client crates — shares one genai instance and one provider mapping.
//!
//! RustyClaw still owns provider selection, credentials / Copilot session
//! tokens, and the binary streaming frame protocol; genai owns the wire
//! format for each provider. The bridge works as follows:
//!
//! * Each RustyClaw provider id maps onto a genai [`AdapterKind`]
//!   (see [`adapter_for`]). OpenAI-compatible providers (openrouter, ollama,
//!   lmstudio, exo, opencode, github-copilot, custom, …) all use the OpenAI
//!   adapter pointed at their configured base URL.
//! * The configured base URL + resolved API key are injected via a
//!   [`ServiceTargetResolver`], so genai never consults environment defaults.
//! * The conversation ([`ProviderRequest`]) is converted into a genai
//!   [`ChatRequest`] by [`to_genai_chat_request`]. Tool-loop continuation
//!   messages use the canonical encoding produced by
//!   [`encode_assistant_message`] / [`encode_tool_result`], decoded back into
//!   genai tool calls / responses here.
//! * Streaming events are forwarded to the client as binary frames; the
//!   non-streaming path is used for internal calls (compaction, summaries).

use anyhow::Result;
use futures_util::StreamExt;
use serde_json::json;
use tracing::{debug, warn};

use genai::Client;
use genai::adapter::AdapterKind;
use genai::chat::{
    ChatMessage as GenChatMessage, ChatOptions, ChatRequest, ChatStreamEvent, ContentPart,
    MessageContent, Tool, ToolCall, ToolResponse,
};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{ModelIden, ServiceTarget};

use crate::gateway::protocol::server;
use crate::gateway::transport::TransportWriter;
use crate::gateway::{ModelResponse, ParsedToolCall, ProviderRequest, ToolCallResult};
use crate::providers;
use crate::tools;

/// Generous default output budget. Anthropic requires `max_tokens`, and the
/// previous implementation used this same ceiling across providers.
const MAX_TOKENS: u32 = 16384;

// ── Public entry points (preserve the previous call surface) ────────────────

/// Call an OpenAI-compatible provider with tools. Streams when a writer is set.
pub async fn call_openai_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
    writer: Option<&mut dyn TransportWriter>,
) -> Result<ModelResponse> {
    genai_chat(http, req, writer).await
}

/// Call Anthropic with tools. Streams (text + thinking) when a writer is set.
pub async fn call_anthropic_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
    writer: Option<&mut dyn TransportWriter>,
) -> Result<ModelResponse> {
    genai_chat(http, req, writer).await
}

/// Call Google Gemini with tools (non-streaming, matching prior behaviour).
pub async fn call_google_with_tools(
    http: &reqwest::Client,
    req: &ProviderRequest,
) -> Result<ModelResponse> {
    genai_chat(http, req, None).await
}

// ── Canonical tool-loop message encoding ─────────────────────────────────────

/// Encode an assistant turn (text + tool calls) into RustyClaw's canonical,
/// provider-agnostic envelope, decoded by [`decode_assistant`].
///
/// Previously this produced a different JSON shape per provider; the genai
/// migration uses one neutral encoding regardless of provider.
pub fn encode_assistant_message(model_resp: &ModelResponse) -> String {
    let tool_calls: Vec<serde_json::Value> = model_resp
        .tool_calls
        .iter()
        .map(|tc| {
            json!({
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            })
        })
        .collect();

    json!({
        "__rustyclaw_kind": "assistant_tools",
        "text": model_resp.text,
        "tool_calls": tool_calls,
    })
    .to_string()
}

/// Encode a single tool result into the canonical `tool_result` envelope,
/// decoded by [`decode_tool_result`].
pub fn encode_tool_result(result: &ToolCallResult) -> String {
    json!({
        "__rustyclaw_kind": "tool_result",
        "id": result.id,
        "name": result.name,
        "output": result.output,
        "is_error": result.is_error,
    })
    .to_string()
}

// ── Core dispatch ───────────────────────────────────────────────────────────

/// Drive a chat turn through genai. When `writer` is `Some`, the response is
/// streamed to the client as binary frames; otherwise it is returned in batch.
async fn genai_chat(
    http: &reqwest::Client,
    req: &ProviderRequest,
    writer: Option<&mut dyn TransportWriter>,
) -> Result<ModelResponse> {
    debug!(
        provider = %req.provider,
        model = %req.model,
        base_url = %req.base_url,
        messages = req.messages.len(),
        streaming = writer.is_some(),
        "Starting genai chat request"
    );

    let client = build_client(http, req);
    let chat_req = to_genai_chat_request(req);

    let copilot = providers::needs_copilot_session(&req.provider);
    let mut options = ChatOptions::default().with_max_tokens(MAX_TOKENS);
    // Copilot/proxy endpoints reject the `stream_options.include_usage` field
    // that genai adds when usage capture is on, so skip usage there.
    options = options.with_capture_usage(!copilot);
    if let Some(headers) = copilot_extra_headers(req) {
        options = options.with_extra_headers(headers);
    }

    match writer {
        Some(w) => {
            let options = options
                .with_capture_content(true)
                .with_capture_tool_calls(true)
                .with_capture_reasoning_content(true);
            let stream = client
                .exec_chat_stream(&req.model, chat_req, Some(&options))
                .await
                .map_err(genai_err)?;
            consume_stream(stream.stream, w).await
        }
        None => {
            let resp = client
                .exec_chat(&req.model, chat_req, Some(&options))
                .await
                .map_err(genai_err)?;
            Ok(chat_response_to_model_response(resp))
        }
    }
}

/// Consume a genai stream, forwarding text/thinking chunks to the client and
/// assembling the final [`ModelResponse`].
async fn consume_stream(
    mut stream: genai::chat::ChatStream,
    writer: &mut dyn TransportWriter,
) -> Result<ModelResponse> {
    let mut result = ModelResponse::default();
    let mut stream_started = false;
    let mut thinking_started = false;

    while let Some(event) = stream.next().await {
        match event.map_err(genai_err)? {
            ChatStreamEvent::Start => {
                server::send_stream_start(writer).await?;
                stream_started = true;
            }
            ChatStreamEvent::Chunk(chunk) => {
                if !stream_started {
                    server::send_stream_start(writer).await?;
                    stream_started = true;
                }
                result.text.push_str(&chunk.content);
                server::send_chunk(writer, &chunk.content).await?;
            }
            ChatStreamEvent::ReasoningChunk(chunk) => {
                if !thinking_started {
                    let _ = server::send_thinking_start(writer).await;
                    thinking_started = true;
                }
                let _ = server::send_thinking_delta(writer, &chunk.content).await;
            }
            ChatStreamEvent::ToolCallChunk(_) => {
                // Tool calls are assembled from the captured content in `End`.
            }
            ChatStreamEvent::ThoughtSignatureChunk(_) => {}
            ChatStreamEvent::End(end) => {
                if thinking_started {
                    let _ = server::send_thinking_end(writer).await;
                }
                if let Some(content) = end.captured_content {
                    for part in content.into_parts() {
                        match part {
                            ContentPart::ToolCall(tc) => result.tool_calls.push(to_parsed_call(tc)),
                            ContentPart::Text(t) if result.text.is_empty() => result.text = t,
                            _ => {}
                        }
                    }
                }
                if let Some(usage) = end.captured_usage {
                    result.prompt_tokens = usage.prompt_tokens.map(|t| t.max(0) as u64);
                    result.completion_tokens = usage.completion_tokens.map(|t| t.max(0) as u64);
                }
            }
        }
    }

    result.finish_reason = Some(finish_reason_for(&result).to_string());
    Ok(result)
}

/// Convert a non-streaming genai [`ChatResponse`] into a [`ModelResponse`].
fn chat_response_to_model_response(resp: genai::chat::ChatResponse) -> ModelResponse {
    let mut result = ModelResponse {
        prompt_tokens: resp.usage.prompt_tokens.map(|t| t.max(0) as u64),
        completion_tokens: resp.usage.completion_tokens.map(|t| t.max(0) as u64),
        ..Default::default()
    };

    for part in resp.content.into_parts() {
        match part {
            ContentPart::Text(t) => {
                if !result.text.is_empty() {
                    result.text.push('\n');
                }
                result.text.push_str(&t);
            }
            ContentPart::ToolCall(tc) => result.tool_calls.push(to_parsed_call(tc)),
            _ => {}
        }
    }

    result.finish_reason = Some(finish_reason_for(&result).to_string());
    result
}

// ── Conversion helpers ───────────────────────────────────────────────────────

/// Build a genai client configured for this request's provider, base URL, and
/// resolved API key. The caller's `reqwest::Client` is reused so connection
/// settings are shared.
fn build_client(http: &reqwest::Client, req: &ProviderRequest) -> Client {
    let adapter = adapter_for(&req.provider);
    let base_url = normalize_base_url(adapter, &req.base_url);
    let api_key = req.api_key.clone().unwrap_or_default();
    let model = req.model.clone();

    let resolver = ServiceTargetResolver::from_resolver_fn(
        move |mut target: ServiceTarget| -> genai::resolver::Result<ServiceTarget> {
            target.endpoint = Endpoint::from_owned(base_url.clone());
            target.auth = AuthData::from_single(api_key.clone());
            target.model = ModelIden::new(adapter, model.clone());
            Ok(target)
        },
    );

    Client::builder()
        .with_reqwest(http.clone())
        .with_service_target_resolver(resolver)
        .build()
}

/// Map a RustyClaw provider id onto a genai adapter. Anthropic, Google, and
/// xAI have native adapters; every other (OpenAI-compatible) provider uses the
/// OpenAI adapter pointed at its configured base URL.
fn adapter_for(provider: &str) -> AdapterKind {
    match provider {
        "anthropic" => AdapterKind::Anthropic,
        "google" => AdapterKind::Gemini,
        "xai" => AdapterKind::Xai,
        _ => AdapterKind::OpenAI,
    }
}

/// Normalise a configured base URL into the form each genai adapter expects.
///
/// genai builds request URLs by joining/concatenating onto the endpoint base,
/// so a trailing slash is required. The Anthropic adapter appends `messages`
/// directly to the base, so the base must include the `/v1/` segment.
fn normalize_base_url(adapter: AdapterKind, base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    match adapter {
        AdapterKind::Anthropic => {
            if trimmed.ends_with("/v1") {
                format!("{trimmed}/")
            } else {
                format!("{trimmed}/v1/")
            }
        }
        _ => format!("{trimmed}/"),
    }
}

/// Convert the resolved [`ProviderRequest`] into a genai [`ChatRequest`],
/// decoding RustyClaw's canonical tool-call / tool-result encoding back into
/// genai tool calls and responses.
fn to_genai_chat_request(req: &ProviderRequest) -> ChatRequest {
    let mut messages: Vec<GenChatMessage> = Vec::with_capacity(req.messages.len());

    for msg in &req.messages {
        match msg.role.as_str() {
            "system" => messages.push(GenChatMessage::system(msg.content.clone())),
            "assistant" => messages.push(decode_assistant(&msg.content)),
            "tool" => messages.push(decode_tool_result(&msg.content)),
            // Treat user and any unknown role as user content.
            _ => messages.push(GenChatMessage::user(msg.content.clone())),
        }
    }

    let mut chat_req = ChatRequest::new(messages);
    let tools = tools_for_genai();
    if !tools.is_empty() {
        chat_req = chat_req.with_tools(tools);
    }
    chat_req
}

/// Decode an assistant message. Plain text passes through; the canonical
/// `assistant_tools` envelope is expanded into a text part plus tool calls.
fn decode_assistant(content: &str) -> GenChatMessage {
    if let Some(env) = parse_canonical(content, "assistant_tools") {
        let mut parts: Vec<ContentPart> = Vec::new();
        if let Some(text) = env.get("text").and_then(|v| v.as_str()) {
            if !text.trim().is_empty() {
                parts.push(ContentPart::from_text(text.to_string()));
            }
        }
        if let Some(calls) = env.get("tool_calls").and_then(|v| v.as_array()) {
            debug!(
                tool_calls_count = calls.len(),
                "Decoding assistant message with tool calls"
            );
            for (idx, tc) in calls.iter().enumerate() {
                let call_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let fn_name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let fn_arguments = tc
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                debug!(
                    tool_call_index = idx,
                    call_id = %call_id,
                    fn_name = %fn_name,
                    arguments_type = %fn_arguments.to_string().chars().take(50).collect::<String>(),
                    "Decoded tool call"
                );

                parts.push(ContentPart::ToolCall(ToolCall {
                    call_id: call_id.to_string(),
                    fn_name: fn_name.to_string(),
                    fn_arguments,
                    thought_signatures: None,
                }));
            }
        }
        return GenChatMessage::assistant(MessageContent::from_parts(parts));
    }
    GenChatMessage::assistant(content.to_string())
}

/// Decode a tool-result message from the canonical `tool_result` envelope.
fn decode_tool_result(content: &str) -> GenChatMessage {
    if let Some(env) = parse_canonical(content, "tool_result") {
        let call_id = env.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let output = env.get("output").and_then(|v| v.as_str()).unwrap_or("");
        return GenChatMessage::from(ToolResponse::new(call_id.to_string(), output.to_string()));
    }
    // Fallback: forward the raw content as an (unanchored) tool response.
    GenChatMessage::from(ToolResponse::new(String::new(), content.to_string()))
}

/// Parse a canonical RustyClaw envelope, verifying the `__rustyclaw_kind` tag.
fn parse_canonical(content: &str, kind: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            // Only log at debug level since this is expected for non-JSON content
            debug!(
                error = %e,
                content_preview = %content.chars().take(100).collect::<String>(),
                expected_kind = kind,
                "Failed to parse canonical envelope as JSON (this is normal for plain text messages)"
            );
            return None;
        }
    };

    if value.get("__rustyclaw_kind").and_then(|v| v.as_str()) == Some(kind) {
        Some(value)
    } else {
        None
    }
}

/// Build genai tool definitions from RustyClaw's tool registry, reusing the
/// OpenAI function-schema formatter as the source of truth.
fn tools_for_genai() -> Vec<Tool> {
    if std::env::var("RUSTYCLAW_SKIP_TOOLS").is_ok() {
        return Vec::new();
    }
    tools::tools_openai()
        .into_iter()
        .filter_map(|v| {
            let function = v.get("function")?;
            let name = function.get("name")?.as_str()?.to_string();
            let mut tool = Tool::new(name);
            if let Some(desc) = function.get("description").and_then(|d| d.as_str()) {
                tool = tool.with_description(desc.to_string());
            }
            if let Some(params) = function.get("parameters") {
                tool = tool.with_schema(params.clone());
            }
            Some(tool)
        })
        .collect()
}

/// Convert a genai [`ToolCall`] into RustyClaw's [`ParsedToolCall`].
fn to_parsed_call(tc: ToolCall) -> ParsedToolCall {
    ParsedToolCall {
        id: tc.call_id,
        name: tc.fn_name,
        arguments: tc.fn_arguments,
    }
}

/// Synthesize a finish reason. genai 0.5.3 does not surface the provider's
/// raw finish reason on the response, so the dispatch loop distinguishes a
/// tool-call turn from a completed turn by whether tool calls are present.
fn finish_reason_for(resp: &ModelResponse) -> &'static str {
    if resp.tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    }
}

/// Build the GitHub Copilot IDE headers required by the Copilot chat API.
/// Returns `None` for non-Copilot providers.
fn copilot_extra_headers(req: &ProviderRequest) -> Option<genai::Headers> {
    if !providers::needs_copilot_session(&req.provider) {
        return None;
    }
    // Agent-initiated unless the last message is from the user.
    let is_agent_call = req
        .messages
        .last()
        .map(|m| m.role != "user")
        .unwrap_or(false);
    let x_initiator = if is_agent_call { "agent" } else { "user" };

    let headers: Vec<(String, String)> = vec![
        (
            "User-Agent".to_string(),
            providers::COPILOT_API_USER_AGENT.to_string(),
        ),
        (
            "Editor-Version".to_string(),
            providers::COPILOT_EDITOR_VERSION.to_string(),
        ),
        (
            "Editor-Plugin-Version".to_string(),
            providers::COPILOT_EDITOR_PLUGIN_VERSION.to_string(),
        ),
        (
            "Copilot-Integration-Id".to_string(),
            providers::COPILOT_INTEGRATION_ID.to_string(),
        ),
        (
            "Openai-Intent".to_string(),
            "conversation-edits".to_string(),
        ),
        ("X-Initiator".to_string(), x_initiator.to_string()),
    ];
    Some(genai::Headers::from(headers))
}

/// Wrap a genai error as an `anyhow::Error`, preserving the full message chain
/// (status code + response body) so callers' auth-error detection still works.
fn genai_err(err: genai::Error) -> anyhow::Error {
    // Log the full error for debugging, including any nested causes
    warn!(
        error = %err,
        error_debug = ?err,
        "genai API call failed"
    );

    // Extract additional context from the error if available
    let error_msg = format!("{err}");

    // Check for common error patterns and add helpful context
    if error_msg.to_lowercase().contains("invalid json")
        || error_msg.to_lowercase().contains("json format")
    {
        anyhow::anyhow!(
            "Web stream error for model. Cause: HTTP error. Body: {}",
            error_msg
        )
    } else {
        anyhow::anyhow!("{err}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genai::chat::ChatRole;

    #[test]
    fn adapter_mapping() {
        assert_eq!(adapter_for("anthropic"), AdapterKind::Anthropic);
        assert_eq!(adapter_for("google"), AdapterKind::Gemini);
        assert_eq!(adapter_for("xai"), AdapterKind::Xai);
        // Everything OpenAI-compatible falls through to the OpenAI adapter.
        assert_eq!(adapter_for("openai"), AdapterKind::OpenAI);
        assert_eq!(adapter_for("openrouter"), AdapterKind::OpenAI);
        assert_eq!(adapter_for("ollama"), AdapterKind::OpenAI);
        assert_eq!(adapter_for("github-copilot"), AdapterKind::OpenAI);
    }

    #[test]
    fn base_url_normalization() {
        // OpenAI-family: just needs a trailing slash for URL joining.
        assert_eq!(
            normalize_base_url(AdapterKind::OpenAI, "https://api.openai.com/v1"),
            "https://api.openai.com/v1/"
        );
        assert_eq!(
            normalize_base_url(
                AdapterKind::Gemini,
                "https://generativelanguage.googleapis.com/v1beta"
            ),
            "https://generativelanguage.googleapis.com/v1beta/"
        );
        // Anthropic: base must include the /v1/ segment.
        assert_eq!(
            normalize_base_url(AdapterKind::Anthropic, "https://api.anthropic.com"),
            "https://api.anthropic.com/v1/"
        );
        assert_eq!(
            normalize_base_url(AdapterKind::Anthropic, "https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/"
        );
        assert_eq!(
            normalize_base_url(AdapterKind::Anthropic, "https://api.anthropic.com/v1/"),
            "https://api.anthropic.com/v1/"
        );
    }

    #[test]
    fn decode_plain_assistant() {
        let msg = decode_assistant("hello world");
        assert_eq!(msg.role, ChatRole::Assistant);
        assert_eq!(msg.content.first_text(), Some("hello world"));
        assert!(msg.content.tool_calls().is_empty());
    }

    #[test]
    fn assistant_round_trip() {
        // Encode via the canonical encoder, then decode back into genai parts.
        let model_resp = ModelResponse {
            text: "let me check".to_string(),
            tool_calls: vec![ParsedToolCall {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: json!({ "path": "a.rs" }),
            }],
            ..Default::default()
        };
        let encoded = encode_assistant_message(&model_resp);
        let msg = decode_assistant(&encoded);
        assert_eq!(msg.role, ChatRole::Assistant);
        assert_eq!(msg.content.first_text(), Some("let me check"));
        let calls = msg.content.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_id, "call_1");
        assert_eq!(calls[0].fn_name, "read_file");
        assert_eq!(calls[0].fn_arguments["path"], "a.rs");
    }

    #[test]
    fn tool_result_round_trip() {
        let result = ToolCallResult {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            output: "file body".to_string(),
            is_error: false,
        };
        let encoded = encode_tool_result(&result);
        let msg = decode_tool_result(&encoded);
        assert_eq!(msg.role, ChatRole::Tool);
        let responses = msg.content.tool_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].call_id, "call_1");
        assert_eq!(responses[0].content, "file body");
    }

    #[test]
    fn to_chat_request_routes_roles_and_tools() {
        let req = ProviderRequest {
            messages: vec![
                crate::gateway::ChatMessage::text("system", "be brief"),
                crate::gateway::ChatMessage::text("user", "hi"),
            ],
            model: "gpt-4.1".to_string(),
            provider: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: Some("sk-test".to_string()),
        };
        // Avoid pulling the full tool registry into the assertion.
        unsafe { std::env::set_var("RUSTYCLAW_SKIP_TOOLS", "1") };
        let chat_req = to_genai_chat_request(&req);
        unsafe { std::env::remove_var("RUSTYCLAW_SKIP_TOOLS") };

        assert_eq!(chat_req.messages.len(), 2);
        assert_eq!(chat_req.messages[0].role, ChatRole::System);
        assert_eq!(chat_req.messages[1].role, ChatRole::User);
        assert!(chat_req.tools.is_none());
    }
}

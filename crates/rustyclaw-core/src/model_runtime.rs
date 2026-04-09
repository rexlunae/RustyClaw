use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::StreamExt;
use rig;
use rig::client::VerifyClient;
use rig::client::completion::CompletionClient;
use rig::completion::{CompletionModel, CompletionRequest, GetTokenUsage, Message};
use rig::message::{
    AssistantContent, Text, ToolCall, ToolFunction, ToolResult, ToolResultContent, UserContent,
};
use rig::streaming::{StreamedAssistantContent, ToolCallDeltaContent};
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::gateway::providers as legacy;
use crate::gateway::transport::TransportWriter;
use crate::gateway::{ChatMessage, ModelResponse, ParsedToolCall, ProbeResult, ProviderRequest};
use crate::providers::ModelInfo;
use crate::tools;

#[async_trait]
trait ModelRuntime {
    fn supports_provider(&self, provider: &str) -> bool;

    async fn execute(
        &self,
        http: &reqwest::Client,
        req: &ProviderRequest,
        writer: Option<&mut dyn TransportWriter>,
    ) -> Result<ModelResponse>;

    async fn validate(
        &self,
        http: &reqwest::Client,
        ctx: &crate::gateway::ModelContext,
        copilot_session: Option<&crate::gateway::CopilotSession>,
    ) -> ProbeResult;
}

pub async fn execute_completion(
    http: &reqwest::Client,
    req: &ProviderRequest,
    writer: Option<&mut dyn TransportWriter>,
) -> Result<ModelResponse> {
    let runtime = HybridModelRuntime::default();
    runtime.execute(http, req, writer).await
}

pub async fn validate_model_connection(
    http: &reqwest::Client,
    ctx: &crate::gateway::ModelContext,
    copilot_session: Option<&crate::gateway::CopilotSession>,
) -> ProbeResult {
    let runtime = HybridModelRuntime::default();
    runtime.validate(http, ctx, copilot_session).await
}

pub async fn list_models_detailed(
    provider_id: &str,
    api_key: Option<&str>,
    base_url_override: Option<&str>,
) -> std::result::Result<Vec<ModelInfo>, String> {
    crate::providers::fetch_models_detailed(provider_id, api_key, base_url_override).await
}

#[doc(hidden)]
pub fn convert_chat_message_for_test(msg: &ChatMessage) -> Result<Message> {
    chat_message_to_rig_message(msg)
}

#[doc(hidden)]
pub fn converted_tool_count_for_test(msg: &ChatMessage) -> Result<usize> {
    Ok(match chat_message_to_rig_message(msg)? {
        Message::Assistant { content, .. } => content
            .iter()
            .filter(|item| matches!(item, AssistantContent::ToolCall(_)))
            .count(),
        Message::User { content } => content
            .iter()
            .filter(|item| matches!(item, UserContent::ToolResult(_)))
            .count(),
        Message::System { .. } => 0,
    })
}

#[doc(hidden)]
pub fn converted_text_for_test(msg: &ChatMessage) -> Result<String> {
    Ok(match chat_message_to_rig_message(msg)? {
        Message::Assistant { content, .. } => content
            .iter()
            .filter_map(|item| match item {
                AssistantContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        Message::User { content } => content
            .iter()
            .filter_map(|item| match item {
                UserContent::Text(text) => Some(text.text.clone()),
                UserContent::ToolResult(tool_result) => {
                    tool_result.content.iter().find_map(|result| match result {
                        ToolResultContent::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        Message::System { content } => content,
    })
}

#[doc(hidden)]
pub fn response_from_assistant_items_for_test(items: Vec<AssistantContent>) -> ModelResponse {
    model_response_from_choice(items, None)
}

#[doc(hidden)]
pub fn normalize_finish_reason_for_test(no_tool_calls: bool) -> &'static str {
    normalize_finish_reason(no_tool_calls)
}

#[derive(Default)]
struct HybridModelRuntime {
    rig: RigModelRuntime,
    legacy: LegacyModelRuntime,
}

#[async_trait]
impl ModelRuntime for HybridModelRuntime {
    fn supports_provider(&self, provider: &str) -> bool {
        self.rig.supports_provider(provider) || self.legacy.supports_provider(provider)
    }

    async fn execute(
        &self,
        http: &reqwest::Client,
        req: &ProviderRequest,
        writer: Option<&mut dyn TransportWriter>,
    ) -> Result<ModelResponse> {
        if self.rig.supports_provider(&req.provider) {
            self.rig.execute(http, req, writer).await
        } else {
            self.legacy.execute(http, req, writer).await
        }
    }

    async fn validate(
        &self,
        http: &reqwest::Client,
        ctx: &crate::gateway::ModelContext,
        copilot_session: Option<&crate::gateway::CopilotSession>,
    ) -> ProbeResult {
        if self.rig.supports_provider(&ctx.provider) {
            self.rig.validate(http, ctx, copilot_session).await
        } else {
            self.legacy.validate(http, ctx, copilot_session).await
        }
    }
}

#[derive(Default)]
struct LegacyModelRuntime;

#[async_trait]
impl ModelRuntime for LegacyModelRuntime {
    fn supports_provider(&self, _provider: &str) -> bool {
        true
    }

    async fn execute(
        &self,
        http: &reqwest::Client,
        req: &ProviderRequest,
        writer: Option<&mut dyn TransportWriter>,
    ) -> Result<ModelResponse> {
        match req.provider.as_str() {
            "anthropic" => legacy::call_anthropic_with_tools(http, req, writer).await,
            "google" => legacy::call_google_with_tools(http, req).await,
            _ => legacy::call_openai_with_tools(http, req).await,
        }
    }

    async fn validate(
        &self,
        http: &reqwest::Client,
        ctx: &crate::gateway::ModelContext,
        copilot_session: Option<&crate::gateway::CopilotSession>,
    ) -> ProbeResult {
        legacy::validate_model_connection(http, ctx, copilot_session).await
    }
}

#[derive(Default)]
struct RigModelRuntime;

#[async_trait]
impl ModelRuntime for RigModelRuntime {
    fn supports_provider(&self, provider: &str) -> bool {
        matches!(
            provider,
            "anthropic"
                | "google"
                | "ollama"
                | "openai"
                | "openrouter"
                | "xai"
                | "opencode"
                | "custom"
        )
    }

    async fn execute(
        &self,
        _http: &reqwest::Client,
        req: &ProviderRequest,
        writer: Option<&mut dyn TransportWriter>,
    ) -> Result<ModelResponse> {
        let rig_request = build_rig_request(req)?;

        match req.provider.as_str() {
            "anthropic" => {
                let client = build_anthropic_client(req)?;
                execute_with_model(
                    client.completion_model(req.model.clone()),
                    rig_request,
                    writer,
                )
                .await
            }
            "google" => {
                let client = build_gemini_client(req)?;
                execute_with_model(
                    client.completion_model(req.model.clone()),
                    rig_request,
                    writer,
                )
                .await
            }
            "openrouter" => {
                let client = build_openrouter_client(req)?;
                execute_with_model(
                    client.completion_model(req.model.clone()),
                    rig_request,
                    writer,
                )
                .await
            }
            "xai" => {
                let client = build_xai_client(req)?;
                execute_with_model(
                    client.completion_model(req.model.clone()),
                    rig_request,
                    writer,
                )
                .await
            }
            "ollama" => {
                let client = build_ollama_client(req)?;
                execute_with_model(
                    client.completion_model(req.model.clone()),
                    rig_request,
                    writer,
                )
                .await
            }
            _ => {
                let client = build_openai_compatible_client(req)?;
                execute_with_model(
                    client.completion_model(req.model.clone()),
                    rig_request,
                    writer,
                )
                .await
            }
        }
    }

    async fn validate(
        &self,
        _http: &reqwest::Client,
        ctx: &crate::gateway::ModelContext,
        _copilot_session: Option<&crate::gateway::CopilotSession>,
    ) -> ProbeResult {
        match ctx.provider.as_str() {
            "anthropic" => verify_client(build_anthropic_client_from_context(ctx)).await,
            "google" => verify_client(build_gemini_client_from_context(ctx)).await,
            "openrouter" => verify_client(build_openrouter_client_from_context(ctx)).await,
            "xai" => verify_client(build_xai_client_from_context(ctx)).await,
            "ollama" => verify_client(build_ollama_client_from_context(ctx)).await,
            _ => verify_client(build_openai_compatible_client_from_context(ctx)).await,
        }
    }
}

fn build_rig_request(req: &ProviderRequest) -> Result<CompletionRequest> {
    let messages = req
        .messages
        .iter()
        .map(chat_message_to_rig_message)
        .collect::<Result<Vec<_>>>()?;

    let chat_history = rig::OneOrMany::many(messages)
        .map_err(|_| anyhow!("Cannot build a completion request without any chat messages"))?;

    let tools = tools::tools_generic()
        .into_iter()
        .map(|tool| rig::completion::ToolDefinition {
            name: tool.name,
            description: tool.description,
            parameters: tool.parameters,
        })
        .collect();

    Ok(CompletionRequest {
        model: Some(req.model.clone()),
        preamble: None,
        chat_history,
        documents: vec![],
        tools,
        temperature: None,
        max_tokens: Some(16_384),
        tool_choice: None,
        additional_params: None,
        output_schema: None,
    })
}

fn chat_message_to_rig_message(msg: &ChatMessage) -> Result<Message> {
    if let Some(value) = parse_json(&msg.content) {
        if let Some(message) = parse_provider_structured_message(msg, &value)? {
            return Ok(message);
        }
    }

    match msg.role.as_str() {
        "system" => Ok(Message::System {
            content: msg.content.clone(),
        }),
        "assistant" => Ok(Message::Assistant {
            id: None,
            content: rig::OneOrMany::one(AssistantContent::Text(Text {
                text: msg.content.clone(),
            })),
        }),
        "tool" => Ok(Message::User {
            content: rig::OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: msg
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "tool_result".to_string()),
                call_id: None,
                content: rig::OneOrMany::one(ToolResultContent::Text(Text {
                    text: msg.content.clone(),
                })),
            })),
        }),
        _ => Ok(Message::User {
            content: rig::OneOrMany::one(UserContent::Text(Text {
                text: msg.display_content(),
            })),
        }),
    }
}

fn parse_provider_structured_message(msg: &ChatMessage, value: &Value) -> Result<Option<Message>> {
    if let Some(array) = value.as_array() {
        if array.iter().any(|item| item.get("type").is_some()) {
            return Ok(Some(parse_anthropic_structured_message(
                msg.role.as_str(),
                array,
            )?));
        }
        if array.iter().any(|item| {
            item.get("functionCall").is_some() || item.get("functionResponse").is_some()
        }) {
            return Ok(Some(parse_google_structured_message(
                msg.role.as_str(),
                array,
            )?));
        }
    }

    if value.get("role").is_some() {
        return Ok(Some(parse_openai_structured_message(value)?));
    }

    Ok(None)
}

fn parse_anthropic_structured_message(role: &str, blocks: &[Value]) -> Result<Message> {
    if role == "user" {
        let content = blocks
            .iter()
            .filter_map(|block| {
                let tool_use_id = block.get("tool_use_id")?.as_str()?.to_string();
                let text = block.get("content").map(text_from_json).unwrap_or_default();
                Some(UserContent::ToolResult(ToolResult {
                    id: tool_use_id,
                    call_id: None,
                    content: rig::OneOrMany::one(ToolResultContent::Text(Text { text })),
                }))
            })
            .collect::<Vec<_>>();

        return Ok(Message::User {
            content: one_or_many_user(content)?,
        });
    }

    let content = blocks
        .iter()
        .filter_map(|block| match block.get("type").and_then(Value::as_str) {
            Some("text") => Some(AssistantContent::Text(Text {
                text: block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })),
            Some("tool_use") => Some(AssistantContent::ToolCall(ToolCall {
                id: block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                call_id: None,
                function: ToolFunction {
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                },
                signature: None,
                additional_params: None,
            })),
            _ => None,
        })
        .collect::<Vec<_>>();

    Ok(Message::Assistant {
        id: None,
        content: one_or_many_assistant(content)?,
    })
}

fn parse_google_structured_message(role: &str, parts: &[Value]) -> Result<Message> {
    if role == "user" {
        let content = parts
            .iter()
            .filter_map(|part| {
                let response = part.get("functionResponse")?;
                let name = response
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let payload = response.get("response").cloned().unwrap_or(Value::Null);
                Some(UserContent::ToolResult(ToolResult {
                    id: name.to_string(),
                    call_id: None,
                    content: rig::OneOrMany::one(ToolResultContent::Text(Text {
                        text: text_from_json(&payload),
                    })),
                }))
            })
            .collect::<Vec<_>>();

        return Ok(Message::User {
            content: one_or_many_user(content)?,
        });
    }

    let content = parts
        .iter()
        .filter_map(|part| {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                return Some(AssistantContent::Text(Text {
                    text: text.to_string(),
                }));
            }

            part.get("functionCall").map(|call| {
                AssistantContent::ToolCall(ToolCall {
                    id: call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool_call")
                        .to_string(),
                    call_id: None,
                    function: ToolFunction {
                        name: call
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: call.get("args").cloned().unwrap_or_else(|| json!({})),
                    },
                    signature: None,
                    additional_params: None,
                })
            })
        })
        .collect::<Vec<_>>();

    Ok(Message::Assistant {
        id: None,
        content: one_or_many_assistant(content)?,
    })
}

fn parse_openai_structured_message(value: &Value) -> Result<Message> {
    match value.get("role").and_then(Value::as_str).unwrap_or("user") {
        "assistant" => {
            let mut content = Vec::new();

            if let Some(text) = value.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    content.push(AssistantContent::Text(Text {
                        text: text.to_string(),
                    }));
                }
            }

            if let Some(tool_calls) = value.get("tool_calls").and_then(Value::as_array) {
                for tool_call in tool_calls {
                    let name = tool_call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let arguments = tool_call
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or_else(|| json!({}));
                    content.push(AssistantContent::ToolCall(ToolCall {
                        id: tool_call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        call_id: None,
                        function: ToolFunction { name, arguments },
                        signature: None,
                        additional_params: None,
                    }));
                }
            }

            Ok(Message::Assistant {
                id: None,
                content: one_or_many_assistant(content)?,
            })
        }
        "tool" => Ok(Message::User {
            content: rig::OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: value
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_result")
                    .to_string(),
                call_id: None,
                content: rig::OneOrMany::one(ToolResultContent::Text(Text {
                    text: value
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })),
            })),
        }),
        "system" => Ok(Message::System {
            content: value
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        _ => Ok(Message::User {
            content: rig::OneOrMany::one(UserContent::Text(Text {
                text: value
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })),
        }),
    }
}

fn one_or_many_assistant(
    content: Vec<AssistantContent>,
) -> Result<rig::OneOrMany<AssistantContent>> {
    rig::OneOrMany::many(content)
        .map_err(|_| anyhow!("Structured assistant message did not contain any content"))
}

fn one_or_many_user(content: Vec<UserContent>) -> Result<rig::OneOrMany<UserContent>> {
    rig::OneOrMany::many(content)
        .map_err(|_| anyhow!("Structured user message did not contain any content"))
}

fn parse_json(content: &str) -> Option<Value> {
    serde_json::from_str(content).ok()
}

fn text_from_json(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn normalize_base_url(provider: &str, base_url: &str) -> String {
    match provider {
        "anthropic" => base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .to_string(),
        "google" => base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1beta")
            .to_string(),
        _ => base_url.to_string(),
    }
}

fn build_openai_compatible_client(
    req: &ProviderRequest,
) -> Result<rig::providers::openai::CompletionsClient> {
    build_openai_compatible_client_from_parts(&req.provider, req.api_key.as_deref(), &req.base_url)
}

fn build_openai_compatible_client_from_context(
    ctx: &crate::gateway::ModelContext,
) -> Result<rig::providers::openai::CompletionsClient> {
    build_openai_compatible_client_from_parts(&ctx.provider, ctx.api_key.as_deref(), &ctx.base_url)
}

fn build_openai_compatible_client_from_parts(
    provider: &str,
    api_key: Option<&str>,
    base_url: &str,
) -> Result<rig::providers::openai::CompletionsClient> {
    let normalized = normalize_base_url(provider, base_url);
    Ok(rig::providers::openai::Client::builder()
        .api_key(api_key.unwrap_or("local"))
        .base_url(&normalized)
        .build()?
        .completions_api())
}

fn build_openrouter_client(req: &ProviderRequest) -> Result<rig::providers::openrouter::Client> {
    build_openrouter_client_from_parts(req.api_key.as_deref(), &req.base_url)
}

fn build_openrouter_client_from_context(
    ctx: &crate::gateway::ModelContext,
) -> Result<rig::providers::openrouter::Client> {
    build_openrouter_client_from_parts(ctx.api_key.as_deref(), &ctx.base_url)
}

fn build_openrouter_client_from_parts(
    api_key: Option<&str>,
    base_url: &str,
) -> Result<rig::providers::openrouter::Client> {
    Ok(rig::providers::openrouter::Client::builder()
        .api_key(api_key.unwrap_or_default())
        .base_url(base_url)
        .build()?)
}

fn build_xai_client(req: &ProviderRequest) -> Result<rig::providers::xai::Client> {
    build_xai_client_from_parts(req.api_key.as_deref(), &req.base_url)
}

fn build_xai_client_from_context(
    ctx: &crate::gateway::ModelContext,
) -> Result<rig::providers::xai::Client> {
    build_xai_client_from_parts(ctx.api_key.as_deref(), &ctx.base_url)
}

fn build_xai_client_from_parts(
    api_key: Option<&str>,
    base_url: &str,
) -> Result<rig::providers::xai::Client> {
    Ok(rig::providers::xai::Client::builder()
        .api_key(api_key.unwrap_or_default())
        .base_url(base_url)
        .build()?)
}

fn build_anthropic_client(req: &ProviderRequest) -> Result<rig::providers::anthropic::Client> {
    build_anthropic_client_from_parts(req.api_key.as_deref(), &req.base_url)
}

fn build_anthropic_client_from_context(
    ctx: &crate::gateway::ModelContext,
) -> Result<rig::providers::anthropic::Client> {
    build_anthropic_client_from_parts(ctx.api_key.as_deref(), &ctx.base_url)
}

fn build_anthropic_client_from_parts(
    api_key: Option<&str>,
    base_url: &str,
) -> Result<rig::providers::anthropic::Client> {
    let normalized = normalize_base_url("anthropic", base_url);
    Ok(rig::providers::anthropic::Client::builder()
        .api_key(api_key.unwrap_or_default())
        .base_url(&normalized)
        .build()?)
}

fn build_gemini_client(req: &ProviderRequest) -> Result<rig::providers::gemini::Client> {
    build_gemini_client_from_parts(req.api_key.as_deref(), &req.base_url)
}

fn build_gemini_client_from_context(
    ctx: &crate::gateway::ModelContext,
) -> Result<rig::providers::gemini::Client> {
    build_gemini_client_from_parts(ctx.api_key.as_deref(), &ctx.base_url)
}

fn build_gemini_client_from_parts(
    api_key: Option<&str>,
    base_url: &str,
) -> Result<rig::providers::gemini::Client> {
    let normalized = normalize_base_url("google", base_url);
    Ok(rig::providers::gemini::Client::builder()
        .api_key(api_key.unwrap_or_default())
        .base_url(&normalized)
        .build()?)
}

fn build_ollama_client(req: &ProviderRequest) -> Result<rig::providers::ollama::Client> {
    build_ollama_client_from_parts(&req.base_url)
}

fn build_ollama_client_from_context(
    ctx: &crate::gateway::ModelContext,
) -> Result<rig::providers::ollama::Client> {
    build_ollama_client_from_parts(&ctx.base_url)
}

fn build_ollama_client_from_parts(base_url: &str) -> Result<rig::providers::ollama::Client> {
    let normalized = base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();
    Ok(rig::providers::ollama::Client::builder()
        .api_key(rig::client::Nothing)
        .base_url(&normalized)
        .build()?)
}

async fn execute_with_model<M>(
    model: M,
    request: CompletionRequest,
    writer: Option<&mut dyn TransportWriter>,
) -> Result<ModelResponse>
where
    M: CompletionModel + 'static,
    M::StreamingResponse: Clone + Unpin + rig::completion::GetTokenUsage + 'static,
{
    match writer {
        Some(writer) => execute_streaming(model, request, writer).await,
        None => execute_non_streaming(model, request).await,
    }
}

async fn execute_non_streaming<M>(model: M, request: CompletionRequest) -> Result<ModelResponse>
where
    M: CompletionModel + 'static,
    M::StreamingResponse: Clone + Unpin + rig::completion::GetTokenUsage + 'static,
{
    let response = model.completion(request).await?;
    Ok(model_response_from_choice(
        response.choice.iter().cloned().collect(),
        Some(response.usage),
    ))
}

async fn execute_streaming<M>(
    model: M,
    request: CompletionRequest,
    writer: &mut dyn TransportWriter,
) -> Result<ModelResponse>
where
    M: CompletionModel + 'static,
    M::StreamingResponse: Clone + Unpin + rig::completion::GetTokenUsage + 'static,
{
    let mut stream = model.stream(request).await?;
    let mut result = ModelResponse::default();
    let mut partial_calls: HashMap<String, PartialToolCall> = HashMap::new();
    let mut thinking_started = false;
    let mut thinking_summary = String::new();

    while let Some(item) = stream.next().await {
        match item? {
            StreamedAssistantContent::Text(text) => {
                result.text.push_str(text.text());
                legacy::send_chunk(writer, text.text()).await?;
            }
            StreamedAssistantContent::Reasoning(reasoning) => {
                let text = reasoning.display_text();
                if !text.is_empty() {
                    legacy::send_thinking_start(writer).await?;
                    legacy::send_thinking_delta(writer, &text).await?;
                    legacy::send_thinking_end(writer, summarize_reasoning(Some(&text))).await?;
                }
            }
            StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                if !thinking_started {
                    legacy::send_thinking_start(writer).await?;
                    thinking_started = true;
                }
                thinking_summary.push_str(&reasoning);
                legacy::send_thinking_delta(writer, &reasoning).await?;
            }
            StreamedAssistantContent::ToolCall {
                tool_call,
                internal_call_id,
            } => {
                result.tool_calls.push(parsed_tool_call(tool_call));
                partial_calls.remove(&internal_call_id);
            }
            StreamedAssistantContent::ToolCallDelta {
                id,
                internal_call_id,
                content,
            } => {
                let entry =
                    partial_calls
                        .entry(internal_call_id)
                        .or_insert_with(|| PartialToolCall {
                            id,
                            name: String::new(),
                            arguments: String::new(),
                        });
                match content {
                    ToolCallDeltaContent::Name(name) => entry.name = name,
                    ToolCallDeltaContent::Delta(delta) => entry.arguments.push_str(&delta),
                }
            }
            StreamedAssistantContent::Final(response) => {
                if let Some(usage) = response.token_usage() {
                    result.prompt_tokens = Some(usage.input_tokens);
                    result.completion_tokens = Some(usage.output_tokens);
                }
            }
        }
    }

    if thinking_started {
        legacy::send_thinking_end(writer, summarize_reasoning(Some(&thinking_summary))).await?;
    }

    for partial in partial_calls.into_values() {
        result.tool_calls.push(ParsedToolCall {
            id: partial.id,
            name: partial.name,
            arguments: serde_json::from_str(&partial.arguments).unwrap_or_else(|_| json!({})),
        });
    }

    result.finish_reason = Some(normalize_finish_reason(result.tool_calls.is_empty()).to_string());
    Ok(result)
}

fn model_response_from_choice(
    choice: Vec<AssistantContent>,
    usage: Option<rig::completion::Usage>,
) -> ModelResponse {
    let mut response = ModelResponse::default();

    for item in choice {
        match item {
            AssistantContent::Text(text) => response.text.push_str(text.text()),
            AssistantContent::ToolCall(tool_call) => {
                response.tool_calls.push(parsed_tool_call(tool_call))
            }
            AssistantContent::Reasoning(_) | AssistantContent::Image(_) => {}
        }
    }

    if let Some(usage) = usage {
        response.prompt_tokens = Some(usage.input_tokens);
        response.completion_tokens = Some(usage.output_tokens);
    }

    response.finish_reason =
        Some(normalize_finish_reason(response.tool_calls.is_empty()).to_string());
    response
}

fn parsed_tool_call(tool_call: ToolCall) -> ParsedToolCall {
    ParsedToolCall {
        id: tool_call.id,
        name: tool_call.function.name,
        arguments: tool_call.function.arguments,
    }
}

fn normalize_finish_reason(no_tool_calls: bool) -> &'static str {
    if no_tool_calls { "stop" } else { "tool_calls" }
}

fn summarize_reasoning(reasoning: Option<&str>) -> Option<&str> {
    reasoning.filter(|text| !text.is_empty()).map(|text| {
        let end = text
            .char_indices()
            .nth(100)
            .map(|(idx, _)| idx)
            .unwrap_or(text.len());
        &text[..end]
    })
}

struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

async fn verify_client<C>(client: Result<C>) -> ProbeResult
where
    C: VerifyClient,
{
    let client = match client {
        Ok(client) => client,
        Err(err) => {
            return ProbeResult::Unreachable {
                detail: err.to_string(),
            };
        }
    };

    match client.verify().await {
        Ok(()) => ProbeResult::Ready,
        Err(rig::client::VerifyError::InvalidAuthentication) => ProbeResult::AuthError {
            detail: "invalid authentication".to_string(),
        },
        Err(rig::client::VerifyError::ProviderError(detail)) => {
            ProbeResult::Connected { warning: detail }
        }
        Err(rig::client::VerifyError::HttpError(err)) => ProbeResult::Unreachable {
            detail: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_openai_tool_payloads_to_rig_messages() {
        let message = ChatMessage::text(
            "assistant",
            r#"{"role":"assistant","content":"Checking","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"/tmp/demo\"}"}}]}"#,
        );

        let converted = chat_message_to_rig_message(&message).unwrap();
        match converted {
            Message::Assistant { content, .. } => {
                let items: Vec<_> = content.iter().collect();
                assert_eq!(items.len(), 2);
            }
            _ => panic!("expected assistant message"),
        }
    }

    #[test]
    fn model_response_extracts_text_and_tool_calls() {
        let response = model_response_from_choice(
            vec![
                AssistantContent::Text(Text {
                    text: "hello".to_string(),
                }),
                AssistantContent::ToolCall(ToolCall {
                    id: "call_1".to_string(),
                    call_id: None,
                    function: ToolFunction {
                        name: "read_file".to_string(),
                        arguments: json!({"path": "/tmp/demo"}),
                    },
                    signature: None,
                    additional_params: None,
                }),
            ],
            None,
        );

        assert_eq!(response.text, "hello");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn finish_reason_normalization_prefers_stop_without_tools() {
        assert_eq!(normalize_finish_reason(true), "stop");
        assert_eq!(normalize_finish_reason(false), "tool_calls");
    }
}

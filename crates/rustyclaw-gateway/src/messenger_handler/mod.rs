//! Messenger integration for the gateway.
//!
//! This module provides the messenger polling loop that receives messages
//! from configured messengers (Telegram, Discord, Matrix, etc.) and routes
//! them through the model for processing with full tool loop support.

use anyhow::Result;
use rustyclaw_core::config::Config;
use rustyclaw_core::messengers::{Message, Messenger, MessengerManager, SendOptions};
use rustyclaw_core::tools;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use crate::providers;
use crate::secrets_handler;
use crate::skills_handler;
use crate::{SharedSkillManager, SharedVault};
use rustyclaw_core::gateway::{
    ChatMessage, MediaRef, ModelContext, ProviderRequest, ToolCallResult,
};

mod builders;
mod media;
mod prompt;

use builders::create_messenger;
use media::process_attachments;
use prompt::build_messenger_system_prompt;

/// Shared messenger manager for the gateway.
pub type SharedMessengerManager = Arc<Mutex<MessengerManager>>;

/// Conversation history storage per chat.
/// Key: "messenger_type:chat_id" or "messenger_type:sender_id"
type ConversationStore = Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>;

/// Maximum messages to keep in conversation history per chat.
const MAX_HISTORY_MESSAGES: usize = 50;

/// Maximum tool loop rounds.
const MAX_TOOL_ROUNDS: usize = 25;

/// Create a messenger manager from config.
pub async fn create_messenger_manager(config: &Config) -> Result<MessengerManager> {
    let mut manager = MessengerManager::new();

    for messenger_config in &config.messengers {
        if !messenger_config.enabled {
            continue;
        }
        match create_messenger(messenger_config).await {
            Ok(messenger) => {
                info!(
                    name = %messenger.name(),
                    messenger_type = %messenger.messenger_type(),
                    "Messenger initialized"
                );
                manager = manager.add_boxed(messenger);
            }
            Err(e) => {
                error!(
                    messenger_type = %messenger_config.messenger_type,
                    error = %e,
                    "Failed to initialize messenger"
                );
            }
        }
    }

    Ok(manager)
}

fn get_messenger_by_type<'a>(
    mgr: &'a MessengerManager,
    messenger_type: &str,
) -> Option<&'a dyn Messenger> {
    mgr.messengers()
        .iter()
        .find(|messenger| messenger.messenger_type() == messenger_type)
        .map(|messenger| messenger.as_ref())
}

/// Run the messenger polling loop.
///
/// This polls all configured messengers for incoming messages and routes
/// them through the model for processing with full tool support.
///
/// When `messenger_max_concurrent` > 1, messages are processed concurrently.
/// The loop continues polling for new messages while processing tasks run.
pub async fn run_messenger_loop(
    config: Config,
    messenger_mgr: SharedMessengerManager,
    model_ctx: Option<Arc<ModelContext>>,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    task_mgr: super::SharedTaskManager,
    model_registry: super::SharedModelRegistry,
    copilot_session: Option<Arc<super::CopilotSession>>,
    cancel: CancellationToken,
) -> Result<()> {
    eprintln!("DEBUG: run_messenger_loop() called");
    // If no model context, we can't process messages
    let model_ctx = match model_ctx {
        Some(ctx) => ctx,
        None => {
            eprintln!("DEBUG: No model context, returning early");
            warn!("No model context — messenger loop disabled");
            return Ok(());
        }
    };

    let poll_interval =
        Duration::from_millis(config.messenger_poll_interval_ms.unwrap_or(2000).max(500) as u64);

    // Concurrent processing setup
    let max_concurrent = config.messenger_max_concurrent.unwrap_or(1);
    let concurrent_mode = max_concurrent > 1;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

    eprintln!(
        "DEBUG: messenger_max_concurrent={}, concurrent_mode={}",
        max_concurrent, concurrent_mode
    );
    if concurrent_mode {
        info!(max_concurrent, "Concurrent message processing enabled");
    }

    // Per-chat conversation history
    let conversations: ConversationStore = Arc::new(Mutex::new(HashMap::new()));

    let http = Arc::new(reqwest::Client::new());

    eprintln!(
        "DEBUG: Starting messenger loop with poll_interval={}ms",
        poll_interval.as_millis()
    );
    info!(
        poll_interval_ms = poll_interval.as_millis(),
        "Starting messenger loop"
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                eprintln!("DEBUG: Messenger loop cancelled");
                info!("Shutting down messenger loop");
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {
                eprintln!("DEBUG: Polling messengers...");
                // Poll all messengers for incoming messages
                let messages = {
                    let mgr = messenger_mgr.lock().await;
                    poll_all_messengers(&mgr).await
                };
                eprintln!("DEBUG: Got {} messages", messages.len());

                // Process each message
                for (messenger_type, msg) in messages {
                    eprintln!("DEBUG: Processing message from {} in {}", msg.sender, messenger_type);

                    if concurrent_mode {
                        // Spawn message processing as a background task
                        let http = Arc::clone(&http);
                        let config = config.clone();
                        let messenger_mgr = Arc::clone(&messenger_mgr);
                        let model_ctx = Arc::clone(&model_ctx);
                        let vault = Arc::clone(&vault);
                        let skill_mgr = Arc::clone(&skill_mgr);
                        let task_mgr = Arc::clone(&task_mgr);
                        let model_registry = Arc::clone(&model_registry);
                        let copilot_session = copilot_session.clone();
                        let conversations = Arc::clone(&conversations);
                        let semaphore = Arc::clone(&semaphore);
                        let messenger_type = messenger_type.clone();

                        tokio::spawn(async move {
                            // Acquire permit (blocks if at capacity)
                            let _permit = semaphore.acquire().await;

                            // Set typing indicator
                            if let Some(channel) = &msg.channel {
                                let mgr = messenger_mgr.lock().await;
                                if let Some(messenger) = get_messenger_by_type(&mgr, &messenger_type) {
                                    let _ = messenger.set_typing(channel, true).await;
                                }
                            }

                            let channel_for_typing = msg.channel.clone();
                            let result = process_incoming_message(
                                &http,
                                &config,
                                &messenger_mgr,
                                &model_ctx,
                                &vault,
                                &skill_mgr,
                                &task_mgr,
                                &model_registry,
                                copilot_session.as_deref(),
                                &conversations,
                                &messenger_type,
                                msg,
                            )
                            .await;

                            // Clear typing indicator
                            if let Some(channel) = channel_for_typing {
                                let mgr = messenger_mgr.lock().await;
                                if let Some(messenger) = get_messenger_by_type(&mgr, &messenger_type) {
                                    let _ = messenger.set_typing(&channel, false).await;
                                }
                            }

                            if let Err(e) = result {
                                eprintln!("DEBUG: Error processing message: {}", e);
                                error!(error = %e, "Error processing message");
                            }
                        });
                        eprintln!("DEBUG: Message processing spawned (concurrent)");
                    } else {
                        // Sequential mode (original behavior)
                        // Set typing indicator before processing
                        if let Some(channel) = &msg.channel {
                            let mgr = messenger_mgr.lock().await;
                            if let Some(messenger) = get_messenger_by_type(&mgr, &messenger_type) {
                                let _ = messenger.set_typing(channel, true).await;
                            }
                        }

                        let channel_for_typing = msg.channel.clone();
                        let result = process_incoming_message(
                            &http,
                            &config,
                            &messenger_mgr,
                            &model_ctx,
                            &vault,
                            &skill_mgr,
                            &task_mgr,
                            &model_registry,
                            copilot_session.as_deref(),
                            &conversations,
                            &messenger_type,
                            msg,
                        )
                        .await;

                        // Clear typing indicator after processing
                        if let Some(channel) = channel_for_typing {
                            let mgr = messenger_mgr.lock().await;
                            if let Some(messenger) = get_messenger_by_type(&mgr, &messenger_type) {
                                let _ = messenger.set_typing(&channel, false).await;
                            }
                        }

                        if let Err(e) = result {
                            eprintln!("DEBUG: Error processing message: {}", e);
                            error!(error = %e, "Error processing message");
                        }
                        eprintln!("DEBUG: Message processing complete");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Poll all messengers and collect incoming messages.
async fn poll_all_messengers(mgr: &MessengerManager) -> Vec<(String, Message)> {
    let mut all_messages = Vec::new();

    for messenger in mgr.messengers() {
        match messenger.receive_messages().await {
            Ok(messages) => {
                for msg in messages {
                    all_messages.push((messenger.messenger_type().to_string(), msg));
                }
            }
            Err(e) => {
                debug!(
                    messenger_type = %messenger.messenger_type(),
                    error = %e,
                    "Error polling messenger"
                );
            }
        }
    }

    all_messages
}

/// Process an incoming message through the model with full tool loop.
async fn process_incoming_message(
    http: &reqwest::Client,
    config: &Config,
    messenger_mgr: &SharedMessengerManager,
    model_ctx: &Arc<ModelContext>,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    task_mgr: &super::SharedTaskManager,
    model_registry: &super::SharedModelRegistry,
    copilot_session: Option<&super::CopilotSession>,
    conversations: &ConversationStore,
    messenger_type: &str,
    msg: Message,
) -> Result<()> {
    debug!(
        sender = %msg.sender,
        messenger_type = %messenger_type,
        content_preview = %if msg.content.len() > 50 {
            format!("{}...", &msg.content[..50])
        } else {
            msg.content.clone()
        },
        "Received message"
    );

    let workspace_dir = config.workspace_dir();

    // Build conversation key for this chat
    let conv_key = format!(
        "{}:{}",
        messenger_type,
        msg.channel.as_deref().unwrap_or(&msg.sender)
    );

    // Get or create conversation history
    let mut messages = {
        let mut store = conversations.lock().await;
        store
            .entry(conv_key.clone())
            .or_insert_with(Vec::new)
            .clone()
    };

    // Build system prompt (async to include task and model context)
    let system_prompt = build_messenger_system_prompt(
        config,
        messenger_type,
        &msg,
        task_mgr,
        model_registry,
        skill_mgr,
        &conv_key,
    )
    .await;

    // Add system message if not present
    if messages.is_empty() || messages[0].role != "system" {
        messages.insert(0, ChatMessage::text("system", &system_prompt));
    } else {
        // Update system prompt
        messages[0].content = system_prompt.clone();
    }

    // Media cache directory
    let cache_dir = config.credentials_dir().join("media_cache");

    // Process any image attachments
    let images = if let Some(attachments) = &msg.media {
        process_attachments(http, attachments, &cache_dir).await
    } else {
        Vec::new()
    };

    if !images.is_empty() {
        debug!(
            image_count = images.len(),
            "Processing images (vision not yet supported in messenger handler)"
        );
    }

    // Build media refs for history storage
    let media_refs: Vec<MediaRef> = images.iter().map(|img| img.media_ref.clone()).collect();

    // Add user message to history (with media refs, not raw data)
    messages.push(ChatMessage::user_with_media(
        &msg.content,
        media_refs.clone(),
    ));

    // Resolve effective bearer token (handles Copilot session exchange)
    let effective_key = crate::auth::resolve_bearer_token(
        http,
        &model_ctx.provider,
        model_ctx.api_key.as_deref(),
        copilot_session,
    )
    .await
    .ok()
    .flatten();

    // Build request - ProviderRequest expects Vec<ChatMessage>
    let mut resolved = ProviderRequest {
        provider: model_ctx.provider.clone(),
        model: model_ctx.model.clone(),
        base_url: model_ctx.base_url.clone(),
        api_key: effective_key,
        messages: messages.clone(),
    };

    // Run the agentic tool loop
    let mut final_response = String::new();

    for _round in 0..MAX_TOOL_ROUNDS {
        let result = if resolved.provider == "anthropic" {
            providers::call_anthropic_with_tools(http, &resolved, None).await
        } else if resolved.provider == "google" {
            providers::call_google_with_tools(http, &resolved).await
        } else {
            providers::call_openai_with_tools(http, &resolved, None).await
        };

        let model_resp = match result {
            Ok(r) => r,
            Err(err) => {
                error!(error = %err, "Model error");
                return Err(err);
            }
        };

        // Collect text response
        if !model_resp.text.is_empty() {
            final_response.push_str(&model_resp.text);
        }

        if model_resp.tool_calls.is_empty() {
            // No tool calls — done
            break;
        }

        // Execute each requested tool
        let mut tool_results: Vec<ToolCallResult> = Vec::new();

        for tc in &model_resp.tool_calls {
            debug!(tool_name = %tc.name, tool_id = %tc.id, "Executing tool call");

            let (output, is_error) = if tools::is_secrets_tool(&tc.name) {
                match secrets_handler::execute_secrets_tool(&tc.name, &tc.arguments, vault).await {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if tools::is_skill_tool(&tc.name) {
                match skills_handler::execute_skill_tool(&tc.name, &tc.arguments, skill_mgr).await {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if crate::mcp_handler::is_mcp_tool(&tc.name) {
                #[cfg(feature = "mcp")]
                {
                    // MCP tools require the MCP manager - for now, return an error
                    // TODO: Pass mcp_mgr to this function
                    (
                        format!(
                            "MCP tool '{}' called but MCP manager not available in this context",
                            tc.name
                        ),
                        true,
                    )
                }
                #[cfg(not(feature = "mcp"))]
                {
                    (
                        format!("MCP tool '{}' requires the 'mcp' feature", tc.name),
                        true,
                    )
                }
            } else if super::canvas_handler::is_canvas_tool(&tc.name) {
                // Canvas tools require the canvas host - for now, return an error
                // TODO: Pass canvas_host to this function
                (
                    format!(
                        "Canvas tool '{}' called but canvas host not available in this context",
                        tc.name
                    ),
                    true,
                )
            } else if crate::task_handler::is_task_tool(&tc.name) {
                // Execute task tool with task manager
                match crate::task_handler::execute_task_tool(
                    &tc.name,
                    &tc.arguments,
                    task_mgr,
                    Some(&conv_key),
                )
                .await
                {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if crate::command_wrapper::should_wrap_in_task(&tc.name) {
                // Wrap execute_command in a Task
                let task_id =
                    crate::command_wrapper::start_command_task(task_mgr, &tc.arguments, &conv_key)
                        .await;

                let result = tools::execute_tool(&tc.name, &tc.arguments, &workspace_dir).await;

                match result {
                    Ok(output) => {
                        // Check if it was backgrounded
                        if let Some(session_id) = crate::command_wrapper::parse_session_id(&output)
                        {
                            crate::command_wrapper::update_command_task_session(
                                task_mgr,
                                task_id,
                                &session_id,
                            )
                            .await;
                        } else {
                            crate::command_wrapper::complete_command_task(
                                task_mgr, task_id, &output,
                            )
                            .await;
                        }
                        (output, false)
                    }
                    Err(err) => {
                        crate::command_wrapper::fail_command_task(task_mgr, task_id, &err).await;
                        (err, true)
                    }
                }
            } else if crate::model_handler::is_model_tool(&tc.name) {
                // Model management tools
                match crate::model_handler::execute_model_tool(
                    &tc.name,
                    &tc.arguments,
                    model_registry,
                )
                .await
                {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else {
                match tools::execute_tool(&tc.name, &tc.arguments, &workspace_dir).await {
                    Ok(text) => (text, false),
                    Err(err) => (err, true),
                }
            };

            trace!(
                tool_name = %tc.name,
                is_error = is_error,
                output_preview = %if output.len() > 100 {
                    format!("{}...", &output[..100])
                } else {
                    output.clone()
                },
                "Tool result"
            );

            tool_results.push(ToolCallResult {
                id: tc.id.clone(),
                name: tc.name.clone(),
                output,
                is_error,
            });
        }

        // Append tool round to conversation
        providers::append_tool_round(
            &resolved.provider,
            &mut resolved.messages,
            &model_resp,
            &tool_results,
        );
    }

    // Update conversation history
    {
        let mut store = conversations.lock().await;
        let history = store.entry(conv_key).or_insert_with(Vec::new);

        // Add user message (with media refs)
        history.push(ChatMessage::user_with_media(
            &msg.content,
            media_refs.clone(),
        ));

        // Add assistant response
        if !final_response.is_empty() {
            history.push(ChatMessage::text("assistant", &final_response));
        }

        // Trim history if too long (keep system message)
        while history.len() > MAX_HISTORY_MESSAGES {
            if history.len() > 1 && history[1].role != "system" {
                history.remove(1);
            } else {
                break;
            }
        }
    }

    // Send response back via messenger
    if !final_response.is_empty()
        && final_response.trim() != "NO_REPLY"
        && final_response.trim() != "HEARTBEAT_OK"
    {
        let mgr = messenger_mgr.lock().await;
        if let Some(messenger) = get_messenger_by_type(&mgr, messenger_type) {
            let recipient = msg.channel.as_deref().unwrap_or(&msg.sender);

            let opts = SendOptions {
                recipient,
                content: &final_response,
                reply_to: Some(&msg.id),
                thread_id: None,
                silent: false,
                media: None,
            };

            match messenger.send_message_with_options(opts).await {
                Ok(msg_id) => {
                    debug!(
                        message_id = %msg_id,
                        response_preview = %if final_response.len() > 50 {
                            format!("{}...", &final_response[..50])
                        } else {
                            final_response.clone()
                        },
                        "Sent response"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Failed to send response");
                }
            }
        }
    }

    Ok(())
}

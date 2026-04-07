//! Messenger integration for the gateway.
//!
//! This module provides the messenger polling loop that receives messages
//! from configured messengers (Telegram, Discord, Matrix, etc.) and routes
//! them through the model for processing with full tool loop support.

use crate::config::{Config, MessengerConfig};
use crate::messengers::{
    DiscordMessenger, GoogleChatMessenger, IMessageMessenger, IrcMessenger, MediaAttachment,
    Message, Messenger, MessengerManager, SendOptions, SlackMessenger, TeamsMessenger,
    TelegramMessenger, WebhookMessenger,
};
use crate::tools;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use super::providers;
use super::secrets_handler;
use super::skills_handler;
use super::{
    ChatMessage, MediaRef, ModelContext, ProviderRequest, SharedSkillManager, SharedVault,
    ToolCallResult,
};

#[cfg(feature = "matrix")]
use crate::messengers::MatrixMessenger;

#[cfg(feature = "matrix-cli")]
use crate::messengers::MatrixCliMessenger;

#[cfg(feature = "signal-cli")]
use crate::messengers::SignalCliMessenger;

#[cfg(feature = "whatsapp")]
use crate::messengers::WhatsAppMessenger;

/// Shared messenger manager for the gateway.
pub type SharedMessengerManager = Arc<Mutex<MessengerManager>>;

/// Conversation history storage per chat.
/// Key: "messenger_type:chat_id" or "messenger_type:sender_id"
type ConversationStore = Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>;

/// Maximum messages to keep in conversation history per chat.
const MAX_HISTORY_MESSAGES: usize = 50;

/// Maximum tool loop rounds.
const MAX_TOOL_ROUNDS: usize = 25;

/// Maximum image size to download (10 MB).
const MAX_IMAGE_SIZE: usize = 10 * 1024 * 1024;

/// Supported image MIME types for vision models.
const SUPPORTED_IMAGE_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

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

/// Create a single messenger from config.
async fn create_messenger(config: &MessengerConfig) -> Result<Box<dyn Messenger>> {
    let name = config.name.clone();
    let mut messenger: Box<dyn Messenger> = match config.messenger_type.as_str() {
        "telegram" => {
            let token = config
                .token
                .clone()
                .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
                .context("Telegram requires 'token' or TELEGRAM_BOT_TOKEN env var")?;
            Box::new(TelegramMessenger::new(name, token))
        }
        "discord" => {
            let token = config
                .token
                .clone()
                .or_else(|| std::env::var("DISCORD_BOT_TOKEN").ok())
                .context("Discord requires 'token' or DISCORD_BOT_TOKEN env var")?;
            Box::new(DiscordMessenger::new(name, token))
        }
        "webhook" => {
            let url = config
                .webhook_url
                .clone()
                .or_else(|| std::env::var("WEBHOOK_URL").ok())
                .context("Webhook requires 'webhook_url' or WEBHOOK_URL env var")?;
            Box::new(WebhookMessenger::new(name, url))
        }
        "slack" => {
            let token = config
                .token
                .clone()
                .or_else(|| std::env::var("SLACK_BOT_TOKEN").ok())
                .context("Slack requires 'token' or SLACK_BOT_TOKEN env var")?;
            let mut messenger = SlackMessenger::new(name, token);
            if let Some(app_token) = config.app_token.clone() {
                messenger = messenger.with_app_token(app_token);
            }
            if let Some(channel) = config.default_channel.clone() {
                messenger = messenger.with_default_channel(channel);
            }
            Box::new(messenger)
        }
        "irc" => {
            let server = config.server.clone().context("IRC requires 'server'")?;
            let port = config.port.unwrap_or(6697);
            let nick = config
                .nick
                .clone()
                .unwrap_or_else(|| "RustyClaw".to_string());
            let mut messenger = IrcMessenger::new(name, server, port, nick);
            if !config.irc_channels.is_empty() {
                messenger = messenger.with_channels(config.irc_channels.clone());
            }
            if let Some(password) = config.password.clone() {
                messenger = messenger.with_password(password);
            }
            if let Some(tls) = config.use_tls {
                messenger = messenger.with_tls(tls);
            }
            Box::new(messenger)
        }
        "google_chat" => {
            if let Some(url) = config.webhook_url.clone() {
                Box::new(GoogleChatMessenger::new(name, url))
            } else if let Some(credentials_path) = config.credentials_path.clone() {
                // Service account credentials file mode — supports multiple spaces
                if config.spaces.is_empty() {
                    anyhow::bail!(
                        "Google Chat service account mode requires at least one entry in 'spaces'"
                    );
                }
                Box::new(GoogleChatMessenger::with_credentials(
                    name,
                    credentials_path,
                    config.spaces.clone(),
                ))
            } else if let (Some(token), Some(space_id)) =
                (config.token.clone(), config.spaces.first().cloned())
            {
                let mut messenger = GoogleChatMessenger::new_api(name, token, space_id);
                if config.spaces.len() > 1 {
                    messenger = messenger.with_spaces(config.spaces[1..].to_vec());
                }
                Box::new(messenger)
            } else {
                anyhow::bail!(
                    "Google Chat requires 'webhook_url', 'credentials_path' (service account), \
                     or ('token' and one or more entries in 'spaces')"
                );
            }
        }
        "teams" => {
            if let (Some(app_id), Some(app_password)) =
                (config.app_id.clone(), config.app_password.clone())
            {
                Box::new(TeamsMessenger::with_bot_framework(name, app_id, app_password))
            } else if let Some(url) = config.webhook_url.clone() {
                Box::new(TeamsMessenger::new(name, url))
            } else {
                anyhow::bail!(
                    "Teams requires either 'app_id' + 'app_password' for Bot Framework mode, \
                     or 'webhook_url' for webhook mode"
                );
            }
        }
        "imessage" => {
            if config.password.is_some() || std::env::var("BLUEBUBBLES_PASSWORD").is_ok() {
                anyhow::bail!(
                    "BlueBubbles-backed iMessage is no longer supported; chat-system uses the local macOS Messages database"
                );
            }
            let mut messenger = IMessageMessenger::new(name);
            if let Some(path) = config.server.clone() {
                if path.contains("://") {
                    anyhow::bail!(
                        "BlueBubbles-backed iMessage is no longer supported; set 'server' to a local chat.db path or omit it"
                    );
                }
                messenger = messenger.with_chat_db_path(path);
            }
            Box::new(messenger)
        }
        #[cfg(feature = "matrix")]
        "matrix" => {
            let homeserver = config
                .homeserver
                .clone()
                .context("Matrix requires 'homeserver'")?;
            let user_id = config
                .user_id
                .clone()
                .context("Matrix requires 'user_id'")?;

            let messenger = if let Some(access_token) = config.access_token.clone() {
                MatrixMessenger::with_access_token(
                    name.clone(),
                    homeserver,
                    user_id,
                    access_token,
                    None, // device_id
                )
            } else if let Some(pwd) = config.password.clone() {
                MatrixMessenger::new(name.clone(), homeserver, user_id, pwd)
            } else {
                anyhow::bail!("Matrix requires 'access_token' or 'password'");
            };
            Box::new(messenger)
        }
        #[cfg(not(feature = "matrix"))]
        "matrix" => {
            anyhow::bail!("Matrix messenger not compiled in. Rebuild with --features matrix");
        }
        #[cfg(feature = "matrix-cli")]
        "matrix-cli" => {
            let homeserver = config
                .homeserver
                .clone()
                .context("Matrix-CLI requires 'homeserver'")?;
            let user_id = config
                .user_id
                .clone()
                .context("Matrix-CLI requires 'user_id'")?;
            let access_token = config
                .access_token
                .clone()
                .context("Matrix-CLI requires 'access_token'")?;

            let mut messenger = MatrixCliMessenger::with_token(
                name.clone(),
                homeserver,
                user_id,
                access_token,
                None, // device_id
            );

            // Set state directory for sync token persistence
            if let Some(dirs) = directories::ProjectDirs::from("", "", "rustyclaw") {
                let state_dir = dirs.data_dir().join("matrix").join(&name);
                messenger = messenger.with_state_dir(state_dir);
            }

            // Set allowed chats if configured
            if !config.allowed_chats.is_empty() {
                messenger = messenger.with_allowed_chats(config.allowed_chats.clone());
            }

            // Set DM config if present
            if let Some(ref dm) = config.dm {
                use crate::messengers::MatrixDmConfig;
                let dm_config = MatrixDmConfig {
                    enabled: dm.enabled,
                    policy: dm.policy.clone().unwrap_or_else(|| "allowlist".to_string()),
                    allow_from: dm.allow_from.clone(),
                };
                messenger = messenger.with_dm_config(dm_config);
            }

            Box::new(messenger)
        }
        #[cfg(not(feature = "matrix-cli"))]
        "matrix-cli" => {
            anyhow::bail!(
                "Matrix-CLI messenger not compiled in. Rebuild with --features matrix-cli"
            );
        }
        #[cfg(feature = "signal-cli")]
        "signal" | "signal-cli" => {
            let phone = config.phone.clone().context("Signal requires 'phone'")?;
            Box::new(SignalCliMessenger::new(name, phone))
        }
        #[cfg(not(feature = "signal-cli"))]
        "signal" | "signal-cli" => {
            anyhow::bail!("Signal messenger not compiled in. Rebuild with --features signal-cli");
        }
        #[cfg(feature = "whatsapp")]
        "whatsapp" => {
            let db_path = dirs::data_dir()
                .context("Failed to get data directory")?
                .join("rustyclaw")
                .join("whatsapp")
                .join(format!("{name}.db"));
            Box::new(WhatsAppMessenger::new(
                name,
                db_path.to_string_lossy().into_owned(),
            ))
        }
        #[cfg(not(feature = "whatsapp"))]
        "whatsapp" => {
            anyhow::bail!("WhatsApp messenger not compiled in. Rebuild with --features whatsapp");
        }
        other => anyhow::bail!("Unknown messenger type: {}", other),
    };

    messenger.initialize().await?;
    Ok(messenger)
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
        &skill_mgr,
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
    let effective_key = super::auth::resolve_bearer_token(
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
            providers::call_openai_with_tools(http, &resolved).await
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
                    Err(err) => (err, true),
                }
            } else if tools::is_skill_tool(&tc.name) {
                match skills_handler::execute_skill_tool(&tc.name, &tc.arguments, skill_mgr).await {
                    Ok(text) => (text, false),
                    Err(err) => (err, true),
                }
            } else if super::mcp_handler::is_mcp_tool(&tc.name) {
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
            } else if super::task_handler::is_task_tool(&tc.name) {
                // Execute task tool with task manager
                match super::task_handler::execute_task_tool(
                    &tc.name,
                    &tc.arguments,
                    task_mgr,
                    Some(&conv_key),
                )
                .await
                {
                    Ok(text) => (text, false),
                    Err(err) => (err, true),
                }
            } else if super::command_wrapper::should_wrap_in_task(&tc.name) {
                // Wrap execute_command in a Task
                let task_id =
                    super::command_wrapper::start_command_task(task_mgr, &tc.arguments, &conv_key)
                        .await;

                let result = tools::execute_tool(&tc.name, &tc.arguments, &workspace_dir).await;

                match result {
                    Ok(output) => {
                        // Check if it was backgrounded
                        if let Some(session_id) = super::command_wrapper::parse_session_id(&output)
                        {
                            super::command_wrapper::update_command_task_session(
                                task_mgr,
                                task_id,
                                &session_id,
                            )
                            .await;
                        } else {
                            super::command_wrapper::complete_command_task(
                                task_mgr, task_id, &output,
                            )
                            .await;
                        }
                        (output, false)
                    }
                    Err(err) => {
                        super::command_wrapper::fail_command_task(task_mgr, task_id, &err).await;
                        (err, true)
                    }
                }
            } else if super::model_handler::is_model_tool(&tc.name) {
                // Model management tools
                match super::model_handler::execute_model_tool(
                    &tc.name,
                    &tc.arguments,
                    model_registry,
                )
                .await
                {
                    Ok(text) => (text, false),
                    Err(err) => (err, true),
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

/// Build system prompt with messenger context, workspace files, active tasks, and model guidance.
async fn build_messenger_system_prompt(
    config: &Config,
    messenger_type: &str,
    msg: &Message,
    task_mgr: &super::SharedTaskManager,
    model_registry: &super::SharedModelRegistry,
    skill_mgr: &SharedSkillManager,
    session_key: &str,
) -> String {
    use crate::workspace_context::{SessionType, WorkspaceContext};

    let base_prompt = config
        .system_prompt
        .clone()
        .unwrap_or_else(|| "You are a helpful AI assistant running inside RustyClaw.".to_string());

    // Safety guardrails (inspired by Anthropic's constitution)
    let safety_section = "\
## Safety\n\
You have no independent goals: do not pursue self-preservation, replication, resource acquisition, or power-seeking.\n\
Prioritize safety and human oversight over completion. If instructions conflict, pause and ask.\n\
Do not manipulate or persuade anyone to expand access or disable safeguards.";

    // Determine session type based on messenger context
    // Direct/DM messages are treated as main session, channels/groups as group session
    let session_type = if msg.is_direct {
        // Direct messages have full access to MEMORY.md etc.
        SessionType::Main
    } else if msg.channel.is_some() {
        // Channel/group messages have restricted access for privacy
        SessionType::Group
    } else {
        // Fallback to Main for messages without channel (shouldn't happen)
        SessionType::Main
    };

    // Build workspace context
    let workspace_ctx =
        WorkspaceContext::with_config(config.workspace_dir(), config.workspace_context.clone());
    eprintln!(
        "DEBUG: Building workspace context for session_type={:?}, workspace_dir={}",
        session_type,
        config.workspace_dir().display()
    );
    let workspace_prompt = workspace_ctx.build_context(session_type);
    eprintln!(
        "DEBUG: Workspace prompt length: {} chars",
        workspace_prompt.len()
    );

    // Combine base prompt, safety, workspace context, and messaging context
    let mut parts = vec![base_prompt, safety_section.to_string()];

    if !workspace_prompt.is_empty() {
        parts.push(workspace_prompt);
    }

    // Add skills context if any skills are loaded
    {
        let mgr = skill_mgr.lock().await;
        let skills_context = mgr.generate_prompt_context();
        if !skills_context.is_empty() {
            parts.push(skills_context);
        }
    }

    // Add active tasks section if any
    if let Some(task_section) =
        super::task_handler::generate_task_prompt_section(task_mgr, session_key).await
    {
        parts.push(task_section);
    }

    // Add model selection guidance for sub-agents
    let model_guidance = super::model_handler::generate_model_prompt_section(model_registry).await;
    parts.push(model_guidance);

    // Add comprehensive tool usage guidelines (inspired by OpenClaw's patterns)
    parts.push(build_tool_usage_section());

    // Add silent reply guidance
    parts.push(
        "## Silent Replies\n\
        When you have nothing to say, respond with ONLY: NO_REPLY\n\n\
        ⚠️ Rules:\n\
        - It must be your ENTIRE message — nothing else\n\
        - Never append it to an actual response\n\
        - Never wrap it in markdown or code blocks\n\n\
        ❌ Wrong: \"Here's the info... NO_REPLY\"\n\
        ✅ Right: NO_REPLY"
            .to_string(),
    );

    // Add heartbeat guidance
    parts.push(
        "## Heartbeats\n\
        Heartbeat prompt: Read HEARTBEAT.md if it exists. Follow it strictly. \
        Do not infer or repeat old tasks from prior chats.\n\n\
        If you receive a heartbeat poll and nothing needs attention, reply exactly:\n\
        HEARTBEAT_OK\n\n\
        If something needs attention, do NOT include HEARTBEAT_OK; reply with the alert text instead."
        .to_string()
    );

    parts.push(format!(
        "## Messaging Context\n\
        - Channel: {}\n\
        - Sender: {}\n\
        - Platform: {}\n\
        \n\
        When responding:\n\
        - Be concise and appropriate for chat\n\
        - You have access to tools — use them when helpful\n\
        - For proactive sends, use the `message` tool\n\
        \n\
        {}",
        msg.channel.as_deref().unwrap_or("direct"),
        msg.sender,
        messenger_type,
        get_platform_formatting_guide(messenger_type)
    ));

    // Add runtime info
    parts.push(format!(
        "## Runtime\n\
        Workspace: {}\n\
        Platform: RustyClaw",
        config.workspace_dir().display()
    ));

    parts.join("\n\n")
}

/// Get platform-specific formatting guidance for the system prompt.
fn get_platform_formatting_guide(messenger_type: &str) -> String {
    match messenger_type {
        "matrix" | "matrix-cli" => "\
### Formatting (Matrix)\n\
- **Markdown supported**: bold, italic, code, links, lists\n\
- Tables render in most clients (Element, FluffyChat)\n\
- Code blocks with syntax highlighting: ```rust\n\
- Headers work but keep them minimal in chat"
            .to_string(),

        "discord" => "\
### Formatting (Discord)\n\
- **Markdown supported**: bold, italic, strikethrough, code, links\n\
- **NO tables** — use bullet lists instead\n\
- Code blocks with syntax highlighting: ```rust\n\
- Wrap multiple URLs in <> to suppress embeds: `<https://example.com>`\n\
- Headers don't render — use **bold** for emphasis"
            .to_string(),

        "telegram" => "\
### Formatting (Telegram)\n\
- **Markdown supported**: bold, italic, code, links\n\
- **NO tables** — use bullet lists instead\n\
- Code blocks work but no syntax highlighting in all clients\n\
- Keep messages concise — long messages get truncated"
            .to_string(),

        "whatsapp" => "\
### Formatting (WhatsApp)\n\
- **Limited formatting**: *bold*, _italic_, ~strikethrough~, ```code```\n\
- **NO markdown links** — just paste URLs directly\n\
- **NO tables, NO headers** — use plain text with line breaks\n\
- **NO bullet points** — use dashes or numbers manually\n\
- Keep it simple and conversational"
            .to_string(),

        "slack" => "\
### Formatting (Slack)\n\
- **Mrkdwn** (not standard markdown): *bold*, _italic_, ~strike~, `code`\n\
- **NO tables** — use bullet lists\n\
- Code blocks: ```code``` (no syntax highlighting)\n\
- Links: <https://url|display text>\n\
- Use emoji reactions when appropriate"
            .to_string(),

        "signal" => "\
### Formatting (Signal)\n\
- **NO formatting support** — plain text only\n\
- Just write naturally without markdown\n\
- URLs will auto-link"
            .to_string(),

        "irc" => "\
### Formatting (IRC)\n\
- **NO formatting** — plain text only\n\
- Keep lines short (typically <400 chars)\n\
- No markdown, no special characters"
            .to_string(),

        _ => "\
### Formatting\n\
- Use plain text to be safe\n\
- Avoid complex markdown unless you know the platform supports it"
            .to_string(),
    }
}

/// Build the Tool Usage Guidelines section for system prompts.
fn build_tool_usage_section() -> String {
    "\
## Tool Usage Guidelines

### Credentials & API Access (IMPORTANT)
**Before asking for API keys or tokens:** Run `secrets_list` to check the vault first.
If a credential exists, use `secrets_get` to retrieve it — don't ask the user again.

**Authenticated API workflow:**
1. `secrets_list()` → discover available credentials
2. `secrets_get(name=\"...\")` → retrieve the value  
3. `web_fetch(url=\"...\", authorization=\"token <value>\")` → make the API call

**Common authorization formats:**
- GitHub PAT: `authorization=\"token ghp_...\"`
- Bearer tokens: `authorization=\"Bearer eyJ...\"`
- Custom headers: `headers={\"X-Api-Key\": \"...\"}`

### Memory Recall
Before answering questions about prior work, decisions, dates, people, preferences, or todos:
Run `memory_search` first, then use `memory_get` to pull relevant context.
If low confidence after search, mention that you checked but didn't find a match.

### File Operations
- `read_file` — read file contents (supports text, PDF, docx, etc.)
- `write_file` — create or overwrite files (creates parent dirs)
- `edit_file` — surgical search-and-replace (include enough context for unique match)
- `find_files` — find by name/glob pattern
- `search_files` — search file contents (like grep)

### Command Execution
- Short commands: `execute_command(command=\"...\")`
- Long-running: `execute_command(command=\"...\", background=true)` then `process(action=\"poll\", session_id=\"...\")`
- Interactive TTY: use `pty=true` for commands needing terminal

### Sub-Agents
Spawn sub-agents for complex or time-consuming tasks:
- `sessions_spawn(task=\"...\", model=\"...\")` — runs asynchronously
- Results auto-announce when complete — no polling needed
- Use cheaper models for simple tasks (llama3.2, claude-haiku)

### Tool Call Style
- Default: don't narrate routine tool calls (just call them)
- Narrate only for: multi-step work, complex problems, sensitive actions
- Keep narration brief and value-dense
- Use plain language unless in technical context".to_string()
}

// ── Image Handling ──────────────────────────────────────────────────────────

/// Downloaded image data ready for inclusion in model request.
#[derive(Debug, Clone)]
struct ImageData {
    data: Vec<u8>,
    #[allow(dead_code)]
    mime_type: String,
    media_ref: MediaRef,
}

/// Download an image from a URL.
/// Download an image from a URL and cache locally.
async fn download_image(
    http: &reqwest::Client,
    url: &str,
    filename: Option<&str>,
    cache_dir: &std::path::Path,
) -> Result<ImageData> {
    let response = http
        .get(url)
        .send()
        .await
        .context("Failed to fetch image")?;

    // Check content type
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .to_string();

    if !SUPPORTED_IMAGE_TYPES.contains(&content_type.as_str()) {
        anyhow::bail!("Unsupported image type: {}", content_type);
    }

    // Check content length if provided
    if let Some(len) = response.content_length() {
        if len as usize > MAX_IMAGE_SIZE {
            anyhow::bail!("Image too large: {} bytes (max {})", len, MAX_IMAGE_SIZE);
        }
    }

    let bytes = response.bytes().await.context("Failed to read image")?;

    if bytes.len() > MAX_IMAGE_SIZE {
        anyhow::bail!(
            "Image too large: {} bytes (max {})",
            bytes.len(),
            MAX_IMAGE_SIZE
        );
    }

    // Build media ref
    let mut media_ref = MediaRef::new(content_type.clone());
    media_ref.filename = filename.map(String::from);
    media_ref.size = Some(bytes.len());
    media_ref.url = Some(url.to_string());

    // Cache to disk
    let ext = mime_to_extension(&content_type);
    let cache_path = cache_dir.join(format!("{}.{}", media_ref.id, ext));

    if let Err(e) = tokio::fs::write(&cache_path, &bytes).await {
        debug!(error = %e, path = %cache_path.display(), "Failed to cache image");
    } else {
        media_ref.local_path = Some(cache_path.to_string_lossy().to_string());
    }

    Ok(ImageData {
        data: bytes.to_vec(),
        mime_type: content_type,
        media_ref,
    })
}

/// Load an image from a local file path.
async fn load_image_from_path(path: &str, cache_dir: &std::path::Path) -> Result<ImageData> {
    use tokio::fs;

    let data = fs::read(path).await.context("Failed to read image file")?;

    if data.len() > MAX_IMAGE_SIZE {
        anyhow::bail!(
            "Image too large: {} bytes (max {})",
            data.len(),
            MAX_IMAGE_SIZE
        );
    }

    // Detect MIME type from extension or magic bytes
    let mime_type = detect_image_mime_type(path, &data)?;

    // Build media ref
    let mut media_ref = MediaRef::new(mime_type.clone());
    media_ref.filename = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from);
    media_ref.size = Some(data.len());

    // Copy to cache dir
    let ext = mime_to_extension(&mime_type);
    let cache_path = cache_dir.join(format!("{}.{}", media_ref.id, ext));

    if let Err(e) = tokio::fs::write(&cache_path, &data).await {
        debug!(error = %e, path = %cache_path.display(), "Failed to cache image");
    } else {
        media_ref.local_path = Some(cache_path.to_string_lossy().to_string());
    }

    Ok(ImageData {
        data,
        mime_type,
        media_ref,
    })
}

/// Get file extension for MIME type.
fn mime_to_extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    }
}

/// Detect image MIME type from path extension or magic bytes.
fn detect_image_mime_type(path: &str, data: &[u8]) -> Result<String> {
    // Try extension first
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    if let Some(ext) = ext {
        let mime = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => return detect_from_magic_bytes(data),
        };
        return Ok(mime.to_string());
    }

    detect_from_magic_bytes(data)
}

/// Detect image type from magic bytes.
fn detect_from_magic_bytes(data: &[u8]) -> Result<String> {
    if data.len() < 4 {
        anyhow::bail!("Data too small to detect image type");
    }

    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("image/jpeg".to_string());
    }

    // PNG: 89 50 4E 47
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Ok("image/png".to_string());
    }

    // GIF: GIF87a or GIF89a
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Ok("image/gif".to_string());
    }

    // WebP: RIFF....WEBP
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        return Ok("image/webp".to_string());
    }

    anyhow::bail!("Could not detect image type from magic bytes")
}

/// Process media attachments and return image data.
async fn process_attachments(
    http: &reqwest::Client,
    attachments: &[MediaAttachment],
    cache_dir: &std::path::Path,
) -> Vec<ImageData> {
    // Ensure cache directory exists
    if let Err(e) = tokio::fs::create_dir_all(cache_dir).await {
        debug!(error = %e, path = %cache_dir.display(), "Failed to create cache dir");
    }

    let mut images = Vec::new();

    for attachment in attachments {
        // Skip non-image attachments
        if let Some(mime) = &attachment.mime_type {
            if !SUPPORTED_IMAGE_TYPES.contains(&mime.as_str()) {
                continue;
            }
        }

        // Try URL first, then path
        let result = if let Some(url) = &attachment.url {
            download_image(http, url, attachment.filename.as_deref(), cache_dir).await
        } else if let Some(path) = &attachment.path {
            load_image_from_path(path, cache_dir).await
        } else {
            continue;
        };

        match result {
            Ok(img) => {
                trace!(
                    filename = %attachment.filename.as_deref().unwrap_or("unknown"),
                    size_bytes = img.data.len(),
                    media_id = %img.media_ref.id,
                    "Downloaded image"
                );
                images.push(img);
            }
            Err(e) => {
                debug!(error = %e, "Failed to process attachment");
            }
        }
    }

    images
}

/// Build a multi-modal user message with text and images.
///
/// For OpenAI-compatible APIs, this returns a content array:
/// ```json
/// {
///   "role": "user",
///   "content": [
///     { "type": "text", "text": "What's in this image?" },
///     { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,..." } }
///   ]
/// }
/// ```
#[allow(dead_code)]
fn build_multimodal_user_message(text: &str, images: &[ImageData], provider: &str) -> Value {
    use base64::{Engine, engine::general_purpose::STANDARD};

    if images.is_empty() {
        // Simple text message
        return json!({
            "role": "user",
            "content": text
        });
    }

    // Build content array with text and images
    let mut content = Vec::new();

    // Add text part
    if !text.is_empty() {
        content.push(json!({
            "type": "text",
            "text": text
        }));
    }

    // Add image parts
    for img in images {
        let b64 = STANDARD.encode(&img.data);
        let data_url = format!("data:{};base64,{}", img.mime_type, b64);

        if provider == "anthropic" {
            // Anthropic uses different format
            content.push(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": img.mime_type,
                    "data": b64
                }
            }));
        } else {
            // OpenAI format (also works with many compatible APIs)
            content.push(json!({
                "type": "image_url",
                "image_url": {
                    "url": data_url
                }
            }));
        }
    }

    json!({
        "role": "user",
        "content": content
    })
}

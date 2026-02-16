//! Messenger integration for the gateway.
//!
//! This module provides the messenger polling loop that receives messages
//! from configured messengers (Telegram, Discord, Signal, etc.) and routes
//! them through the model for processing with full tool loop support.

use crate::config::{Config, MessengerConfig};
use crate::messengers::{
    DiscordMessenger, GmailConfig, GmailMessenger, GoogleChatConfig, GoogleChatMessenger,
    IrcConfig, IrcMessenger, MattermostConfig, MattermostMessenger, MediaAttachment, Message,
    Messenger, MessengerManager, SendOptions, SlackConfig, SlackMessenger, TeamsConfig,
    TeamsMessenger, TelegramMessenger, WebhookMessenger, WhatsAppConfig, WhatsAppMessenger,
    XmppConfig, XmppMessenger,
};
use crate::pairing::PairingManager;
use crate::tools;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::providers;
use super::secrets_handler;
use super::skills_handler;
use super::{ChatMessage, MediaRef, ModelContext, ProviderRequest, SharedSkillManager, SharedVault, ToolCallResult};

#[cfg(feature = "matrix")]
use crate::messengers::MatrixMessenger;

#[cfg(feature = "signal")]
use crate::messengers::SignalMessenger;

/// Shared messenger manager for the gateway.
pub type SharedMessengerManager = Arc<Mutex<MessengerManager>>;

/// Shared pairing manager for DM security.
pub type SharedPairingManager = Arc<PairingManager>;

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
const SUPPORTED_IMAGE_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
];

/// Create a messenger manager from config.
pub async fn create_messenger_manager(config: &Config) -> Result<MessengerManager> {
    let mut manager = MessengerManager::new();

    for messenger_config in &config.messengers {
        if !messenger_config.enabled {
            continue;
        }
        match create_messenger(messenger_config).await {
            Ok(messenger) => {
                eprintln!(
                    "[messenger] Initialized {} ({})",
                    messenger.name(),
                    messenger.messenger_type()
                );
                manager.add_messenger(messenger);
            }
            Err(e) => {
                eprintln!(
                    "[messenger] Failed to initialize {}: {}",
                    messenger_config.messenger_type, e
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
        "slack" => {
            let token = config
                .token
                .clone()
                .or_else(|| std::env::var("SLACK_BOT_TOKEN").ok())
                .context("Slack requires 'token' or SLACK_BOT_TOKEN env var")?;

            let slack_config = SlackConfig {
                token,
                channel: config.channel_id.clone(),
                poll_interval: 5,
            };

            let messenger = SlackMessenger::new(name, slack_config)
                .context("Failed to create Slack messenger")?;
            Box::new(messenger)
        }
        "whatsapp" => {
            let token = config
                .token
                .clone()
                .or_else(|| std::env::var("WHATSAPP_ACCESS_TOKEN").ok())
                .context("WhatsApp requires 'token' or WHATSAPP_ACCESS_TOKEN env var")?;
            let phone_number_id = config
                .phone_number_id
                .clone()
                .or_else(|| std::env::var("WHATSAPP_PHONE_NUMBER_ID").ok())
                .context(
                    "WhatsApp requires 'phone_number_id' or WHATSAPP_PHONE_NUMBER_ID env var",
                )?;
            let api_version = config
                .api_version
                .clone()
                .or_else(|| std::env::var("WHATSAPP_API_VERSION").ok())
                .unwrap_or_else(|| "v20.0".to_string());

            let wa_config = WhatsAppConfig {
                token,
                phone_number_id,
                api_version,
            };

            Box::new(WhatsAppMessenger::new(name, wa_config))
        }
        "google-chat" => {
            let webhook_url = config
                .webhook_url
                .clone()
                .or_else(|| std::env::var("GOOGLE_CHAT_WEBHOOK_URL").ok());
            let token = config
                .token
                .clone()
                .or_else(|| std::env::var("GOOGLE_CHAT_TOKEN").ok())
                .or_else(|| std::env::var("GOOGLE_CHAT_BOT_TOKEN").ok());
            let space = config
                .space
                .clone()
                .or_else(|| config.channel_id.clone())
                .or_else(|| std::env::var("GOOGLE_CHAT_SPACE").ok());

            if webhook_url.is_none() && (token.is_none() || space.is_none()) {
                anyhow::bail!(
                    "Google Chat requires webhook_url OR token + space (config or env vars)"
                );
            }

            let gc_config = GoogleChatConfig {
                token,
                webhook_url,
                space,
                api_base: config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://chat.googleapis.com/v1".to_string()),
            };

            Box::new(GoogleChatMessenger::new(name, gc_config))
        }
        "teams" => {
            let teams_config = TeamsConfig {
                token: config
                    .token
                    .clone()
                    .or_else(|| std::env::var("TEAMS_BOT_TOKEN").ok())
                    .or_else(|| std::env::var("MICROSOFT_TEAMS_TOKEN").ok()),
                webhook_url: config
                    .webhook_url
                    .clone()
                    .or_else(|| std::env::var("TEAMS_WEBHOOK_URL").ok()),
                team_id: config
                    .team_id
                    .clone()
                    .or_else(|| std::env::var("TEAMS_TEAM_ID").ok()),
                channel_id: config
                    .channel_id
                    .clone()
                    .or_else(|| std::env::var("TEAMS_CHANNEL_ID").ok()),
                api_base: config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://graph.microsoft.com/v1.0".to_string()),
            };

            if teams_config.webhook_url.is_none() && teams_config.token.is_none() {
                anyhow::bail!("Teams requires webhook_url OR token (config or env vars)");
            }

            Box::new(TeamsMessenger::new(name, teams_config))
        }
        "mattermost" => {
            let mattermost_config = MattermostConfig {
                token: config
                    .token
                    .clone()
                    .or_else(|| std::env::var("MATTERMOST_TOKEN").ok()),
                webhook_url: config
                    .webhook_url
                    .clone()
                    .or_else(|| std::env::var("MATTERMOST_WEBHOOK_URL").ok()),
                base_url: config
                    .base_url
                    .clone()
                    .or_else(|| std::env::var("MATTERMOST_BASE_URL").ok())
                    .unwrap_or_else(|| "http://localhost:8065".to_string()),
                default_channel: config
                    .channel_id
                    .clone()
                    .or_else(|| config.default_recipient.clone())
                    .or_else(|| std::env::var("MATTERMOST_CHANNEL_ID").ok()),
            };

            if mattermost_config.webhook_url.is_none() && mattermost_config.token.is_none() {
                anyhow::bail!("Mattermost requires webhook_url OR token (config or env vars)");
            }

            Box::new(MattermostMessenger::new(name, mattermost_config))
        }
        "irc" => {
            let port = config
                .port
                .or_else(|| std::env::var("IRC_PORT").ok().and_then(|v| v.parse().ok()))
                .unwrap_or(6667);
            let irc_config = IrcConfig {
                server: config
                    .server
                    .clone()
                    .or_else(|| std::env::var("IRC_SERVER").ok())
                    .unwrap_or_else(|| "irc.libera.chat".to_string()),
                port,
                nickname: config
                    .nickname
                    .clone()
                    .or_else(|| std::env::var("IRC_NICK").ok())
                    .unwrap_or_else(|| "rustyclaw".to_string()),
                username: config
                    .username
                    .clone()
                    .or_else(|| std::env::var("IRC_USERNAME").ok())
                    .unwrap_or_else(|| "rustyclaw".to_string()),
                realname: config
                    .realname
                    .clone()
                    .or_else(|| std::env::var("IRC_REALNAME").ok())
                    .unwrap_or_else(|| "RustyClaw".to_string()),
                password: config
                    .password
                    .clone()
                    .or_else(|| std::env::var("IRC_PASSWORD").ok()),
                channel: config
                    .channel_id
                    .clone()
                    .or_else(|| config.default_recipient.clone())
                    .or_else(|| std::env::var("IRC_CHANNEL").ok()),
            };

            Box::new(IrcMessenger::new(name, irc_config))
        }
        "xmpp" => {
            let webhook_url = config
                .webhook_url
                .clone()
                .or_else(|| std::env::var("XMPP_WEBHOOK_URL").ok());
            let api_url = config
                .base_url
                .clone()
                .or_else(|| std::env::var("XMPP_API_URL").ok());
            let token = config
                .token
                .clone()
                .or_else(|| std::env::var("XMPP_TOKEN").ok());
            let default_recipient = config
                .default_recipient
                .clone()
                .or_else(|| config.channel_id.clone())
                .or_else(|| std::env::var("XMPP_TO").ok());
            let from = config
                .from
                .clone()
                .or_else(|| std::env::var("XMPP_FROM").ok());

            if webhook_url.is_none() && (api_url.is_none() || token.is_none()) {
                anyhow::bail!("XMPP requires webhook_url OR api_url + token");
            }

            let xmpp_config = XmppConfig {
                webhook_url,
                api_url,
                token,
                from,
                default_recipient,
            };

            Box::new(XmppMessenger::new(name, xmpp_config))
        }
        "webhook" => {
            let url = config
                .webhook_url
                .clone()
                .or_else(|| std::env::var("WEBHOOK_URL").ok())
                .context("Webhook requires 'webhook_url' or WEBHOOK_URL env var")?;
            Box::new(WebhookMessenger::new(name, url))
        }
        #[cfg(feature = "matrix")]
        "matrix" => {
            let homeserver = config
                .homeserver
                .clone()
                .context("Matrix requires 'homeserver'")?;
            let user_id = config.user_id.clone().context("Matrix requires 'user_id'")?;
            let password = config.password.clone();
            let access_token = config.access_token.clone();

            // Store path for Matrix SQLite database
            let store_path = dirs::data_dir()
                .context("Failed to get data directory")?
                .join("rustyclaw")
                .join("matrix")
                .join(&name);

            let messenger = if let Some(pwd) = password {
                MatrixMessenger::with_password(name.clone(), homeserver, user_id, pwd, store_path)
            } else if let Some(token) = access_token {
                // Device ID is optional, defaults to "RUSTYCLAW" if not provided
                MatrixMessenger::with_token(name.clone(), homeserver, user_id, token, None, store_path)
            } else {
                anyhow::bail!("Matrix requires either 'password' or 'access_token'");
            };
            Box::new(messenger)
        }
        #[cfg(feature = "signal")]
        "signal" => {
            let phone = config
                .phone
                .clone()
                .context("Signal requires 'phone' number")?;
            let messenger =
                SignalMessenger::new(&phone).context("Failed to create Signal messenger")?;
            Box::new(messenger)
        }
        "gmail" => {
            let client_id = config
                .client_id
                .clone()
                .context("Gmail requires 'client_id'")?;
            let client_secret = config
                .client_secret
                .clone()
                .context("Gmail requires 'client_secret'")?;
            let refresh_token = config
                .refresh_token
                .clone()
                .context("Gmail requires 'refresh_token'")?;

            let gmail_config = GmailConfig {
                client_id,
                client_secret,
                refresh_token,
                user: config.gmail_user.clone().unwrap_or_else(|| "me".to_string()),
                poll_interval: config.gmail_poll_interval.unwrap_or(60),
                label: config.gmail_label.clone().unwrap_or_else(|| "INBOX".to_string()),
                unread_only: config.gmail_unread_only.unwrap_or(true),
            };

            let messenger = GmailMessenger::new(gmail_config)
                .context("Failed to create Gmail messenger")?;
            Box::new(messenger)
        }
        other => anyhow::bail!("Unknown messenger type: {}", other),
    };

    messenger.initialize().await?;
    Ok(messenger)
}

/// Run the messenger polling loop.
///
/// This polls all configured messengers for incoming messages and routes
/// them through the model for processing with full tool support.
pub async fn run_messenger_loop(
    config: Config,
    messenger_mgr: SharedMessengerManager,
    model_ctx: Option<Arc<ModelContext>>,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    pairing_mgr: Option<SharedPairingManager>,
    cancel: CancellationToken,
) -> Result<()> {
    // If no model context, we can't process messages
    let model_ctx = match model_ctx {
        Some(ctx) => ctx,
        None => {
            eprintln!("[messenger] No model context — messenger loop disabled");
            return Ok(());
        }
    };

    let poll_interval = Duration::from_millis(
        config
            .messenger_poll_interval_ms
            .unwrap_or(2000)
            .max(500) as u64,
    );

    // Per-chat conversation history
    let conversations: ConversationStore = Arc::new(Mutex::new(HashMap::new()));

    let http = reqwest::Client::new();

    eprintln!(
        "[messenger] Starting messenger loop (poll interval: {:?})",
        poll_interval
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                eprintln!("[messenger] Shutting down messenger loop");
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {
                // Poll all messengers for incoming messages
                let messages = {
                    let mgr = messenger_mgr.lock().await;
                    poll_all_messengers(&mgr).await
                };

                // Process each message
                for (messenger_type, msg) in messages {
                    if let Err(e) = process_incoming_message(
                        &http,
                        &config,
                        &messenger_mgr,
                        &model_ctx,
                        &vault,
                        &skill_mgr,
                        &pairing_mgr,
                        &conversations,
                        &messenger_type,
                        msg,
                    )
                    .await
                    {
                        eprintln!("[messenger] Error processing message: {}", e);
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

    for messenger in mgr.get_messengers() {
        match messenger.receive_messages().await {
            Ok(messages) => {
                for msg in messages {
                    all_messages.push((messenger.messenger_type().to_string(), msg));
                }
            }
            Err(e) => {
                eprintln!(
                    "[messenger] Error polling {}: {}",
                    messenger.messenger_type(),
                    e
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
    pairing_mgr: &Option<SharedPairingManager>,
    conversations: &ConversationStore,
    messenger_type: &str,
    msg: Message,
) -> Result<()> {
    eprintln!(
        "[messenger] Received from {} ({}): {}",
        msg.sender,
        messenger_type,
        if msg.content.len() > 50 {
            format!("{}...", &msg.content[..50])
        } else {
            msg.content.clone()
        }
    );

    // Check pairing authorization if enabled
    if let Some(pairing) = pairing_mgr {
        if config.pairing.enabled {
            let is_authorized = pairing.is_authorized(messenger_type, &msg.sender).await;

            if !is_authorized {
                // Check if message contains a pairing code
                let content = msg.content.trim().to_uppercase();

                // If it's an 8-character alphanumeric code, try to verify it
                if content.len() == 8 && content.chars().all(|c| c.is_ascii_alphanumeric()) {
                    if pairing.verify_code(messenger_type, &msg.sender, &content).await {
                        // Code is valid! Auto-approve the sender
                        let sender_name = msg.sender.clone();
                        if let Err(e) = pairing.approve_sender(messenger_type, &msg.sender, sender_name.clone()).await {
                            eprintln!("[pairing] Failed to approve sender {}: {}", sender_name, e);
                            return send_pairing_error(messenger_mgr, messenger_type, &msg).await;
                        }

                        eprintln!("[pairing] ✓ Sender {} ({}) authorized via pairing code", msg.sender, messenger_type);

                        // Send welcome message
                        return send_pairing_welcome(messenger_mgr, messenger_type, &msg).await;
                    } else {
                        // Invalid code
                        return send_pairing_invalid_code(messenger_mgr, messenger_type, &msg).await;
                    }
                }

                // Not authorized and not a valid code — generate and send pairing code
                let code = pairing.generate_code(messenger_type, &msg.sender).await;
                eprintln!("[pairing] Generated pairing code for {} ({}): {}", msg.sender, messenger_type, code);

                return send_pairing_challenge(messenger_mgr, messenger_type, &msg, &code).await;
            }
        }
    }

    let workspace_dir = config.workspace_dir();

    // Show typing indicator while processing
    let typing_channel = msg.channel.as_deref().unwrap_or(&msg.sender);
    {
        let mgr = messenger_mgr.lock().await;
        if let Some(messenger) = mgr.get_messenger_by_type(messenger_type) {
            let _ = messenger.set_typing(typing_channel, true).await;
        }
    }

    // Build conversation key for this chat
    let conv_key = format!(
        "{}:{}",
        messenger_type,
        msg.channel.as_deref().unwrap_or(&msg.sender)
    );

    // Get or create conversation history
    let mut messages = {
        let mut store = conversations.lock().await;
        store.entry(conv_key.clone()).or_insert_with(Vec::new).clone()
    };

    // Build system prompt
    let system_prompt = build_messenger_system_prompt(config, messenger_type, &msg);

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
        eprintln!("[messenger] Processing {} image(s) (vision not yet supported in messenger handler)", images.len());
    }

    // Build media refs for history storage
    let media_refs: Vec<MediaRef> = images.iter().map(|img| img.media_ref.clone()).collect();

    // Add user message to history (with media refs, not raw data)
    messages.push(ChatMessage::user_with_media(&msg.content, media_refs.clone()));

    // Build request - ProviderRequest expects Vec<ChatMessage>
    let mut resolved = ProviderRequest {
        provider: model_ctx.provider.clone(),
        model: model_ctx.model.clone(),
        base_url: model_ctx.base_url.clone(),
        api_key: model_ctx.api_key.clone(),
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
                eprintln!("[messenger] Model error: {}", err);
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
            eprintln!("[messenger] Tool call: {} ({})", tc.name, tc.id);

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
            } else {
                match tools::execute_tool(&tc.name, &tc.arguments, &workspace_dir) {
                    Ok(text) => (text, false),
                    Err(err) => (err, true),
                }
            };

            eprintln!(
                "[messenger] Tool result ({}): {}",
                if is_error { "error" } else { "ok" },
                if output.len() > 100 {
                    format!("{}...", &output[..100])
                } else {
                    output.clone()
                }
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
        history.push(ChatMessage::user_with_media(&msg.content, media_refs.clone()));

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
        if let Some(messenger) = mgr.get_messenger_by_type(messenger_type) {
            // Stop typing indicator before sending response
            let _ = messenger.set_typing(typing_channel, false).await;

            let recipient = msg.channel.as_deref().unwrap_or(&msg.sender);

            let opts = SendOptions {
                recipient,
                content: &final_response,
                reply_to: Some(&msg.id),
                silent: false,
                media: None,
            };

            match messenger.send_message_with_options(opts).await {
                Ok(msg_id) => {
                    eprintln!(
                        "[messenger] Sent response ({}): {}",
                        msg_id,
                        if final_response.len() > 50 {
                            format!("{}...", &final_response[..50])
                        } else {
                            final_response.clone()
                        }
                    );
                }
                Err(e) => {
                    eprintln!("[messenger] Failed to send response: {}", e);
                }
            }
        }
    } else {
        // No response being sent - ensure typing indicator is stopped
        let mgr = messenger_mgr.lock().await;
        if let Some(messenger) = mgr.get_messenger_by_type(messenger_type) {
            let _ = messenger.set_typing(typing_channel, false).await;
        }
    }

    Ok(())
}

/// Build system prompt with messenger context.
fn build_messenger_system_prompt(config: &Config, messenger_type: &str, msg: &Message) -> String {
    let base_prompt = config.system_prompt.clone().unwrap_or_else(|| {
        "You are a helpful AI assistant.".to_string()
    });

    format!(
        "{}\n\n## Messaging Context\n\
        - Channel: {}\n\
        - Sender: {}\n\
        - Platform: {}\n\
        \n\
        When responding:\n\
        - Be concise and appropriate for chat\n\
        - You have access to tools — use them when helpful\n\
        - If you have nothing to say, reply with: NO_REPLY",
        base_prompt,
        msg.channel.as_deref().unwrap_or("direct"),
        msg.sender,
        messenger_type
    )
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
        anyhow::bail!("Image too large: {} bytes (max {})", bytes.len(), MAX_IMAGE_SIZE);
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
        eprintln!("[messenger] Failed to cache image: {}", e);
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
        anyhow::bail!("Image too large: {} bytes (max {})", data.len(), MAX_IMAGE_SIZE);
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
        eprintln!("[messenger] Failed to cache image: {}", e);
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
        eprintln!("[messenger] Failed to create cache dir: {}", e);
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
                eprintln!(
                    "[messenger] Downloaded image: {} ({} bytes) -> {}",
                    attachment.filename.as_deref().unwrap_or("unknown"),
                    img.data.len(),
                    img.media_ref.id
                );
                images.push(img);
            }
            Err(e) => {
                eprintln!("[messenger] Failed to process attachment: {}", e);
            }
        }
    }

    images
}

// ── Pairing Helper Functions ───────────────────────────────────────────────

/// Send pairing challenge to an unauthorized sender
async fn send_pairing_challenge(
    messenger_mgr: &SharedMessengerManager,
    messenger_type: &str,
    msg: &Message,
    code: &str,
) -> Result<()> {
    let response = format!(
        "🔒 Authorization Required\n\n\
        You are not authorized to use this assistant.\n\n\
        **Pairing Code:** `{}`\n\n\
        To complete pairing:\n\
        1. Share this code with the admin\n\
        2. Wait for admin approval\n\
        3. Send the code back to me to verify\n\n\
        This code will expire in 5 minutes.",
        code
    );

    send_messenger_response(messenger_mgr, messenger_type, msg, &response).await
}

/// Send welcome message after successful pairing
async fn send_pairing_welcome(
    messenger_mgr: &SharedMessengerManager,
    messenger_type: &str,
    msg: &Message,
) -> Result<()> {
    let response = "✅ Pairing successful! You are now authorized to use this assistant.";
    send_messenger_response(messenger_mgr, messenger_type, msg, response).await
}

/// Send invalid code message
async fn send_pairing_invalid_code(
    messenger_mgr: &SharedMessengerManager,
    messenger_type: &str,
    msg: &Message,
) -> Result<()> {
    let response = "❌ Invalid pairing code. Please request a new code or contact the admin.";
    send_messenger_response(messenger_mgr, messenger_type, msg, response).await
}

/// Send pairing error message
async fn send_pairing_error(
    messenger_mgr: &SharedMessengerManager,
    messenger_type: &str,
    msg: &Message,
) -> Result<()> {
    let response = "⚠️ Pairing system error. Please try again later or contact the admin.";
    send_messenger_response(messenger_mgr, messenger_type, msg, response).await
}

/// Helper to send a response via the messenger
async fn send_messenger_response(
    messenger_mgr: &SharedMessengerManager,
    messenger_type: &str,
    msg: &Message,
    content: &str,
) -> Result<()> {
    let mgr = messenger_mgr.lock().await;
    if let Some(messenger) = mgr.get_messenger_by_type(messenger_type) {
        let recipient = msg.channel.as_deref().unwrap_or(&msg.sender);

        let opts = SendOptions {
            recipient,
            content,
            reply_to: Some(&msg.id),
            silent: false,
            media: None,
        };

        messenger.send_message_with_options(opts).await?;
    }
    Ok(())
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
    use base64::{engine::general_purpose::STANDARD, Engine};

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

//! Messenger integration for the gateway.
//!
//! This module provides the messenger polling loop that receives messages
//! from configured messengers (Telegram, Discord, Signal, etc.) and routes
//! them through the model for processing with full tool loop support.

use crate::config::{Config, MessengerConfig};
use crate::messengers::{
    DiscordMessenger, MediaAttachment, Message, Messenger, MessengerManager, SendOptions,
    TelegramMessenger, WebhookMessenger,
};
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

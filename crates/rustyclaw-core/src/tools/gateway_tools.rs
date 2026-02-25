//! Gateway tools: gateway, message, tts, image.
//!
//! Provides both sync and async implementations.

use super::helpers::resolve_path;
use serde_json::Value;
use std::path::Path;
use tracing::{debug, warn, instrument};

// ── Async implementations ───────────────────────────────────────────────────

/// Gateway management (async).
#[instrument(skip(args, workspace_dir), fields(action))]
pub async fn exec_gateway_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing gateway tool");

    let config_path = workspace_dir
        .parent()
        .unwrap_or(workspace_dir)
        .join("openclaw.json");

    match action {
        "restart" => {
            let reason = args
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("Restart requested via gateway tool");

            Ok(format!(
                "Gateway restart requested.\nReason: {}\nNote: Actual restart requires daemon integration.",
                reason
            ))
        }

        "config.get" => {
            let exists = tokio::fs::try_exists(&config_path).await.unwrap_or(false);
            if !exists {
                return Ok(serde_json::json!({
                    "config": {},
                    "hash": "",
                    "exists": false
                })
                .to_string());
            }

            let content = tokio::fs::read_to_string(&config_path)
                .await
                .map_err(|e| format!("Failed to read config: {}", e))?;

            let hash = format!(
                "{:x}",
                content.len() * 31 + content.bytes().map(|b| b as usize).sum::<usize>()
            );

            Ok(serde_json::json!({
                "config": content,
                "hash": hash,
                "exists": true,
                "path": config_path.display().to_string()
            })
            .to_string())
        }

        "config.schema" => Ok(serde_json::json!({
            "type": "object",
            "properties": {
                "agents": { "type": "object", "description": "Agent configuration" },
                "channels": { "type": "object", "description": "Channel plugins" },
                "session": { "type": "object", "description": "Session settings" },
                "messages": { "type": "object", "description": "Message formatting" },
                "providers": { "type": "object", "description": "AI providers" }
            }
        })
        .to_string()),

        "config.apply" => {
            let raw = args
                .get("raw")
                .and_then(|v| v.as_str())
                .ok_or("Missing raw config for config.apply")?;

            let _: serde_json::Value =
                serde_json::from_str(raw).map_err(|e| format!("Invalid JSON config: {}", e))?;

            if let Some(parent) = config_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("Failed to create config directory: {}", e))?;
            }

            tokio::fs::write(&config_path, raw)
                .await
                .map_err(|e| format!("Failed to write config: {}", e))?;

            Ok(format!(
                "Config written to {}. Gateway restart required for changes to take effect.",
                config_path.display()
            ))
        }

        "config.patch" => {
            let raw = args
                .get("raw")
                .and_then(|v| v.as_str())
                .ok_or("Missing raw patch for config.patch")?;

            let patch: serde_json::Value =
                serde_json::from_str(raw).map_err(|e| format!("Invalid JSON patch: {}", e))?;

            let exists = tokio::fs::try_exists(&config_path).await.unwrap_or(false);
            let existing = if exists {
                let content = tokio::fs::read_to_string(&config_path)
                    .await
                    .map_err(|e| format!("Failed to read config: {}", e))?;
                serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse existing config: {}", e))?
            } else {
                serde_json::json!({})
            };

            let merged = merge_json(existing, patch);

            let output = serde_json::to_string_pretty(&merged)
                .map_err(|e| format!("Failed to serialize config: {}", e))?;

            tokio::fs::write(&config_path, &output)
                .await
                .map_err(|e| format!("Failed to write config: {}", e))?;

            Ok(format!(
                "Config patched at {}. Gateway restart required for changes to take effect.",
                config_path.display()
            ))
        }

        "update.run" => Ok(
            "Update check requested. Note: Self-update requires external tooling (npm/cargo)."
                .to_string(),
        ),

        _ => {
            warn!(action, "Unknown gateway action");
            Err(format!(
                "Unknown action: {}. Valid: restart, config.get, config.schema, config.apply, config.patch, update.run",
                action
            ))
        }
    }
}

/// Send messages via channel plugins (async).
#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_message_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing message tool");

    match action {
        "send" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("Missing message for send action")?;

            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .ok_or("Missing target for send action")?;

            let channel = args
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("auto");

            match channel {
                "discord" => send_discord_async(target, message).await,
                "telegram" => send_telegram_async(target, message).await,
                "webhook" => {
                    let webhook_url = args
                        .get("webhookUrl")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| std::env::var("WEBHOOK_URL").ok());
                    
                    match webhook_url {
                        Some(url) => send_webhook_async(&url, target, message).await,
                        None => Err("Missing webhookUrl for webhook channel".to_string()),
                    }
                }
                "auto" | _ => {
                    if std::env::var("DISCORD_BOT_TOKEN").is_ok() {
                        send_discord_async(target, message).await
                    } else if std::env::var("TELEGRAM_BOT_TOKEN").is_ok() {
                        send_telegram_async(target, message).await
                    } else {
                        Ok(format!(
                            "Message queued for delivery:\n- Channel: {}\n- Target: {}\n- Message: {} chars\n\nNote: Set DISCORD_BOT_TOKEN or TELEGRAM_BOT_TOKEN to enable actual delivery.",
                            channel,
                            target,
                            message.len()
                        ))
                    }
                }
            }
        }

        "broadcast" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("Missing message for broadcast action")?;

            let targets = args
                .get("targets")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
                .unwrap_or_default();

            if targets.is_empty() {
                return Err("No targets specified for broadcast".to_string());
            }

            let channel = args
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("auto");

            let mut results = Vec::new();
            for target in &targets {
                let result = match channel {
                    "discord" => send_discord_async(target, message).await,
                    "telegram" => send_telegram_async(target, message).await,
                    _ => Ok(format!("Would send to {}", target)),
                };
                results.push(format!("{}: {}", target, result.unwrap_or_else(|e| e)));
            }

            Ok(format!(
                "Broadcast results:\n{}",
                results.join("\n")
            ))
        }

        _ => Err(format!("Unknown action: {}. Valid: send, broadcast", action)),
    }
}

/// Text-to-speech using OpenAI API (async).
#[instrument(skip(args, workspace_dir), fields(text_len))]
pub async fn exec_tts_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: text".to_string())?;

    tracing::Span::current().record("text_len", text.len());
    debug!("Executing TTS");

    let output_dir = workspace_dir.join(".tts");
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(|e| format!("Failed to create TTS output directory: {}", e))?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let output_path = output_dir.join(format!("speech_{}.mp3", timestamp));

    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("TTS_API_KEY"))
        .ok();

    let Some(api_key) = api_key else {
        return Ok(format!(
            "TTS conversion requested:\n- Text: {} chars\n- Output would be: {}\n\nNote: Set OPENAI_API_KEY or TTS_API_KEY to enable actual TTS.\n\nMEDIA: {}",
            text.len(),
            output_path.display(),
            output_path.display()
        ));
    };

    let voice = args
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or("alloy");
    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("tts-1");
    let speed = args
        .get("speed")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .clamp(0.25, 4.0);

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/audio/speech")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": model,
            "input": text,
            "voice": voice,
            "speed": speed,
            "response_format": "mp3"
        }))
        .send()
        .await
        .map_err(|e| format!("TTS API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        return Err(format!("TTS API error ({}): {}", status, error_body));
    }

    let audio_bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read TTS response: {}", e))?;

    tokio::fs::write(&output_path, &audio_bytes)
        .await
        .map_err(|e| format!("Failed to write audio file: {}", e))?;

    Ok(format!(
        "TTS conversion complete:\n- Text: {} chars\n- Voice: {}\n- Model: {}\n- Output: {}\n\nMEDIA: {}",
        text.len(),
        voice,
        model,
        output_path.display(),
        output_path.display()
    ))
}

/// Analyze an image using a vision model (async).
#[instrument(skip(args, workspace_dir))]
pub async fn exec_image_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let image_path = args
        .get("image")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: image".to_string())?;

    debug!(image = image_path, "Executing image analysis");

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("Describe this image in detail.");

    let is_url = image_path.starts_with("http://") || image_path.starts_with("https://");

    let (image_data, media_type) = if is_url {
        (image_path.to_string(), "url".to_string())
    } else {
        let full_path = resolve_path(workspace_dir, image_path);
        let exists = tokio::fs::try_exists(&full_path).await.unwrap_or(false);
        if !exists {
            return Err(format!("Image file not found: {}", image_path));
        }

        let ext = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let mime_type = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => {
                return Err(format!(
                    "Unsupported image format: {}. Supported: jpg, jpeg, png, gif, webp",
                    ext
                ))
            }
        };

        let bytes = tokio::fs::read(&full_path)
            .await
            .map_err(|e| format!("Failed to read image: {}", e))?;
        
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let base64_data = STANDARD.encode(&bytes);
        
        (format!("data:{};base64,{}", mime_type, base64_data), mime_type.to_string())
    };

    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        return call_openai_vision_async(&api_key, &image_data, is_url, prompt).await;
    }
    
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        return call_anthropic_vision_async(&api_key, &image_data, is_url, &media_type, prompt).await;
    }
    
    if let Ok(api_key) = std::env::var("GOOGLE_API_KEY") {
        return call_google_vision_async(&api_key, &image_data, is_url, prompt).await;
    }

    Ok(format!(
        "Image analysis requested:\n- Image: {}\n- Prompt: {}\n- Is URL: {}\n\nNote: Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or GOOGLE_API_KEY to enable vision analysis.",
        image_path,
        prompt,
        is_url
    ))
}

// ── Async helper functions ──────────────────────────────────────────────────

async fn send_discord_async(channel_id: &str, content: &str) -> Result<String, String> {
    let token = std::env::var("DISCORD_BOT_TOKEN")
        .map_err(|_| "DISCORD_BOT_TOKEN not set")?;

    let client = reqwest::Client::new();
    let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_id);

    let response = client
        .post(&url)
        .header("Authorization", format!("Bot {}", token))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "content": content }))
        .send()
        .await
        .map_err(|e| format!("Discord API request failed: {}", e))?;

    if response.status().is_success() {
        let data: Value = response.json().await.unwrap_or_default();
        let msg_id = data["id"].as_str().unwrap_or("unknown");
        Ok(format!("Message sent to Discord channel {}. ID: {}", channel_id, msg_id))
    } else {
        let status = response.status();
        let error = response.text().await.unwrap_or_default();
        Err(format!("Discord API error ({}): {}", status, error))
    }
}

async fn send_telegram_async(chat_id: &str, content: &str) -> Result<String, String> {
    let token = std::env::var("TELEGRAM_BOT_TOKEN")
        .map_err(|_| "TELEGRAM_BOT_TOKEN not set")?;

    let client = reqwest::Client::new();
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": content,
            "parse_mode": "Markdown"
        }))
        .send()
        .await
        .map_err(|e| format!("Telegram API request failed: {}", e))?;

    if response.status().is_success() {
        let data: Value = response.json().await.unwrap_or_default();
        if data["ok"].as_bool() == Some(true) {
            let msg_id = data["result"]["message_id"].as_i64().unwrap_or(0);
            Ok(format!("Message sent to Telegram chat {}. ID: {}", chat_id, msg_id))
        } else {
            Err(format!("Telegram API error: {}", data["description"].as_str().unwrap_or("unknown")))
        }
    } else {
        let status = response.status();
        let error = response.text().await.unwrap_or_default();
        Err(format!("Telegram API error ({}): {}", status, error))
    }
}

async fn send_webhook_async(url: &str, target: &str, content: &str) -> Result<String, String> {
    let client = reqwest::Client::new();

    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "target": target,
            "content": content,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
        .send()
        .await
        .map_err(|e| format!("Webhook request failed: {}", e))?;

    if response.status().is_success() {
        Ok(format!("Message sent via webhook to {}", target))
    } else {
        let status = response.status();
        Err(format!("Webhook error ({})", status))
    }
}

async fn call_openai_vision_async(api_key: &str, image_data: &str, _is_url: bool, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    
    let image_content = serde_json::json!({
        "type": "image_url",
        "image_url": { "url": image_data }
    });

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": prompt },
                    image_content
                ]
            }],
            "max_tokens": 1024
        }))
        .send()
        .await
        .map_err(|e| format!("Vision API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, error_body));
    }

    let data: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse vision response: {}", e))?;

    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("No response generated");

    Ok(content.to_string())
}

async fn call_anthropic_vision_async(api_key: &str, image_data: &str, is_url: bool, media_type: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    
    let image_content = if is_url {
        serde_json::json!({
            "type": "image",
            "source": {
                "type": "url",
                "url": image_data
            }
        })
    } else {
        let base64_data = image_data
            .split(',')
            .nth(1)
            .unwrap_or(image_data);
        serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": base64_data
            }
        })
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    image_content,
                    { "type": "text", "text": prompt }
                ]
            }]
        }))
        .send()
        .await
        .map_err(|e| format!("Vision API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, error_body));
    }

    let data: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse vision response: {}", e))?;

    let content = data["content"][0]["text"]
        .as_str()
        .unwrap_or("No response generated");

    Ok(content.to_string())
}

async fn call_google_vision_async(api_key: &str, image_data: &str, is_url: bool, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    
    let image_part = if is_url {
        serde_json::json!({
            "file_data": {
                "file_uri": image_data,
                "mime_type": "image/jpeg"
            }
        })
    } else {
        let parts: Vec<&str> = image_data.split(',').collect();
        let base64_data = parts.get(1).unwrap_or(&"");
        let mime_type = parts.first()
            .and_then(|p| p.strip_prefix("data:"))
            .and_then(|p| p.split(';').next())
            .unwrap_or("image/jpeg");
        
        serde_json::json!({
            "inline_data": {
                "mime_type": mime_type,
                "data": base64_data
            }
        })
    };

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:generateContent?key={}",
        api_key
    );

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "contents": [{
                "parts": [
                    image_part,
                    { "text": prompt }
                ]
            }]
        }))
        .send()
        .await
        .map_err(|e| format!("Vision API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, error_body));
    }

    let data: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse vision response: {}", e))?;

    let content = data["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("No response generated");

    Ok(content.to_string())
}

// ── Sync implementations ────────────────────────────────────────────────────

/// Recursively merge two JSON values (patch semantics).
fn merge_json(base: Value, patch: Value) -> Value {
    match (base, patch) {
        (Value::Object(mut base_map), Value::Object(patch_map)) => {
            for (key, patch_val) in patch_map {
                if patch_val.is_null() {
                    base_map.remove(&key);
                } else if let Some(base_val) = base_map.remove(&key) {
                    base_map.insert(key, merge_json(base_val, patch_val));
                } else {
                    base_map.insert(key, patch_val);
                }
            }
            Value::Object(base_map)
        }
        (_, patch) => patch,
    }
}

/// Gateway management (sync wrapper).
#[instrument(skip(args, workspace_dir), fields(action))]
pub fn exec_gateway(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing gateway tool");

    let config_path = workspace_dir
        .parent()
        .unwrap_or(workspace_dir)
        .join("openclaw.json");

    match action {
        "restart" => {
            let reason = args
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("Restart requested via gateway tool");

            Ok(format!(
                "Gateway restart requested.\nReason: {}\nNote: Actual restart requires daemon integration.",
                reason
            ))
        }

        "config.get" => {
            if !config_path.exists() {
                return Ok(serde_json::json!({
                    "config": {},
                    "hash": "",
                    "exists": false
                })
                .to_string());
            }

            let content = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("Failed to read config: {}", e))?;

            let hash = format!(
                "{:x}",
                content.len() * 31 + content.bytes().map(|b| b as usize).sum::<usize>()
            );

            Ok(serde_json::json!({
                "config": content,
                "hash": hash,
                "exists": true,
                "path": config_path.display().to_string()
            })
            .to_string())
        }

        "config.schema" => Ok(serde_json::json!({
            "type": "object",
            "properties": {
                "agents": { "type": "object", "description": "Agent configuration" },
                "channels": { "type": "object", "description": "Channel plugins" },
                "session": { "type": "object", "description": "Session settings" },
                "messages": { "type": "object", "description": "Message formatting" },
                "providers": { "type": "object", "description": "AI providers" }
            }
        })
        .to_string()),

        "config.apply" => {
            let raw = args
                .get("raw")
                .and_then(|v| v.as_str())
                .ok_or("Missing raw config for config.apply")?;

            let _: serde_json::Value =
                serde_json::from_str(raw).map_err(|e| format!("Invalid JSON config: {}", e))?;

            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create config directory: {}", e))?;
            }

            std::fs::write(&config_path, raw)
                .map_err(|e| format!("Failed to write config: {}", e))?;

            Ok(format!(
                "Config written to {}. Gateway restart required for changes to take effect.",
                config_path.display()
            ))
        }

        "config.patch" => {
            let raw = args
                .get("raw")
                .and_then(|v| v.as_str())
                .ok_or("Missing raw patch for config.patch")?;

            let patch: serde_json::Value =
                serde_json::from_str(raw).map_err(|e| format!("Invalid JSON patch: {}", e))?;

            let existing = if config_path.exists() {
                let content = std::fs::read_to_string(&config_path)
                    .map_err(|e| format!("Failed to read config: {}", e))?;
                serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse existing config: {}", e))?
            } else {
                serde_json::json!({})
            };

            let merged = merge_json(existing, patch);

            let output = serde_json::to_string_pretty(&merged)
                .map_err(|e| format!("Failed to serialize config: {}", e))?;

            std::fs::write(&config_path, &output)
                .map_err(|e| format!("Failed to write config: {}", e))?;

            Ok(format!(
                "Config patched at {}. Gateway restart required for changes to take effect.",
                config_path.display()
            ))
        }

        "update.run" => Ok(
            "Update check requested. Note: Self-update requires external tooling (npm/cargo)."
                .to_string(),
        ),

        _ => {
            warn!(action, "Unknown gateway action");
            Err(format!(
                "Unknown action: {}. Valid: restart, config.get, config.schema, config.apply, config.patch, update.run",
                action
            ))
        }
    }
}

/// Send messages via channel plugins (sync wrapper).
#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_message(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing message tool");

    match action {
        "send" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("Missing message for send action")?;

            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .ok_or("Missing target for send action")?;

            let channel = args
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("auto");

            match channel {
                "discord" => send_discord_sync(target, message),
                "telegram" => send_telegram_sync(target, message),
                "webhook" => {
                    let webhook_url = args
                        .get("webhookUrl")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| std::env::var("WEBHOOK_URL").ok());
                    
                    match webhook_url {
                        Some(url) => send_webhook_sync(&url, target, message),
                        None => Err("Missing webhookUrl for webhook channel".to_string()),
                    }
                }
                "auto" | _ => {
                    if std::env::var("DISCORD_BOT_TOKEN").is_ok() {
                        send_discord_sync(target, message)
                    } else if std::env::var("TELEGRAM_BOT_TOKEN").is_ok() {
                        send_telegram_sync(target, message)
                    } else {
                        Ok(format!(
                            "Message queued for delivery:\n- Channel: {}\n- Target: {}\n- Message: {} chars\n\nNote: Set DISCORD_BOT_TOKEN or TELEGRAM_BOT_TOKEN to enable actual delivery.",
                            channel,
                            target,
                            message.len()
                        ))
                    }
                }
            }
        }

        "broadcast" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("Missing message for broadcast action")?;

            let targets = args
                .get("targets")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();

            if targets.is_empty() {
                return Err("No targets specified for broadcast".to_string());
            }

            let channel = args
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("auto");

            let mut results = Vec::new();
            for target in &targets {
                let result = match channel {
                    "discord" => send_discord_sync(target, message),
                    "telegram" => send_telegram_sync(target, message),
                    _ => Ok(format!("Would send to {}", target)),
                };
                results.push(format!("{}: {}", target, result.unwrap_or_else(|e| e)));
            }

            Ok(format!(
                "Broadcast results:\n{}",
                results.join("\n")
            ))
        }

        _ => Err(format!("Unknown action: {}. Valid: send, broadcast", action)),
    }
}

/// Text-to-speech using OpenAI API (sync wrapper).
#[instrument(skip(args, workspace_dir), fields(text_len))]
pub fn exec_tts(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    use std::fs;
    use std::io::Write;

    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: text".to_string())?;

    tracing::Span::current().record("text_len", text.len());
    debug!("Executing TTS");

    let output_dir = workspace_dir.join(".tts");
    fs::create_dir_all(&output_dir)
        .map_err(|e| format!("Failed to create TTS output directory: {}", e))?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let output_path = output_dir.join(format!("speech_{}.mp3", timestamp));

    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("TTS_API_KEY"))
        .ok();

    let Some(api_key) = api_key else {
        return Ok(format!(
            "TTS conversion requested:\n- Text: {} chars\n- Output would be: {}\n\nNote: Set OPENAI_API_KEY or TTS_API_KEY to enable actual TTS.\n\nMEDIA: {}",
            text.len(),
            output_path.display(),
            output_path.display()
        ));
    };

    let voice = args
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or("alloy");
    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("tts-1");
    let speed = args
        .get("speed")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .clamp(0.25, 4.0);

    let client = reqwest::blocking::Client::new();
    let response = client
        .post("https://api.openai.com/v1/audio/speech")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": model,
            "input": text,
            "voice": voice,
            "speed": speed,
            "response_format": "mp3"
        }))
        .send()
        .map_err(|e| format!("TTS API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().unwrap_or_default();
        return Err(format!("TTS API error ({}): {}", status, error_body));
    }

    let audio_bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read TTS response: {}", e))?;

    let mut file = fs::File::create(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;
    file.write_all(&audio_bytes)
        .map_err(|e| format!("Failed to write audio file: {}", e))?;

    Ok(format!(
        "TTS conversion complete:\n- Text: {} chars\n- Voice: {}\n- Model: {}\n- Output: {}\n\nMEDIA: {}",
        text.len(),
        voice,
        model,
        output_path.display(),
        output_path.display()
    ))
}

/// Analyze an image using a vision model (sync wrapper).
#[instrument(skip(args, workspace_dir))]
pub fn exec_image(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    use std::fs;

    let image_path = args
        .get("image")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: image".to_string())?;

    debug!(image = image_path, "Executing image analysis");

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("Describe this image in detail.");

    let is_url = image_path.starts_with("http://") || image_path.starts_with("https://");

    let (image_data, media_type) = if is_url {
        (image_path.to_string(), "url".to_string())
    } else {
        let full_path = resolve_path(workspace_dir, image_path);
        if !full_path.exists() {
            return Err(format!("Image file not found: {}", image_path));
        }

        let ext = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let mime_type = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => {
                return Err(format!(
                    "Unsupported image format: {}. Supported: jpg, jpeg, png, gif, webp",
                    ext
                ))
            }
        };

        let bytes = fs::read(&full_path)
            .map_err(|e| format!("Failed to read image: {}", e))?;
        
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let base64_data = STANDARD.encode(&bytes);
        
        (format!("data:{};base64,{}", mime_type, base64_data), mime_type.to_string())
    };

    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        return call_openai_vision_sync(&api_key, &image_data, is_url, prompt);
    }
    
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        return call_anthropic_vision_sync(&api_key, &image_data, is_url, &media_type, prompt);
    }
    
    if let Ok(api_key) = std::env::var("GOOGLE_API_KEY") {
        return call_google_vision_sync(&api_key, &image_data, is_url, prompt);
    }

    Ok(format!(
        "Image analysis requested:\n- Image: {}\n- Prompt: {}\n- Is URL: {}\n\nNote: Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or GOOGLE_API_KEY to enable vision analysis.",
        image_path,
        prompt,
        is_url
    ))
}

// ── Sync helper functions ───────────────────────────────────────────────────

fn send_discord_sync(channel_id: &str, content: &str) -> Result<String, String> {
    let token = std::env::var("DISCORD_BOT_TOKEN")
        .map_err(|_| "DISCORD_BOT_TOKEN not set")?;

    let client = reqwest::blocking::Client::new();
    let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_id);

    let response = client
        .post(&url)
        .header("Authorization", format!("Bot {}", token))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "content": content }))
        .send()
        .map_err(|e| format!("Discord API request failed: {}", e))?;

    if response.status().is_success() {
        let data: Value = response.json().unwrap_or_default();
        let msg_id = data["id"].as_str().unwrap_or("unknown");
        Ok(format!("Message sent to Discord channel {}. ID: {}", channel_id, msg_id))
    } else {
        let status = response.status();
        let error = response.text().unwrap_or_default();
        Err(format!("Discord API error ({}): {}", status, error))
    }
}

fn send_telegram_sync(chat_id: &str, content: &str) -> Result<String, String> {
    let token = std::env::var("TELEGRAM_BOT_TOKEN")
        .map_err(|_| "TELEGRAM_BOT_TOKEN not set")?;

    let client = reqwest::blocking::Client::new();
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": content,
            "parse_mode": "Markdown"
        }))
        .send()
        .map_err(|e| format!("Telegram API request failed: {}", e))?;

    if response.status().is_success() {
        let data: Value = response.json().unwrap_or_default();
        if data["ok"].as_bool() == Some(true) {
            let msg_id = data["result"]["message_id"].as_i64().unwrap_or(0);
            Ok(format!("Message sent to Telegram chat {}. ID: {}", chat_id, msg_id))
        } else {
            Err(format!("Telegram API error: {}", data["description"].as_str().unwrap_or("unknown")))
        }
    } else {
        let status = response.status();
        let error = response.text().unwrap_or_default();
        Err(format!("Telegram API error ({}): {}", status, error))
    }
}

fn send_webhook_sync(url: &str, target: &str, content: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();

    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "target": target,
            "content": content,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
        .send()
        .map_err(|e| format!("Webhook request failed: {}", e))?;

    if response.status().is_success() {
        Ok(format!("Message sent via webhook to {}", target))
    } else {
        let status = response.status();
        Err(format!("Webhook error ({})", status))
    }
}

fn call_openai_vision_sync(api_key: &str, image_data: &str, _is_url: bool, prompt: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    
    let image_content = serde_json::json!({
        "type": "image_url",
        "image_url": { "url": image_data }
    });

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": prompt },
                    image_content
                ]
            }],
            "max_tokens": 1024
        }))
        .send()
        .map_err(|e| format!("Vision API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, error_body));
    }

    let data: Value = response
        .json()
        .map_err(|e| format!("Failed to parse vision response: {}", e))?;

    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("No response generated");

    Ok(content.to_string())
}

fn call_anthropic_vision_sync(api_key: &str, image_data: &str, is_url: bool, media_type: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    
    let image_content = if is_url {
        serde_json::json!({
            "type": "image",
            "source": {
                "type": "url",
                "url": image_data
            }
        })
    } else {
        let base64_data = image_data
            .split(',')
            .nth(1)
            .unwrap_or(image_data);
        serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": base64_data
            }
        })
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    image_content,
                    { "type": "text", "text": prompt }
                ]
            }]
        }))
        .send()
        .map_err(|e| format!("Vision API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, error_body));
    }

    let data: Value = response
        .json()
        .map_err(|e| format!("Failed to parse vision response: {}", e))?;

    let content = data["content"][0]["text"]
        .as_str()
        .unwrap_or("No response generated");

    Ok(content.to_string())
}

fn call_google_vision_sync(api_key: &str, image_data: &str, is_url: bool, prompt: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    
    let image_part = if is_url {
        serde_json::json!({
            "file_data": {
                "file_uri": image_data,
                "mime_type": "image/jpeg"
            }
        })
    } else {
        let parts: Vec<&str> = image_data.split(',').collect();
        let base64_data = parts.get(1).unwrap_or(&"");
        let mime_type = parts.first()
            .and_then(|p| p.strip_prefix("data:"))
            .and_then(|p| p.split(';').next())
            .unwrap_or("image/jpeg");
        
        serde_json::json!({
            "inline_data": {
                "mime_type": mime_type,
                "data": base64_data
            }
        })
    };

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:generateContent?key={}",
        api_key
    );

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "contents": [{
                "parts": [
                    image_part,
                    { "text": prompt }
                ]
            }]
        }))
        .send()
        .map_err(|e| format!("Vision API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, error_body));
    }

    let data: Value = response
        .json()
        .map_err(|e| format!("Failed to parse vision response: {}", e))?;

    let content = data["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("No response generated");

    Ok(content.to_string())
}

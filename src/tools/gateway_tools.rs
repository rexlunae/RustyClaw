//! Gateway tools: gateway, message, tts, image.

use super::helpers::resolve_path;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Gateway management.
pub fn exec_gateway(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

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

        "audit.query" => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 1000) as usize)
                .unwrap_or(100);
            let event = args.get("event").and_then(|v| v.as_str());

            let log_path = resolve_audit_log_path(args, workspace_dir, &config_path);
            let entries = crate::hooks::builtin::query_audit_log(&log_path, event, limit)
                .map_err(|e| format!("Failed to query audit log {}: {}", log_path.display(), e))?;

            Ok(serde_json::json!({
                "path": log_path.display().to_string(),
                "exists": log_path.exists(),
                "event": event,
                "limit": limit,
                "count": entries.len(),
                "entries": entries,
            })
            .to_string())
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: restart, config.get, config.schema, config.apply, config.patch, update.run, audit.query",
            action
        )),
    }
}

fn resolve_audit_log_path(args: &Value, workspace_dir: &Path, config_path: &Path) -> PathBuf {
    if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
        return resolve_path(workspace_dir, path);
    }

    if let Some(path) = configured_audit_log_path(config_path) {
        return resolve_path(workspace_dir, &path);
    }

    workspace_dir.join(".rustyclaw").join("logs").join("audit.log")
}

fn configured_audit_log_path(config_path: &Path) -> Option<String> {
    if !config_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(config_path).ok()?;
    let parsed: Value = serde_json::from_str(&content).ok()?;
    parsed
        .get("hooks")
        .and_then(|h| h.get("audit_log_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

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

/// Send messages via channel plugins.
///
/// Supports Discord and Telegram when bot tokens are configured via environment.
/// Falls back to stub behavior if no tokens are available.
pub fn exec_message(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

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

            // Try to send via configured messenger
            match channel {
                "discord" => send_discord(target, message),
                "telegram" => send_telegram(target, message),
                "webhook" => {
                    let webhook_url = args
                        .get("webhookUrl")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| std::env::var("WEBHOOK_URL").ok());
                    
                    match webhook_url {
                        Some(url) => send_webhook(&url, target, message),
                        None => Err("Missing webhookUrl for webhook channel".to_string()),
                    }
                }
                "auto" | _ => {
                    // Try messengers in order
                    if std::env::var("DISCORD_BOT_TOKEN").is_ok() {
                        send_discord(target, message)
                    } else if std::env::var("TELEGRAM_BOT_TOKEN").is_ok() {
                        send_telegram(target, message)
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
                    "discord" => send_discord(target, message),
                    "telegram" => send_telegram(target, message),
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

/// Send a message via Discord bot API.
fn send_discord(channel_id: &str, content: &str) -> Result<String, String> {
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

/// Send a message via Telegram bot API.
fn send_telegram(chat_id: &str, content: &str) -> Result<String, String> {
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

/// Send a message via webhook POST.
fn send_webhook(url: &str, target: &str, content: &str) -> Result<String, String> {
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

/// Text-to-speech conversion using OpenAI TTS API.
///
/// Falls back to stub behavior if no API key is available.
pub fn exec_tts(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: text".to_string())?;

    // Create output directory
    let output_dir = workspace_dir.join(".tts");
    fs::create_dir_all(&output_dir)
        .map_err(|e| format!("Failed to create TTS output directory: {}", e))?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let output_path = output_dir.join(format!("speech_{}.mp3", timestamp));

    // Try to get API key from environment or vault
    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("TTS_API_KEY"))
        .ok();

    let Some(api_key) = api_key else {
        // No API key - return stub response
        return Ok(format!(
            "TTS conversion requested:\n- Text: {} chars\n- Output would be: {}\n\nNote: Set OPENAI_API_KEY or TTS_API_KEY to enable actual TTS.\n\nMEDIA: {}",
            text.len(),
            output_path.display(),
            output_path.display()
        ));
    };

    // Get optional parameters
    let voice = args
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or("alloy"); // alloy, echo, fable, onyx, nova, shimmer
    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("tts-1"); // tts-1 or tts-1-hd
    let speed = args
        .get("speed")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .clamp(0.25, 4.0);

    // Call OpenAI TTS API (blocking)
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

    // Write audio to file
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

/// Analyze an image using a vision model.
///
/// Supports OpenAI GPT-4V, Anthropic Claude 3, and Google Gemini.
/// Falls back to stub behavior if no API key is available.
pub fn exec_image(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let image_path = args
        .get("image")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: image".to_string())?;

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("Describe this image in detail.");

    // Check if it's a URL or local path
    let is_url = image_path.starts_with("http://") || image_path.starts_with("https://");

    // For local files, read and encode as base64
    let (image_data, media_type) = if is_url {
        (image_path.to_string(), "url".to_string())
    } else {
        // Resolve local path
        let full_path = resolve_path(workspace_dir, image_path);
        if !full_path.exists() {
            return Err(format!("Image file not found: {}", image_path));
        }

        // Check it's actually an image
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

        // Read and encode
        let bytes = fs::read(&full_path)
            .map_err(|e| format!("Failed to read image: {}", e))?;
        
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let base64_data = STANDARD.encode(&bytes);
        
        (format!("data:{};base64,{}", mime_type, base64_data), mime_type.to_string())
    };

    // Try providers in order: OpenAI, Anthropic, Google
    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        return call_openai_vision(&api_key, &image_data, is_url, prompt);
    }
    
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        return call_anthropic_vision(&api_key, &image_data, is_url, &media_type, prompt);
    }
    
    if let Ok(api_key) = std::env::var("GOOGLE_API_KEY") {
        return call_google_vision(&api_key, &image_data, is_url, prompt);
    }

    // No API key available - return stub
    Ok(format!(
        "Image analysis requested:\n- Image: {}\n- Prompt: {}\n- Is URL: {}\n\nNote: Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or GOOGLE_API_KEY to enable vision analysis.",
        image_path,
        prompt,
        is_url
    ))
}

/// Call OpenAI GPT-4V for image analysis.
fn call_openai_vision(api_key: &str, image_data: &str, is_url: bool, prompt: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    
    let image_content = if is_url {
        serde_json::json!({
            "type": "image_url",
            "image_url": { "url": image_data }
        })
    } else {
        serde_json::json!({
            "type": "image_url",
            "image_url": { "url": image_data }
        })
    };

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

/// Call Anthropic Claude 3 for image analysis.
fn call_anthropic_vision(api_key: &str, image_data: &str, is_url: bool, media_type: &str, prompt: &str) -> Result<String, String> {
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
        // Extract base64 from data URL
        let base64_data = image_data
            .split(",")
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

/// Call Google Gemini for image analysis.
fn call_google_vision(api_key: &str, image_data: &str, is_url: bool, prompt: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    
    let image_part = if is_url {
        serde_json::json!({
            "file_data": {
                "file_uri": image_data,
                "mime_type": "image/jpeg"
            }
        })
    } else {
        // Extract base64 and mime type from data URL
        let parts: Vec<&str> = image_data.split(",").collect();
        let base64_data = parts.get(1).unwrap_or(&"");
        let mime_type = parts.get(0)
            .and_then(|p| p.strip_prefix("data:"))
            .and_then(|p| p.split(";").next())
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

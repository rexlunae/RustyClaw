//! Gateway WebSocket client helpers and the headless `ask` command.

use anyhow::{Context, Result};
use clap::Args;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use rustyclaw_core::commands::{CommandAction, CommandContext, handle_command};
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{
    ClientFrame, ClientFrameType, ClientPayload, ServerFrame, ServerFrameType, ServerPayload,
    deserialize_frame, serialize_frame,
};
use rustyclaw_core::skills::SkillManager;

use super::shared::open_secrets;

#[derive(Debug, Args)]
#[command(after_help = "\
EXAMPLES:
  rustyclaw ask 'What is 2+2?'
  echo 'Summarize this' | rustyclaw ask --stdin
  rustyclaw ask --model anthropic/claude-haiku 'Quick question'
  rustyclaw ask --no-tools 'Just chat, no actions'
")]
pub(crate) struct AskArgs {
    /// Prompt text (can also be provided via --stdin)
    #[arg(value_name = "PROMPT", trailing_var_arg = true)]
    prompt: Vec<String>,
    /// Read prompt from stdin
    #[arg(long)]
    stdin: bool,
    /// Model to use (overrides default)
    #[arg(long, short, value_name = "MODEL")]
    model: Option<String>,
    /// Disable tool use (pure chat mode)
    #[arg(long)]
    no_tools: bool,
    /// Output raw JSON response
    #[arg(long)]
    json: bool,
    /// System prompt override
    #[arg(long, value_name = "PROMPT")]
    system: Option<String>,
    /// Gateway WebSocket URL (ws://…)
    #[arg(
        long = "gateway",
        alias = "url",
        alias = "ws",
        value_name = "WS_URL",
        env = "RUSTYCLAW_GATEWAY"
    )]
    gateway: Option<String>,
    /// Maximum tokens in response
    #[arg(long, value_name = "TOKENS")]
    max_tokens: Option<u32>,
    /// Temperature (0.0-2.0)
    #[arg(long, value_name = "TEMP")]
    temperature: Option<f32>,
}

pub(crate) fn run_local_command(config: &mut Config, input: &str) -> Result<()> {
    let mut secrets_manager = open_secrets(config)?;
    let skills_dir = config.skills_dir();
    let mut skill_manager = SkillManager::new(skills_dir);
    skill_manager.load_skills()?;

    let mut context = CommandContext {
        secrets_manager: &mut secrets_manager,
        skill_manager: &mut skill_manager,
        config,
    };

    let response = handle_command(input, &mut context);
    if response.action == CommandAction::ClearMessages {
        for message in response.messages {
            println!("{}", message);
        }
        return Ok(());
    }

    if response.action == CommandAction::Quit {
        return Ok(());
    }

    for message in response.messages {
        println!("{}", message);
    }

    Ok(())
}

/// Send a reload command to the running gateway and wait for the result.
pub(crate) async fn send_gateway_reload(
    gateway_url: &str,
    totp_enabled: bool,
) -> Result<(String, String)> {
    let url = Url::parse(gateway_url).context("Invalid gateway URL")?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(url.to_string())
        .await
        .context("Failed to connect to gateway. Is it running?")?;
    let (mut writer, mut reader) = ws_stream.split();

    // Handle auth challenge if TOTP is enabled
    if totp_enabled {
        loop {
            let msg = match reader.next().await {
                Some(m) => m,
                None => anyhow::bail!("Connection closed"),
            };
            let msg = msg.context("Gateway read error")?;
            // Handle both binary and text frames (for backwards compat during transition)
            match msg {
                Message::Binary(data) => {
                    if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                        match frame.frame_type {
                            ServerFrameType::AuthChallenge => {
                                if let ServerPayload::AuthChallenge { method: _ } = frame.payload {
                                    let code = rpassword::prompt_password(format!(
                                        "{} 2FA code: ",
                                        rustyclaw_core::theme::info("🔑")
                                    ))
                                    .unwrap_or_default();
                                    let auth_frame = ClientFrame {
                                        frame_type: ClientFrameType::AuthResponse,
                                        payload: ClientPayload::AuthResponse {
                                            code: code.trim().to_string(),
                                        },
                                    };
                                    let bytes = serialize_frame(&auth_frame)
                                        .map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
                                    writer.send(Message::Binary(bytes.into())).await?;
                                }
                            }
                            ServerFrameType::AuthResult => {
                                if let ServerPayload::AuthResult {
                                    ok,
                                    message,
                                    retry: _,
                                } = frame.payload
                                {
                                    if !ok {
                                        let msg = message.as_deref().unwrap_or("Auth failed");
                                        anyhow::bail!("{}", msg);
                                    }
                                    break; // Auth succeeded
                                }
                            }
                            ServerFrameType::Hello => {
                                break; // No auth needed
                            }
                            _ => {}
                        }
                    }
                }
                Message::Text(text) => {
                    // Also handle text frames for backwards compat
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                        let frame_type = val.get("type").and_then(|t| t.as_str());
                        if frame_type == Some("auth_challenge") {
                            let code = rpassword::prompt_password(format!(
                                "{} 2FA code: ",
                                rustyclaw_core::theme::info("🔑")
                            ))
                            .unwrap_or_default();
                            let auth_frame = ClientFrame {
                                frame_type: ClientFrameType::AuthResponse,
                                payload: ClientPayload::AuthResponse {
                                    code: code.trim().to_string(),
                                },
                            };
                            let bytes = serialize_frame(&auth_frame)
                                .map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
                            writer.send(Message::Binary(bytes.into())).await?;
                            continue;
                        }
                        if frame_type == Some("auth_result") {
                            let ok = val.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                            if !ok {
                                let msg = val
                                    .get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Auth failed");
                                anyhow::bail!("{}", msg);
                            }
                            break;
                        }
                        if frame_type == Some("hello") {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Wait for hello frame
    let mut _result_provider = String::new();
    let mut _result_model = String::new();
    loop {
        match reader.next().await {
            Some(Ok(Message::Binary(data))) => {
                if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                    match frame.frame_type {
                        ServerFrameType::Hello => {
                            if let ServerPayload::Hello {
                                provider, model, ..
                            } = frame.payload
                            {
                                _result_provider = provider.unwrap_or_default();
                                _result_model = model.unwrap_or_default();
                                break;
                            }
                        }
                        ServerFrameType::AuthChallenge if totp_enabled => {
                            // Prompt the user for their TOTP 2FA code and reply
                            // with an AuthResponse frame.
                            let code = rpassword::prompt_password(format!(
                                "{} 2FA code: ",
                                rustyclaw_core::theme::info("🔑")
                            ))
                            .unwrap_or_default();
                            let auth_frame = ClientFrame {
                                frame_type: ClientFrameType::AuthResponse,
                                payload: ClientPayload::AuthResponse {
                                    code: code.trim().to_string(),
                                },
                            };
                            let bytes = serialize_frame(&auth_frame)
                                .map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
                            writer.send(Message::Binary(bytes.into())).await?;
                        }
                        _ => {}
                    }
                }
            }
            Some(Ok(Message::Text(text))) => {
                // Also handle text frames for backwards compat
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                    let frame_type = val.get("type").and_then(|t| t.as_str());
                    if frame_type == Some("hello") || frame_type == Some("auth_challenge") {
                        if frame_type == Some("auth_challenge") && !totp_enabled {
                            let code = rpassword::prompt_password(format!(
                                "{} 2FA code: ",
                                rustyclaw_core::theme::info("🔑")
                            ))
                            .unwrap_or_default();
                            let auth_frame = ClientFrame {
                                frame_type: ClientFrameType::AuthResponse,
                                payload: ClientPayload::AuthResponse {
                                    code: code.trim().to_string(),
                                },
                            };
                            let bytes = serialize_frame(&auth_frame)
                                .map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
                            writer.send(Message::Binary(bytes.into())).await?;
                            continue;
                        }
                        let provider = val
                            .get("provider")
                            .and_then(|p| p.as_str())
                            .unwrap_or("")
                            .to_string();
                        let model = val
                            .get("model")
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .to_string();
                        _result_provider = provider;
                        _result_model = model;
                        break;
                    }
                }
            }
            Some(Ok(Message::Close(_))) => {
                anyhow::bail!("Gateway closed connection");
            }
            None => {
                anyhow::bail!("Gateway disconnected");
            }
            _ => {}
        }
    }

    // Drain remaining status frames briefly
    let drain_timeout = tokio::time::sleep(std::time::Duration::from_millis(500));
    tokio::pin!(drain_timeout);
    loop {
        tokio::select! {
            _ = &mut drain_timeout => break,
            msg = reader.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                            let ft = val.get("type").and_then(|t| t.as_str());
                            if ft == Some("status") || ft == Some("model_configured") || ft == Some("model_ready") || ft == Some("model_error") {
                                continue; // Skip status frames
                            }
                        }
                        break; // Non-status frame, stop draining
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => anyhow::bail!("Gateway error during drain: {}", e),
                    None => anyhow::bail!("Gateway closed unexpectedly"),
                }
            }
        }
    }

    // Send reload command using binary frame
    let reload_frame = ClientFrame {
        frame_type: ClientFrameType::Reload,
        payload: ClientPayload::Reload,
    };
    let bytes =
        serialize_frame(&reload_frame).map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
    writer
        .send(Message::Binary(bytes.into()))
        .await
        .context("Failed to send reload command")?;

    // Wait for reload_result
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => {
                anyhow::bail!("Timeout waiting for reload result");
            }
            msg = reader.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                            match frame.frame_type {
                                ServerFrameType::ReloadResult => {
                                    if let ServerPayload::ReloadResult { ok, provider, model, message } = frame.payload {
                                        if ok {
                                            // Close cleanly
                                            let _ = writer.send(Message::Close(None)).await;
                                            return Ok((provider, model));
                                        } else {
                                            let msg = message.as_deref().unwrap_or("Unknown error");
                                            anyhow::bail!("{}", msg);
                                        }
                                    }
                                }
                                _ => continue,
                            }
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                            let frame_type = val.get("type").and_then(|t| t.as_str());
                            if frame_type == Some("reload_result") {
                                let ok = val.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                                if ok {
                                    let provider = val.get("provider").and_then(|p| p.as_str()).unwrap_or("unknown").to_string();
                                    let model = val.get("model").and_then(|m| m.as_str()).unwrap_or("unknown").to_string();
                                    // Close cleanly
                                    let _ = writer.send(Message::Close(None)).await;
                                    return Ok((provider, model));
                                } else {
                                    let msg = val.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
                                    anyhow::bail!("{}", msg);
                                }
                            }
                            // Skip other frames (status updates from reload)
                            continue;
                        }
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => anyhow::bail!("Gateway error: {}", e),
                    None => anyhow::bail!("Gateway closed without reload result"),
                }
            }
        }
    }
}

pub(crate) async fn send_command_via_gateway(gateway_url: &str, command: &str) -> Result<String> {
    let url = Url::parse(gateway_url).context("Invalid gateway URL")?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(url.to_string())
        .await
        .context("Failed to connect to gateway")?;
    let (mut writer, mut reader) = ws_stream.split();
    writer
        .send(Message::Text(command.to_string().into()))
        .await
        .context("Failed to send command")?;

    while let Some(message) = reader.next().await {
        let message = message.context("Gateway read error")?;
        if let Message::Text(text) = message {
            return Ok(text.to_string());
        }
    }

    anyhow::bail!("Gateway closed without responding")
}

/// Handle the `ask` command — headless model interaction.
pub(crate) async fn handle_ask(config: &Config, args: AskArgs) -> Result<()> {
    use rustyclaw_core::gateway::protocol::types::ChatMessage;
    use rustyclaw_core::gateway::protocol::{
        ClientFrame, ClientFrameType, ClientPayload, ServerFrame, ServerFrameType, ServerPayload,
        deserialize_frame, serialize_frame,
    };
    use std::io::{self, Read};

    // Gather the prompt
    let prompt = if args.stdin {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        if !args.prompt.is_empty() {
            // Prepend CLI args to stdin content
            format!("{}\n\n{}", args.prompt.join(" "), buf)
        } else {
            buf
        }
    } else {
        args.prompt.join(" ")
    };

    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        anyhow::bail!("No prompt provided. Use `rustyclaw ask 'your prompt'` or `--stdin`.");
    }

    // Determine gateway URL
    let gateway_url = args
        .gateway
        .or_else(|| config.gateway_url.clone())
        .unwrap_or_else(|| "ws://127.0.0.1:9001".to_string());

    // Connect to gateway
    let url = Url::parse(&gateway_url).context("Invalid gateway URL")?;
    let (ws_stream, _) = tokio_tungstenite::connect_async(url.to_string())
        .await
        .context("Failed to connect to gateway. Is it running? Try `rustyclaw gateway start`")?;
    let (mut writer, mut reader) = ws_stream.split();

    // Handle auth if needed (simplified — skip TOTP for now)
    // TODO: Add TOTP support for headless mode

    // Build the chat message
    let message = ChatMessage::text("user", &prompt);

    // Send as ClientFrame
    let frame = ClientFrame {
        frame_type: ClientFrameType::Chat,
        payload: ClientPayload::Chat {
            messages: vec![message],
        },
    };
    let bytes = serialize_frame(&frame).map_err(|e| anyhow::anyhow!("serialize failed: {}", e))?;
    writer.send(Message::Binary(bytes.into())).await?;

    // Collect response
    let mut response_text = String::new();
    let mut tool_outputs: Vec<String> = Vec::new();

    while let Some(message) = reader.next().await {
        let message = message.context("Gateway read error")?;

        match message {
            Message::Binary(data) => {
                if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                    match frame.frame_type {
                        ServerFrameType::Chunk => {
                            if let ServerPayload::Chunk { delta } = frame.payload {
                                if !args.json {
                                    // Stream text to stdout
                                    print!("{}", delta);
                                    io::Write::flush(&mut io::stdout())?;
                                }
                                response_text.push_str(&delta);
                            }
                        }
                        ServerFrameType::ResponseDone => {
                            // Model finished
                            if !args.json {
                                println!(); // Final newline
                            }
                            break;
                        }
                        ServerFrameType::ToolCall => {
                            if let ServerPayload::ToolCall { id: _, name, .. } = frame.payload {
                                if !args.json {
                                    eprintln!("  → {}", name);
                                }
                            }
                        }
                        ServerFrameType::ToolResult => {
                            if let ServerPayload::ToolResult {
                                id: _,
                                name,
                                result,
                                ..
                            } = frame.payload
                            {
                                tool_outputs.push(format!("{}: {}", name, result));
                            }
                        }
                        ServerFrameType::Error => {
                            if let ServerPayload::Error { message, .. } = frame.payload {
                                anyhow::bail!("Gateway error: {}", message);
                            }
                        }
                        ServerFrameType::Info => {
                            if let ServerPayload::Info { message } = frame.payload {
                                if !args.json {
                                    eprintln!("  ℹ {}", message);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::Text(text) => {
                // Legacy text frame — just print it
                if !args.json {
                    print!("{}", text);
                }
                response_text.push_str(&text);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // JSON output if requested
    if args.json {
        let output = serde_json::json!({
            "response": response_text,
            "tool_calls": tool_outputs,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

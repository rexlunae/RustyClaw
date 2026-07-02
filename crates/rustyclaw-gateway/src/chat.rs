//! Chat-frame handling: thread bookkeeping and context assembly.
//!
//! [`handle_chat_frame`] is the per-frame entry point for a client `Chat`
//! payload. It auto-switches threads, records the user message, assembles the
//! full prompt (system prompt, prior history, background-task context, and
//! relevant memories), then hands off to
//! [`dispatch_text_message`](crate::dispatch::dispatch_text_message) for the
//! model/tool loop.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::warn;

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{
    ChatMessage, ChatRequest, ScopedTransportWriter, ServerFrame, ServerFrameType, ServerPayload,
    transport,
};

use crate::dispatch::dispatch_text_message;
use crate::thread_updates::{send_thread_messages_update, send_threads_update};
use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedObserver, SharedSkillManager,
    SharedTaskManager, SharedVault, ToolCancelFlag, providers, system_prompt,
};
use protocol::server::send_frame;
use rustyclaw_core::gateway::protocol;

/// Handle a client `Chat` frame: bookkeeping, context assembly, dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_chat_frame(
    http: &reqwest::Client,
    messages: Vec<ChatMessage>,
    stream_id: u64,
    writer: &mut dyn transport::TransportWriter,
    config: &Config,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    task_mgr: &SharedTaskManager,
    observer: Option<&SharedObserver>,
    tool_cancel: &ToolCancelFlag,
    shared_config: &SharedConfig,
    shared_model_ctx: &SharedModelCtx,
    shared_copilot_session: &SharedCopilotSession,
    approval_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, bool)>>>,
    user_prompt_rx: &Arc<
        Mutex<
            tokio::sync::mpsc::Receiver<(
                String,
                bool,
                rustyclaw_core::user_prompt_types::PromptResponseValue,
            )>,
        >,
    >,
    credential_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, bool, Option<String>)>>>,
    dom_query_rx: &Arc<Mutex<tokio::sync::mpsc::Receiver<(String, String, bool)>>>,
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    threads_path: &std::path::Path,
) -> Result<()> {
    // Check for auto-switch: find better matching thread
    if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
        if let Some(better_thread_id) = thread_mgr.find_best_match(&last_user.content) {
            // Found a better match — switch threads
            if thread_mgr.switch_foreground(better_thread_id) {
                // Get the context summary from the new foreground thread
                let context_summary = thread_mgr
                    .foreground()
                    .and_then(|t| t.compact_summary.clone());
                // Send ThreadSwitched notification
                let frame = ServerFrame {
                    frame_type: ServerFrameType::ThreadSwitched,
                    payload: ServerPayload::ThreadSwitched {
                        thread_id: better_thread_id.0,
                        context_summary,
                    },
                };
                send_frame(writer, &frame).await?;
                // Update thread list
                send_threads_update(writer, thread_mgr, task_mgr, None).await?;
                send_thread_messages_update(writer, better_thread_id, thread_mgr).await?;
            }
        }
    }

    // Add user message to current thread's history
    let mut did_auto_label = false;
    let mut needs_caption = false;
    let mut did_append_user_message = false;
    let mut active_thread_id = None;
    if let Some(thread) = thread_mgr.foreground_mut() {
        active_thread_id = Some(thread.id);
        // Find the last user message (typically the new one)
        if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
            // Check if this is the first message in a new thread
            let is_first_message = thread.message_count() == 0
                && (thread.label.is_empty()
                    || thread.label.starts_with("Session #")
                    || thread.label == "Main");
            thread.add_message(
                rustyclaw_core::threads::MessageRole::User,
                &last_user.content,
            );
            did_append_user_message = true;
            if is_first_message {
                // Set a temporary auto-label as fallback
                let label = auto_thread_label(&last_user.content);
                thread.label = label;
                did_auto_label = true;
                // Flag for agent captioning
                needs_caption = true;
            }
        }
    }
    if did_append_user_message && let Err(e) = thread_mgr.save_to_file(threads_path) {
        warn!(error = %e, path = ?threads_path, "Failed to persist user message to thread history");
    }
    if did_auto_label {
        send_threads_update(writer, thread_mgr, task_mgr, None).await?;
    }

    // Auto-ingest user message into Steel Memory
    #[cfg(feature = "semantic-memory")]
    if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
        let ws = config.workspace_dir().to_path_buf();
        let text = last_user.content.clone();
        tokio::spawn(async move {
            if let Ok(mem) = rustyclaw_core::steel_memory::SteelMemory::new(&ws) {
                let _ = mem.add_memory(&text, "conversations", "user", None).await;
            }
        });
    }
    if let Some(thread_id) = active_thread_id {
        send_thread_messages_update(writer, thread_id, thread_mgr).await?;
    }

    // Re-read model_ctx from shared state for each dispatch
    let current_model_ctx = shared_model_ctx.read().await.clone();
    // Re-read copilot session from shared state
    let copilot_session = shared_copilot_session.read().await.clone();
    let workspace_dir = config.workspace_dir();

    // Ensure a system prompt is present. The TUI
    // sends the full conversation (including a
    // system message), but the desktop client
    // only sends the user message. When missing,
    // build one from the workspace context so
    // that SOUL.md, IDENTITY.md, etc. are
    // included.
    let mut messages = messages;
    let client_sent_history = !messages.is_empty() && messages[0].role == "system";
    if !client_sent_history {
        let sys = system_prompt::build_system_prompt(config, task_mgr, skill_mgr).await;
        messages.insert(0, ChatMessage::text("system", &sys));

        // Inject conversation history from the
        // thread. The desktop client only sends
        // the current user message; we need to
        // include prior turns so the model has
        // context of the conversation.
        if let Some(thread) = thread_mgr.foreground() {
            let history = &thread.messages;
            // history includes the message we just
            // added — skip it (last element) to
            // avoid duplication with the client's
            // user message already in `messages`.
            let prior_count = history.len().saturating_sub(1);
            if prior_count > 0 {
                // Optionally include compact summary as context
                if let Some(summary) = &thread.compact_summary {
                    messages.insert(
                        1,
                        ChatMessage::text(
                            "system",
                            &format!("# Previous conversation summary\n\n{}", summary),
                        ),
                    );
                }
                let insert_pos = if thread.compact_summary.is_some() {
                    2
                } else {
                    1
                };
                // Reconstruct the history with structured
                // tool_call / tool_result payloads so that
                // assistant messages keep their `tool_calls`
                // and following tool results stay anchored
                // to them. Flattening to plain text would
                // produce orphan `tool` messages that the
                // provider rejects.
                let provider_name = current_model_ctx
                    .as_deref()
                    .map(|c| c.provider.as_str())
                    .unwrap_or("openai");
                let history_slice: Vec<rustyclaw_core::threads::ThreadMessage> =
                    history.iter().take(prior_count).cloned().collect();
                let history_msgs: Vec<ChatMessage> =
                    providers::thread_history_to_chat_messages(provider_name, &history_slice);
                // Insert history between system prompt and current user message
                let tail = messages.split_off(insert_pos);
                messages.extend(history_msgs);
                messages.extend(tail);
            }
        }
    }

    // Inject thread context into system prompt if available
    let mut messages_with_context = {
        let global_ctx = thread_mgr.build_global_context();
        let provider_name = current_model_ctx
            .as_deref()
            .map(|c| c.provider.as_str())
            .unwrap_or("openai");
        let thread_context = active_thread_id.and_then(|thread_id| {
            thread_mgr.get(thread_id).map(|thread| {
                let history: Vec<rustyclaw_core::threads::ThreadMessage> =
                    thread.messages.iter().cloned().collect();
                (
                    providers::thread_history_to_chat_messages(provider_name, &history),
                    thread.compact_summary.clone(),
                )
            })
        });
        let (mut msgs, compact_summary) =
            thread_context.unwrap_or_else(|| (messages.clone(), None));
        if let Some(system_message) = messages.first().filter(|m| m.role == "system") {
            if msgs.first().map(|m| m.role.as_str()) != Some("system") {
                msgs.insert(0, system_message.clone());
            }
        }
        // Re-inject the stored compaction summary so context from compacted
        // turns survives across prompts (the thread history above only holds
        // the messages kept after compaction).
        if let Some(summary) = compact_summary {
            let insert_pos = if msgs.first().map(|m| m.role.as_str()) == Some("system") {
                1
            } else {
                0
            };
            msgs.insert(
                insert_pos,
                ChatMessage::text(
                    "system",
                    &format!("# Previous conversation summary\n\n{}", summary),
                ),
            );
        }
        if !global_ctx.is_empty() && !msgs.is_empty() && msgs[0].role == "system" {
            msgs[0].content = format!(
                "{}\n\n# Background Tasks\n\n{}",
                msgs[0].content, global_ctx
            );
            msgs
        } else {
            msgs
        }
    };

    // Inject captioning instruction for new threads
    if needs_caption
        && !messages_with_context.is_empty()
        && messages_with_context[0].role == "system"
    {
        messages_with_context[0].content = format!(
            "{}\n\n## Thread Captioning\n\
            This is the first message in a new conversation thread. \
            After responding, call `set_thread_caption` with a short \
            2-6 word caption that summarises the topic of this conversation.",
            messages_with_context[0].content
        );
    }

    // Inject relevant memory context from Steel Memory
    #[cfg(feature = "semantic-memory")]
    if !messages_with_context.is_empty() && messages_with_context[0].role == "system" {
        if let Some(last_user) = messages_with_context
            .iter()
            .rev()
            .find(|m| m.role == "user")
        {
            let query = last_user.content.clone();
            let ws = config.workspace_dir().to_path_buf();
            if let Ok(mem) = rustyclaw_core::steel_memory::SteelMemory::new(&ws) {
                if let Ok(results) = mem.search(&query, 3, Some(0.4)).await {
                    if !results.is_empty() {
                        let mut ctx = String::from("\n\n## Relevant Memories\n");
                        for r in &results {
                            let snippet = if r.content.len() > 300 {
                                format!("{}…", &r.content[..300])
                            } else {
                                r.content.clone()
                            };
                            ctx.push_str(&format!(
                                "- (similarity {:.2}) {}\n",
                                r.similarity, snippet
                            ));
                        }
                        messages_with_context[0].content.push_str(&ctx);
                    }
                }
            }
        }
    }

    // Build a ChatRequest from the messages
    let chat_request = ChatRequest {
        msg_type: "chat".to_string(),
        messages: messages_with_context,
        model: None,
        provider: None,
        base_url: None,
        api_key: None,
    };

    let mut stream_writer = ScopedTransportWriter::new(writer, stream_id);
    if let Err(err) = dispatch_text_message(
        http,
        &chat_request,
        current_model_ctx.as_deref(),
        copilot_session.as_deref(),
        &mut stream_writer,
        &workspace_dir,
        vault,
        skill_mgr,
        task_mgr,
        observer,
        tool_cancel,
        shared_config,
        shared_copilot_session,
        approval_rx,
        user_prompt_rx,
        credential_rx,
        dom_query_rx,
        thread_mgr,
        threads_path,
    )
    .await
    {
        warn!(error = %err, error_debug = ?err, "Chat dispatch failed");
        let error_frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: format!("{err:#}"),
            },
        };
        send_frame(&mut stream_writer, &error_frame).await?;
    }

    Ok(())
}

/// Derive a short thread label from the first user message.
fn auto_thread_label(content: &str) -> String {
    let trimmed = content.trim();
    // Use the first line, capped at 50 chars on a word boundary.
    let first_line = trimmed.lines().next().unwrap_or(trimmed);
    if first_line.len() <= 50 {
        first_line.to_string()
    } else {
        match first_line[..50].rfind(' ') {
            Some(pos) if pos > 20 => format!("{}…", &first_line[..pos]),
            _ => format!("{}…", &first_line[..50]),
        }
    }
}

//! Thread/task client-frame handlers.
//!
//! Each function handles one `ClientPayload` variant in the thread family
//! (create / switch / list / history / close / rename) plus `TasksRequest`,
//! operating on the connection's [`ThreadManager`](rustyclaw_core::threads::ThreadManager)
//! and streaming the resulting frames back to the client.

use anyhow::Result;
use tracing::{debug, info};

use rustyclaw_core::gateway::protocol::server::{send_frame, send_info};
use rustyclaw_core::gateway::{
    ChatMessage, ProviderRequest, ServerFrame, ServerFrameType, ServerPayload, protocol, transport,
};
use rustyclaw_core::threads::ThreadId;

use crate::thread_updates::{send_thread_messages_update, send_threads_update};
use crate::{SharedModelCtx, SharedTaskManager, providers};

/// Handle a `TasksRequest`: send the current task list.
pub(crate) async fn handle_tasks_request(
    writer: &mut dyn transport::TransportWriter,
    task_mgr: &SharedTaskManager,
    session: Option<String>,
) -> Result<()> {
    // Build task list and send back
    let tasks = if let Some(ref sess) = session {
        task_mgr.for_session(sess).await
    } else {
        task_mgr.active().await
    };
    let dto_tasks: Vec<protocol::TaskInfoDto> = tasks
        .iter()
        .map(|t| protocol::TaskInfoDto {
            id: t.id.0,
            label: t.display_label(),
            description: t.description.clone(),
            status: format!("{:?}", t.status)
                .split('{')
                .next()
                .unwrap_or("Unknown")
                .trim()
                .to_string(),
            is_foreground: t.status.is_foreground(),
        })
        .collect();
    let frame = ServerFrame {
        frame_type: ServerFrameType::TasksUpdate,
        payload: ServerPayload::TasksUpdate { tasks: dto_tasks },
    };
    send_frame(writer, &frame).await
}

/// Handle a `ThreadCreate`: create a new thread and broadcast the new list.
pub(crate) async fn handle_thread_create(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    project_id: rustyclaw_core::projects::ProjectId,
    label: String,
) -> Result<()> {
    let label = if label.is_empty() {
        format!("Session #{}", thread_mgr.list().len() + 1)
    } else {
        label
    };
    debug!("Thread create request: {} (project {})", label, project_id);
    let thread_id = thread_mgr.create_chat_in(project_id, &label);
    let frame = ServerFrame {
        frame_type: ServerFrameType::ThreadCreated,
        payload: ServerPayload::ThreadCreated {
            thread_id: thread_id.0,
            label,
        },
    };
    send_frame(writer, &frame).await?;
    // Send updated thread list
    send_threads_update(writer, thread_mgr, task_mgr, None).await?;
    // Persist thread state
    let _ = thread_mgr.save_to_file(threads_path);
    Ok(())
}

/// Handle a `ThreadSwitch`: compact the current thread, switch foreground.
///
/// `thread_id == 0` is a sentinel meaning "background the current thread".
pub(crate) async fn handle_thread_switch(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    shared_model_ctx: &SharedModelCtx,
    http: &reqwest::Client,
    thread_id: u64,
) -> Result<()> {
    debug!("Thread switch request: {}", thread_id);

    // thread_id == 0 is a sentinel meaning "background current thread"
    if thread_id == 0 {
        // Clear foreground — no thread is active
        thread_mgr.clear_foreground();
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadSwitched,
            payload: ServerPayload::ThreadSwitched {
                thread_id: 0,
                context_summary: None,
            },
        };
        send_frame(writer, &frame).await?;
        send_threads_update(writer, thread_mgr, task_mgr, None).await?;
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadMessages,
            payload: ServerPayload::ThreadMessages {
                thread_id: 0,
                messages: Vec::new(),
            },
        };
        send_frame(writer, &frame).await?;
        let _ = thread_mgr.save_to_file(threads_path);
        return Ok(());
    }

    let target_id = ThreadId(thread_id);

    // Get current foreground thread for compaction
    let current_fg_id = thread_mgr.foreground().map(|t| t.task_id());

    // Compact the current thread if it has messages
    if let Some(fg_id) = current_fg_id {
        if fg_id != target_id {
            if let Some(thread) = thread_mgr.get_mut(fg_id) {
                if thread.messages.len() > 3 && thread.compact_summary.is_none() {
                    // Generate compaction prompt
                    let prompt = thread.compaction_prompt();

                    // Notify client about compaction
                    send_info(writer, &format!("Compacting thread '{}'...", thread.label)).await?;

                    // Call LLM to summarize
                    let current_model_ctx = shared_model_ctx.read().await.clone();
                    if let Some(ref ctx) = current_model_ctx {
                        let summary_req = ProviderRequest {
                            messages: vec![ChatMessage::text("user", &prompt)],
                            model: ctx.model.clone(),
                            provider: ctx.provider.clone(),
                            base_url: ctx.base_url.clone(),
                            api_key: ctx.api_key.clone(),
                        };

                        let summary_result = if ctx.provider == "anthropic" {
                            providers::call_anthropic_with_tools(http, &summary_req, None).await
                        } else if ctx.provider == "google" {
                            providers::call_google_with_tools(http, &summary_req).await
                        } else {
                            providers::call_openai_with_tools(http, &summary_req, None).await
                        };

                        match summary_result {
                            Ok(resp) if !resp.text.is_empty() => {
                                thread.apply_compaction(resp.text);
                                debug!(thread = %thread.label, "Thread compacted");
                            }
                            Ok(_) => {
                                debug!(thread = %thread.label, "Empty summary from LLM");
                            }
                            Err(e) => {
                                debug!(thread = %thread.label, error = %e, "Compaction failed");
                            }
                        }
                    }
                }
            }
        }
    }

    // Get summary of thread being switched to
    let context_summary = thread_mgr
        .get(target_id)
        .and_then(|t| t.compact_summary.clone());

    // Perform the switch (use switch_foreground which returns bool,
    // not switch_to which returns old foreground ID — the latter
    // returns None when there is no previous foreground, e.g. after /thread bg)
    if thread_mgr.switch_foreground(target_id) {
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadSwitched,
            payload: ServerPayload::ThreadSwitched {
                thread_id,
                context_summary,
            },
        };
        send_frame(writer, &frame).await?;
        // Send updated thread list
        send_threads_update(writer, thread_mgr, task_mgr, None).await?;
        send_thread_messages_update(writer, target_id, thread_mgr).await?;
        // Persist thread state (includes compaction summary)
        let _ = thread_mgr.save_to_file(threads_path);
    } else {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: format!("Thread {} not found", thread_id),
            },
        };
        send_frame(writer, &frame).await?;
    }
    Ok(())
}

/// Handle a `ThreadList`: broadcast the thread list and foreground history.
pub(crate) async fn handle_thread_list(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    task_mgr: &SharedTaskManager,
) -> Result<()> {
    debug!("Thread list request");
    send_threads_update(writer, thread_mgr, task_mgr, None).await?;
    let fg_id = thread_mgr.foreground().map(|t| t.id);
    if let Some(id) = fg_id {
        send_thread_messages_update(writer, id, thread_mgr).await?;
    }
    Ok(())
}

/// Handle a `ThreadHistoryRequest`: reply with one thread's full message log.
pub(crate) async fn handle_thread_history(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &rustyclaw_core::threads::ThreadManager,
    thread_id: u64,
) -> Result<()> {
    debug!("Thread history request: {}", thread_id);
    let target_id = ThreadId(thread_id);
    let (ok, messages, error) = match thread_mgr.get(target_id) {
        Some(thread) => {
            let wire: Vec<ChatMessage> = thread
                .messages
                .iter()
                .map(|m| {
                    let role = match m.role {
                        rustyclaw_core::threads::MessageRole::User => "user",
                        rustyclaw_core::threads::MessageRole::Assistant => "assistant",
                        rustyclaw_core::threads::MessageRole::System => "system",
                        rustyclaw_core::threads::MessageRole::Tool => "tool",
                    };
                    ChatMessage {
                        role: role.to_string(),
                        content: m.content.clone(),
                        tool_calls: m.tool_calls.clone(),
                        tool_call_id: m.tool_call_id.clone(),
                        media: None,
                    }
                })
                .collect();
            info!(
                thread_id,
                caption = %thread.label,
                message_count = wire.len(),
                "Gateway loaded thread history"
            );
            (true, wire, None)
        }
        None => (
            false,
            Vec::new(),
            Some(format!("Thread {} not found", thread_id)),
        ),
    };
    let frame = ServerFrame {
        frame_type: ServerFrameType::ThreadHistoryReply,
        payload: ServerPayload::ThreadHistoryReply {
            thread_id,
            ok,
            messages,
            error,
        },
    };
    debug!(thread_id, ok, "Sending ThreadHistoryReply");
    send_frame(writer, &frame).await
}

/// Handle a `ThreadClose`: remove a thread and broadcast the new list.
pub(crate) async fn handle_thread_close(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    thread_id: u64,
) -> Result<()> {
    debug!("Thread close request: {}", thread_id);
    let task_id = ThreadId(thread_id);
    thread_mgr.remove(task_id);
    // Send updated thread list
    send_threads_update(writer, thread_mgr, task_mgr, None).await?;
    // Persist thread state
    let _ = thread_mgr.save_to_file(threads_path);
    Ok(())
}

/// Handle a `ThreadRename`: relabel a thread and broadcast the new list.
pub(crate) async fn handle_thread_rename(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    thread_id: u64,
    new_label: String,
) -> Result<()> {
    debug!("Thread rename request: {} -> {}", thread_id, new_label);
    let task_id = ThreadId(thread_id);
    if thread_mgr.rename(task_id, &new_label) {
        // Send updated thread list
        send_threads_update(writer, thread_mgr, task_mgr, None).await?;
        // Persist thread state
        let _ = thread_mgr.save_to_file(threads_path);
    } else {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: format!("Thread {} not found", thread_id),
            },
        };
        send_frame(writer, &frame).await?;
    }
    Ok(())
}

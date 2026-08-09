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
use tracing::warn;

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{
    ChatMessage, ChatRequest, ScopedTransportWriter, ServerFrame, ServerFrameType, ServerPayload,
    SessionOrigin, transport,
};

use crate::dispatch::dispatch_text_message;
use crate::thread_updates::{send_thread_messages_update_shared, send_threads_update_shared};
use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedObserver, SharedSkillManager,
    SharedTaskManager, SharedThreadMgr, SharedVault, ToolCancelFlag, providers, system_prompt,
};
use protocol::server::send_frame;
use rustyclaw_core::gateway::protocol;

/// Handle a client `Chat` frame: bookkeeping, context assembly, dispatch.
///
/// `origin` says where the message came from — resolved by the connection
/// loop from the peer address and the client's declared kind, and injected
/// into the system prompt so the agent knows where it is being spoken to
/// from.
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
    approvals: &Arc<crate::pending::PendingResponses<bool>>,
    user_prompts: &Arc<
        crate::pending::PendingResponses<(
            bool,
            rustyclaw_core::user_prompt_types::PromptResponseValue,
        )>,
    >,
    credentials: &Arc<crate::pending::PendingResponses<(bool, Option<String>)>>,
    dom_queries: &Arc<crate::pending::PendingResponses<(String, bool)>>,
    thread_mgr: &SharedThreadMgr,
    turn_thread: Option<rustyclaw_core::threads::ThreadId>,
    threads_path: &std::path::Path,
    origin: SessionOrigin,
    // Read live rather than captured: a sidebar update sent from this turn
    // carries a `foreground_id` the client acts on, and the user may have
    // switched threads since the turn started.
    foreground: &crate::ForegroundCell,
    // A resumed turn replays the conversation already in the thread's log;
    // its last user message is recorded, and recording it again would
    // duplicate it in the transcript.
    is_resume: bool,
    // Direction the user adds after this turn starts. Drained between rounds
    // by the tool loop, so the model reads it on its next pass.
    steers: &crate::concurrent::SteerQueue,
) -> Result<()> {
    // The thread this turn belongs to. The connection loop settles it —
    // including the auto-switch to a better-matching thread — before
    // handing the turn off, because that loop keeps serving ThreadSwitch
    // frames while this runs: anything resolved here, whether now or
    // lazily through `foreground()`, would file the message and the reply
    // in whichever thread the user opened next. Every lock scope below is
    // a single operation that ends before the next client write, since a
    // write blocking while the lock is held would wedge both sides.
    let active_thread_id = turn_thread;

    // Add user message to the turn's thread history
    let mut did_auto_label = false;
    let mut needs_caption = false;
    let mut did_append_user_message = false;
    if let Some(turn_thread) = active_thread_id.filter(|_| !is_resume) {
        let mut tm = thread_mgr.lock().await;
        if let Some(thread) = tm.get_mut(turn_thread) {
            // Find the last user message (typically the new one)
            if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
                // Check if this is the first message in a new thread
                let is_first_message = thread.message_count() == 0
                    && (thread.label.is_empty()
                        || thread.label.starts_with("Session #")
                        || thread.label == "Main");
                thread.add_message_with_id(
                    last_user.id.clone(),
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
        if did_append_user_message {
            // Through the store, like every other persistence point: the
            // legacy save wrote a threads.json the loader no longer reads,
            // so a crash mid-answer lost the message — and left a start
            // marker with no message behind it, a thread stuck open.
            crate::helpers::persist_threads(&mut tm, threads_path);
        }
    }
    if did_auto_label {
        send_threads_update_shared(
            writer,
            thread_mgr,
            task_mgr,
            None,
            crate::foreground_of(foreground),
        )
        .await?;
    }

    // Auto-ingest user message into Steel Memory
    #[cfg(feature = "semantic-memory")]
    if let Some(last_user) = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .filter(|_| !is_resume)
    {
        let ws = config.workspace_dir().to_path_buf();
        let text = last_user.content.clone();
        tokio::spawn(async move {
            if let Ok(mem) = rustyclaw_core::steel_memory::SteelMemory::new(&ws) {
                // Detached, so there is no caller to return this to — but a
                // dropped write means the conversation is not remembered, and
                // the only symptom later is recall that comes up empty for
                // something the user knows they said.
                if let Err(e) = mem.add_memory(&text, "conversations", "user", None).await {
                    tracing::warn!(error = %e, "the user's message was not written to memory");
                }
            }
        });
    }
    if let Some(thread_id) = active_thread_id {
        send_thread_messages_update_shared(writer, thread_id, thread_mgr).await?;
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
        // `origin` was resolved by the connection loop: Desktop/Tui for local
        // connections whose client declared its kind, Remote for anything
        // non-loopback, Local as the fallback for older clients.
        let sys = system_prompt::build_system_prompt_full(
            config,
            task_mgr,
            None,
            skill_mgr,
            system_prompt::SessionContext {
                platform: Some(origin.as_ref()),
                origin: Some(origin),
                ..Default::default()
            },
        )
        .await;
        messages.insert(0, ChatMessage::text("system", &sys));

        // Inject conversation history from the
        // thread. The desktop client only sends
        // the current user message; we need to
        // include prior turns so the model has
        // context of the conversation.
        // Only the messages inside the context window: the record keeps
        // the whole conversation, and the summary below stands in for the
        // part before the boundary. Sending both would make compaction
        // *grow* the prompt.
        let turn_history = {
            let tm = thread_mgr.lock().await;
            active_thread_id.and_then(|id| tm.get(id)).map(|t| {
                (
                    t.context_messages().cloned().collect::<Vec<_>>(),
                    t.compact_summary.clone(),
                )
            })
        };
        // A resumed turn's `messages` *is* the recorded conversation;
        // injecting the history again would double every prior message.
        if let Some((history, compact_summary)) = turn_history.filter(|_| !is_resume) {
            let history = &history;
            // history includes the message we just
            // added — skip it (last element) to
            // avoid duplication with the client's
            // user message already in `messages`.
            let prior_count = history.len().saturating_sub(1);
            if prior_count > 0 {
                // Optionally include compact summary as context
                if let Some(summary) = &compact_summary {
                    messages.insert(
                        1,
                        ChatMessage::text(
                            "system",
                            &format!("# Previous conversation summary\n\n{}", summary),
                        ),
                    );
                }
                let insert_pos = if compact_summary.is_some() { 2 } else { 1 };
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
        let provider_name = current_model_ctx
            .as_deref()
            .map(|c| c.provider.as_str())
            .unwrap_or("openai");
        let (global_ctx, thread_context) = {
            let tm = thread_mgr.lock().await;
            let thread_context = active_thread_id.and_then(|thread_id| {
                tm.get(thread_id).map(|thread| {
                    // The context window, not the record — see above.
                    let history: Vec<rustyclaw_core::threads::ThreadMessage> =
                        thread.context_messages().cloned().collect();
                    (
                        providers::thread_history_to_chat_messages(provider_name, &history),
                        thread.compact_summary.clone(),
                    )
                })
            });
            (tm.build_global_context(turn_thread), thread_context)
        };
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
        approvals,
        user_prompts,
        credentials,
        dom_queries,
        thread_mgr,
        active_thread_id,
        threads_path,
        foreground,
        steers,
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
///
/// Budgets are in characters, not bytes: this runs on arbitrary user input,
/// and byte-indexing it panicked on any message whose first line exceeded 50
/// bytes without a character boundary there (e.g. CJK or emoji).
fn auto_thread_label(content: &str) -> String {
    /// Hard cap on the generated label.
    const MAX_CHARS: usize = 50;
    /// Only break at a space if it leaves at least this much text; otherwise
    /// a long unbroken run would collapse to a stub.
    const MIN_WORD_BREAK_CHARS: usize = 20;

    let trimmed = content.trim();
    let first_line = trimmed.lines().next().unwrap_or(trimmed);

    if first_line.chars().count() <= MAX_CHARS {
        return first_line.to_string();
    }

    let head: String = first_line.chars().take(MAX_CHARS).collect();
    // `rfind` yields a byte index of an ASCII space, which is always a
    // character boundary, so slicing `head` here is safe.
    match head.rfind(' ') {
        Some(pos) if head[..pos].chars().count() > MIN_WORD_BREAK_CHARS => {
            format!("{}…", &head[..pos])
        }
        _ => format!("{}…", head),
    }
}

#[cfg(test)]
mod auto_thread_label_tests {
    use super::auto_thread_label;

    #[test]
    fn short_messages_pass_through() {
        assert_eq!(auto_thread_label("hello there"), "hello there");
        assert_eq!(auto_thread_label("  padded  "), "padded");
        assert_eq!(auto_thread_label(""), "");
    }

    #[test]
    fn only_the_first_line_is_used() {
        assert_eq!(auto_thread_label("title\nbody\nmore"), "title");
    }

    #[test]
    fn long_ascii_breaks_on_a_word_boundary() {
        let msg = "the quick brown fox jumps over the lazy dog and keeps running onward";
        let label = auto_thread_label(msg);
        assert!(label.ends_with('…'));
        assert!(
            !label.trim_end_matches('…').ends_with(' '),
            "breaks at a word, not mid-word: {label}"
        );
        assert!(label.chars().count() <= 51, "within budget: {label}");
    }

    /// Regression: this input panicked with
    /// "byte index 50 is not a char boundary".
    #[test]
    fn multibyte_input_does_not_panic() {
        let msg = "日本語のメッセージをここに書きます、これはとても長いテキストです";
        assert!(msg.len() > 50, "exceeds a 50-byte budget");
        let label = auto_thread_label(msg);
        assert_eq!(label, msg, "32 characters is under the 50-character cap");

        // A genuinely over-budget multibyte line still truncates cleanly.
        let long: String = "あ".repeat(80);
        let label = auto_thread_label(&long);
        assert_eq!(label.chars().count(), 51, "50 chars plus the ellipsis");
        assert!(label.ends_with('…'));
    }

    #[test]
    fn emoji_input_does_not_panic() {
        let label = auto_thread_label(&"🦞".repeat(60));
        assert_eq!(label.chars().count(), 51);
    }
}

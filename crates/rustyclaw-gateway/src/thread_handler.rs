//! Thread/task client-frame handlers.
//!
//! Each function handles one `ClientPayload` variant in the thread family
//! (create / switch / list / history / close / rename / update) plus `TasksRequest`,
//! operating on the connection's [`ThreadManager`](rustyclaw_core::threads::ThreadManager)
//! and streaming the resulting frames back to the client.

use anyhow::Result;
use tracing::{debug, info};

use rustyclaw_core::gateway::protocol::server::{send_frame, send_info};
use rustyclaw_core::gateway::{
    ChatMessage, ProviderRequest, ServerFrame, ServerFrameType, ServerPayload, protocol, transport,
};
use rustyclaw_core::threads::ThreadId;

use crate::thread_updates::{send_thread_messages_update_shared, send_threads_update_shared};
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
    thread_mgr: &crate::SharedThreadMgr,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    project_id: rustyclaw_core::projects::ProjectId,
    label: String,
    foreground: &crate::ForegroundCell,
) -> Result<()> {
    let (thread_id, label) = {
        let mut tm = thread_mgr.lock().await;
        let label = if label.is_empty() {
            format!("Session #{}", tm.list().len() + 1)
        } else {
            label
        };
        debug!("Thread create request: {} (project {})", label, project_id);
        let thread_id = tm.create_chat_in(project_id, &label);
        crate::helpers::persist_threads(&mut tm, threads_path);
        (thread_id, label)
    };
    let frame = ServerFrame {
        frame_type: ServerFrameType::ThreadCreated,
        payload: ServerPayload::ThreadCreated {
            thread_id: thread_id.0,
            label,
        },
    };
    send_frame(writer, &frame).await?;
    // The client that asked for the thread is the one now in it. Set here
    // rather than left to the manager: `create_chat_in` foregrounds on the
    // shared manager, which every other window on this agent is watching.
    crate::set_foreground(foreground, Some(thread_id));
    // Send updated thread list
    send_threads_update_shared(writer, thread_mgr, task_mgr, None, Some(thread_id)).await?;
    Ok(())
}

/// Recent messages left in context by a compaction, over and above the
/// summarised prefix.
const COMPACT_KEEP_RECENT: usize = 3;

/// How many trailing messages must stay in context when a summary that
/// described `covered` messages is applied to a thread that now holds `now`.
///
/// Keeping a fixed count is only right when nothing was appended in between.
/// Anything added after the prompt was taken is *not* in the summary, so it
/// has to stay on the visible side of the boundary — otherwise it is dropped
/// from the model's view without ever having been described. The boundary
/// this produces is always `covered - COMPACT_KEEP_RECENT`, wherever the end
/// of the thread has moved to.
fn keep_after_compaction(covered: usize, now: usize) -> usize {
    COMPACT_KEEP_RECENT + now.saturating_sub(covered)
}

/// Handle a `ThreadSwitch`: compact the current thread, switch foreground.
///
/// `thread_id == 0` is a sentinel meaning "background the current thread".
///
/// Takes the shared manager rather than a borrow, because compaction calls
/// the model: a guard held across that call would freeze a turn running in
/// parallel — which is exactly the thing a user is doing when they switch
/// threads mid-answer — for as long as the provider takes. Each lock scope
/// below is one operation, and the model call happens between them.
pub(crate) async fn handle_thread_switch(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &crate::SharedThreadMgr,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    shared_model_ctx: &SharedModelCtx,
    http: &reqwest::Client,
    thread_id: u64,
    // This connection's focused thread. Not the manager's: that is shared
    // with every other window on this agent, and a switch is one client
    // saying where *it* is looking.
    foreground: &crate::ForegroundCell,
    // Threads with a turn running. Their history is still being written, so
    // summarising one now would both miss the answer in flight and drop
    // messages that answer is building on.
    //
    // A set, not one id: several turns run at once, and asking "is the
    // thread I am about to compact busy" of a single arbitrarily-chosen
    // entry answers a different question.
    busy_threads: &[ThreadId],
) -> Result<()> {
    debug!("Thread switch request: {}", thread_id);

    // thread_id == 0 is a sentinel meaning "background current thread"
    if thread_id == 0 {
        // Clear foreground — no thread is active for this client
        crate::set_foreground(foreground, None);
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadSwitched,
            payload: ServerPayload::ThreadSwitched {
                thread_id: 0,
                context_summary: None,
            },
        };
        send_frame(writer, &frame).await?;
        send_threads_update_shared(writer, thread_mgr, task_mgr, None, None).await?;
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadMessages,
            payload: ServerPayload::ThreadMessages {
                thread_id: 0,
                messages: Vec::new(),
            },
        };
        send_frame(writer, &frame).await?;
        // `None`, and recorded as such: leaving the old pointer on disk
        // would make the next connection reopen the very thread the user
        // just asked to be put down.
        crate::helpers::persist_threads_focused(thread_mgr, threads_path, None).await;
        return Ok(());
    }

    let target_id = ThreadId(thread_id);

    // Resolve what the id names before acting on its behalf. The compaction
    // below summarises the outgoing foreground and trims its context — a
    // cost only justified when the foreground is actually about to move.
    // Sub-agent/cron sidebar rows reach this handler with pseudo-ids that
    // are not threads: peeking at one must not burn a provider call or
    // shrink the active chat's memory.
    if thread_mgr.lock().await.get(target_id).is_none() {
        if let Some((_, messages)) = crate::thread_updates::session_transcript(thread_id) {
            // A read-only session preview. The client is told it "switched"
            // so it repaints onto this transcript; the manager's foreground
            // stays put, so a message typed here names the pseudo-id and is
            // refused by the vanished-thread guard rather than silently
            // filed elsewhere — and clicking any real thread switches back
            // normally. No thread-list frame follows: the client re-fetches
            // the foreground's history whenever one moves its pointer,
            // which would wipe this view moments after it appeared. The
            // read-only note is the transcript's own last message (see
            // `session_transcript`), so every repaint keeps it.
            let frame = ServerFrame {
                frame_type: ServerFrameType::ThreadSwitched,
                payload: ServerPayload::ThreadSwitched {
                    thread_id,
                    context_summary: None,
                },
            };
            send_frame(writer, &frame).await?;
            let frame = ServerFrame {
                frame_type: ServerFrameType::ThreadMessages,
                payload: ServerPayload::ThreadMessages {
                    thread_id,
                    messages,
                },
            };
            send_frame(writer, &frame).await?;
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
        return Ok(());
    }

    // Compact the outgoing thread if it has enough history to be worth
    // summarising. The prompt is taken under the lock and the summary
    // applied under it again; the provider round trip in between holds
    // nothing.
    //
    // Read before the decision below, not after it: with no model there is
    // nothing to summarise *with*, and a thread marked as being summarised
    // by a request that was never made stays marked for the life of the
    // process — never eligible again, its context growing without bound.
    let current_model_ctx = shared_model_ctx.read().await.clone();
    let to_compact = {
        // A write lock, because deciding to compact and marking the thread
        // as being compacted have to be one step. Between them is exactly
        // the window a second switch would use to start the same summary
        // again, and that window is open for a whole provider round trip
        // now that the call no longer blocks the loop.
        let mut tm = thread_mgr.lock().await;
        let candidate = current_model_ctx
            .as_ref()
            .and_then(|_| crate::foreground_of(foreground))
            .and_then(|fg| tm.get(fg))
            .map(|t| t.task_id())
            .filter(|fg_id| *fg_id != target_id && !busy_threads.contains(fg_id))
            .and_then(|fg_id| tm.get(fg_id))
            .filter(|thread| {
                thread.messages.len() > 3
                    && thread.compact_summary.is_none()
                    // Already being summarised: both conditions above stay
                    // true for the whole call, so this is what stops a
                    // second request being paid for and discarded.
                    && !thread.compacting
            })
            .map(|thread| {
                (
                    thread.id,
                    thread.label.clone(),
                    thread.compaction_prompt(),
                    // How much of the conversation this prompt describes.
                    // The summary is only true of these messages, and the
                    // thread can grow while it is being written.
                    thread.messages.len(),
                )
            });
        if let Some((id, ..)) = &candidate
            && let Some(thread) = tm.get_mut(*id)
        {
            thread.compacting = true;
        }
        candidate
    };
    if let Some((fg_id, label, prompt, covered)) = to_compact {
        // `to_compact` is only ever `Some` when there is a model, so this
        // cannot fail — but taking it by `if let` keeps the claim above and
        // the spawn below with nothing fallible in between. That is what
        // makes "marked as being summarised" and "a task exists to clear
        // the mark" the same condition rather than two that have to be kept
        // in step by hand.
        if let Some(ctx) = current_model_ctx {
            // Kept for the notice below, since the task takes the original.
            let notice_label = label.clone();
            let summary_req = ProviderRequest {
                messages: vec![ChatMessage::text("user", &prompt)],
                model: ctx.model.clone(),
                provider: ctx.provider.clone(),
                base_url: ctx.base_url.clone(),
                api_key: ctx.api_key.clone(),
                // Summarisation never needs tools.
                allowed_tools: Some(Vec::new()),
            };

            // Summarising is housekeeping on the thread being *left*, and
            // nothing the client is about to be sent depends on it: the
            // `ThreadSwitched` frame below carries the *target* thread's
            // summary, not this one. Awaiting it here bought nothing and
            // cost everything — a provider round trip inside the connection
            // loop, during which the connection answered nothing at all.
            // Not another thread's stream, not a Stop, not a thread being
            // opened. Opening a thread was therefore also the act of
            // freezing the session for as long as the provider took.
            //
            // So it runs on its own task and the switch returns immediately.
            let http = http.clone();
            let thread_mgr = thread_mgr.clone();
            tokio::spawn(async move {
                let resp = providers::call_with_tools(&http, &summary_req, None).await;
                let mut tm = thread_mgr.lock().await;
                let Some(thread) = tm.get_mut(fg_id) else {
                    // Closed while the summary was in flight; its marker went
                    // with it.
                    return;
                };
                // Cleared here and nowhere else, so no early return below can
                // leave a thread that can never be compacted again.
                thread.compacting = false;
                let resp = match resp {
                    Ok(resp) if !resp.text.is_empty() => resp,
                    Ok(_) => {
                        debug!(thread = %label, "Empty summary from LLM");
                        return;
                    }
                    Err(e) => {
                        debug!(thread = %label, error = %e, "Compaction failed");
                        return;
                    }
                };
                // Re-checked here, not just before the call. Compaction
                // trims the context it summarised, so applying it to a
                // thread that started a turn meanwhile would cut the ground
                // out from under a running answer — the caller's snapshot of
                // which threads were busy is a round trip old by now, and a
                // turn can start on this thread in that window.
                if thread.open_turn.is_some() {
                    debug!(thread = %label, "Thread started answering while it was being summarised; leaving it alone");
                    return;
                }
                // Another switch may have summarised it already.
                if thread.compact_summary.is_some() {
                    return;
                }
                // `apply_compaction` would put the boundary three messages
                // from the *current* end, and the thread may have grown
                // while the summary was being written — a whole turn can
                // start and finish in that window now that the loop is not
                // blocked, which clears `open_turn` again and slips past the
                // check above. Everything added since the prompt would then
                // sit behind the boundary, described by a summary that
                // predates it: silently invisible to the model while still
                // shown in the transcript.
                //
                // So the tail the summary does not cover is kept explicitly.
                // That is what `apply_compaction_keeping` is for — its own
                // doc says "when the caller knows exactly which tail of the
                // conversation the summary does *not* cover", and here we
                // do.
                thread.apply_compaction_keeping(
                    resp.text,
                    keep_after_compaction(covered, thread.messages.len()),
                );
                debug!(thread = %label, "Thread compacted");
                // Deliberately not persisted from here.
                //
                // `ThreadStore::persist` is a reconciliation, not an append:
                // it deletes the files of every thread the manager it is
                // handed does not contain. This manager belongs to the
                // connection that asked for the switch, and this task now
                // outlives it — the connection can be gone, or its agent
                // session reloaded, by the time the provider replies. A later
                // connection that created a thread meanwhile would have it
                // deleted from disk by this write, which is a far worse
                // outcome than the one being avoided.
                //
                // The summary is a cache: it is in memory for the context
                // that needs it, the next persist on a live manager writes it
                // out, and if it is lost the only cost is summarising again
                // on the next switch.
            });
            // Sent after the spawn rather than before it: this is the one
            // fallible step here, and a failed write must not be able to
            // leave a thread marked as being summarised with nothing running
            // to unmark it.
            send_info(writer, &format!("Compacting thread '{}'...", notice_label)).await?;
        }
    }

    // Perform the switch. Moving this connection's pointer, not the shared
    // manager's: the target is re-checked here rather than trusted from the
    // guard above, because the lock was released for the compaction round
    // trip and a close could have landed in between.
    let switched = {
        let tm = thread_mgr.lock().await;
        // Get summary of thread being switched to
        tm.get(target_id).map(|t| t.compact_summary.clone())
    };
    if let Some(context_summary) = switched {
        crate::set_foreground(foreground, Some(target_id));
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadSwitched,
            payload: ServerPayload::ThreadSwitched {
                thread_id,
                context_summary,
            },
        };
        send_frame(writer, &frame).await?;
        // Send updated thread list
        send_threads_update_shared(writer, thread_mgr, task_mgr, None, Some(target_id)).await?;
        send_thread_messages_update_shared(writer, target_id, thread_mgr).await?;
        // Persist thread state (includes compaction summary), recording the
        // switch as this connection's foreground. Durability is why it
        // happens here and not only at teardown: a process that is killed
        // never reaches teardown, and the user's last choice should survive
        // a crash the same as it did before the pointer moved off the store.
        crate::helpers::persist_threads_focused(thread_mgr, threads_path, Some(target_id)).await;
    } else {
        // Unreachable in practice — the thread existed above and nothing
        // here removes threads — but a racing close could still take it.
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
    thread_mgr: &crate::SharedThreadMgr,
    task_mgr: &SharedTaskManager,
    foreground: Option<ThreadId>,
) -> Result<()> {
    debug!("Thread list request");
    send_threads_update_shared(writer, thread_mgr, task_mgr, None, foreground).await?;
    if let Some(id) = foreground {
        send_thread_messages_update_shared(writer, id, thread_mgr).await?;
    }
    Ok(())
}

/// Handle a `ThreadHistoryRequest`: reply with one thread's full message log.
pub(crate) async fn handle_thread_history(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &crate::SharedThreadMgr,
    thread_id: u64,
) -> Result<()> {
    debug!("Thread history request: {}", thread_id);
    let target_id = ThreadId(thread_id);
    let tm = thread_mgr.lock().await;
    let (ok, messages, error) = match tm.get(target_id) {
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
                        // Threads persist as JSON and can hold a bare `Value`;
                        // the bincode wire cannot. Decode here, at the boundary.
                        tool_calls: m
                            .tool_calls
                            .as_ref()
                            .map(rustyclaw_core::gateway::ToolCallRecord::from_stored_json),
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
        // Not a thread — but sidebar rows for sub-agent and cron *sessions*
        // carry pseudo-ids, and a row whose count says "300 messages" must
        // not open onto nothing. Serve the session's transcript.
        None => match crate::thread_updates::session_transcript(thread_id) {
            Some((label, messages)) => {
                info!(
                    thread_id,
                    caption = %label,
                    message_count = messages.len(),
                    "Serving session transcript for sidebar row"
                );
                (true, messages, None)
            }
            None => (
                false,
                Vec::new(),
                Some(format!("Thread {} not found", thread_id)),
            ),
        },
    };
    drop(tm);
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
    thread_mgr: &crate::SharedThreadMgr,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    thread_id: u64,
    foreground: &crate::ForegroundCell,
) -> Result<()> {
    debug!("Thread close request: {}", thread_id);
    let task_id = ThreadId(thread_id);
    {
        let mut tm = thread_mgr.lock().await;
        tm.remove(task_id);
        // Persist thread state
        crate::helpers::persist_threads(&mut tm, threads_path);
        // Closing the thread this client was looking at hands its foreground
        // to another one. Elected per connection, and only when the closed
        // thread was *this* client's: a window that closes a thread another
        // window happens to be in must not move that window as well.
        if crate::foreground_of(foreground) == Some(task_id) {
            crate::set_foreground(foreground, tm.elect_foreground());
        }
    }
    let now_foreground = crate::foreground_of(foreground);
    // Send updated thread list
    send_threads_update_shared(writer, thread_mgr, task_mgr, None, now_foreground).await?;
    // …and follow up with the new foreground's history — otherwise the
    // client's sidebar highlight moves while its transcript still shows the
    // closed thread.
    if let Some(fg) = now_foreground {
        send_thread_messages_update_shared(writer, fg, thread_mgr).await?;
    }
    Ok(())
}

/// Handle a `ThreadUpdate`: set a thread's caption and working-directory
/// override in one edit.
///
/// `working_dir: None` clears the override, so the thread falls back to its
/// project's directory. An override directory is created if missing, matching
/// project creation — you may well want to point a thread at a directory you
/// are about to populate.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_thread_update(
    writer: &mut dyn transport::TransportWriter,
    config: &mut rustyclaw_core::config::Config,
    thread_mgr: &crate::SharedThreadMgr,
    project_mgr: &rustyclaw_core::projects::ProjectManager,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    thread_id: u64,
    label: String,
    working_dir: Option<std::path::PathBuf>,
    foreground: Option<ThreadId>,
) -> Result<()> {
    debug!(
        thread_id,
        label = %label,
        working_dir = ?working_dir,
        "Thread update request"
    );
    let id = ThreadId(thread_id);

    if thread_mgr.lock().await.get(id).is_none() {
        return send_error(writer, format!("Cannot edit thread {thread_id}: not found")).await;
    }

    let label = label.trim().to_string();
    if label.is_empty() {
        return send_error(writer, "Thread caption cannot be empty".to_string()).await;
    }

    // An empty path is an empty override, not a directory with no name —
    // treat it as "inherit from the project". Whitespace is left alone: the
    // client trims its text field, and a directory name that ends in a space
    // is legal, so trimming here could only corrupt a deliberate path.
    let working_dir = working_dir.filter(|d| !d.as_os_str().is_empty());

    let working_dir = match working_dir {
        Some(dir) => {
            match crate::helpers::prepare_workspace_dir(&dir, "thread's working directory") {
                Ok(dir) => Some(dir),
                Err(message) => return send_error(writer, message).await,
            }
        }
        None => None,
    };

    {
        let mut tm = thread_mgr.lock().await;
        tm.rename(id, &label);
        tm.set_working_dir(id, working_dir);
        crate::helpers::persist_threads(&mut tm, threads_path);

        // Editing the foreground thread's directory has to take effect right
        // away, otherwise the next tool call still runs in the old one. Only
        // when it is *this* connection's foreground: the workspace being
        // repointed is this connection's, so another window's thread being
        // edited is not its business.
        if foreground == Some(id) {
            crate::project_handler::repoint_workspace(config, project_mgr, &tm, foreground);
        }
    }

    send_threads_update_shared(writer, thread_mgr, task_mgr, None, foreground).await
}

/// Send an `Error` frame. Edits fail loudly: the client shows the reason
/// rather than silently reverting the dialog.
async fn send_error(writer: &mut dyn transport::TransportWriter, message: String) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::Error,
        payload: ServerPayload::Error { ok: false, message },
    };
    send_frame(writer, &frame).await
}

/// Handle a `ThreadRename`: relabel a thread and broadcast the new list.
pub(crate) async fn handle_thread_rename(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &crate::SharedThreadMgr,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    thread_id: u64,
    new_label: String,
    foreground: Option<ThreadId>,
) -> Result<()> {
    debug!("Thread rename request: {} -> {}", thread_id, new_label);
    let task_id = ThreadId(thread_id);
    let renamed = {
        let mut tm = thread_mgr.lock().await;
        let renamed = tm.rename(task_id, &new_label);
        if renamed {
            // Persist thread state
            crate::helpers::persist_threads(&mut tm, threads_path);
        }
        renamed
    };
    if renamed {
        // Send updated thread list
        send_threads_update_shared(writer, thread_mgr, task_mgr, None, foreground).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use rustyclaw_core::config::Config;
    use rustyclaw_core::projects::ProjectManager;
    use rustyclaw_core::threads::ThreadManager;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct CapturingWriter {
        frames: Vec<ServerFrame>,
    }

    #[async_trait]
    impl transport::TransportWriter for CapturingWriter {
        async fn send_on_stream(&mut self, _stream_id: u64, frame: &ServerFrame) -> Result<()> {
            self.frames.push(frame.clone());
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl CapturingWriter {
        fn errors(&self) -> Vec<String> {
            self.frames
                .iter()
                .filter_map(|f| match &f.payload {
                    ServerPayload::Error { message, .. } => Some(message.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    /// Fixture: one project with one foreground thread in it.
    fn fixture(
        tmp: &std::path::Path,
    ) -> (Config, ProjectManager, crate::SharedThreadMgr, ThreadId) {
        let config = Config {
            settings_dir: tmp.join("state"),
            ..Config::default()
        };

        let mut projects = ProjectManager::new();
        let api = projects.create("Api", tmp.join("api"));
        projects.set_active(api);

        let mut threads = ThreadManager::new();
        let id = threads.create_chat("Original");
        threads.set_project(id, api);

        (config, projects, Arc::new(Mutex::new(threads)), id)
    }

    /// A connection's foreground pointer, standing in for one client's view.
    fn fg_cell(id: Option<ThreadId>) -> crate::ForegroundCell {
        Arc::new(std::sync::RwLock::new(id))
    }

    #[tokio::test]
    async fn thread_update_sets_the_caption_and_override_and_repoints() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut config, projects, threads, id) = fixture(tmp.path());
        let task_mgr = std::sync::Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let threads_path = tmp.path().join("threads.json");
        let override_dir = tmp.path().join("worktree");
        let mut writer = CapturingWriter { frames: Vec::new() };

        handle_thread_update(
            &mut writer,
            &mut config,
            &threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "  Renamed  ".to_string(),
            Some(override_dir.clone()),
            Some(id),
        )
        .await
        .unwrap();

        assert!(writer.errors().is_empty(), "{:?}", writer.errors());
        assert_eq!(
            threads.lock().await.get(id).unwrap().label,
            "Renamed",
            "caption trimmed"
        );
        assert_eq!(
            threads.lock().await.get(id).unwrap().working_dir,
            Some(override_dir.clone())
        );
        assert!(override_dir.is_dir(), "the override directory is created");
        // The edited thread is the foreground one, so tools run there now.
        assert_eq!(config.workspace_dir(), override_dir);
        assert!(
            threads_path
                .parent()
                .unwrap()
                .join("threads")
                .join("state.json")
                .is_file(),
            "the edit is persisted to the per-thread store"
        );

        // Clearing the override hands the thread back to its project.
        handle_thread_update(
            &mut writer,
            &mut config,
            &threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "Renamed".to_string(),
            None,
            Some(id),
        )
        .await
        .unwrap();
        assert_eq!(threads.lock().await.get(id).unwrap().working_dir, None);
        assert_eq!(config.workspace_dir(), tmp.path().join("api"));
    }

    /// An empty path is an empty override, not a directory with no name.
    #[tokio::test]
    async fn blank_override_means_inherit() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut config, projects, threads, id) = fixture(tmp.path());
        let task_mgr = std::sync::Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let threads_path = tmp.path().join("threads.json");
        let mut writer = CapturingWriter { frames: Vec::new() };

        handle_thread_update(
            &mut writer,
            &mut config,
            &threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "Keep".to_string(),
            Some(std::path::PathBuf::new()),
            Some(id),
        )
        .await
        .unwrap();

        assert!(writer.errors().is_empty());
        assert_eq!(threads.lock().await.get(id).unwrap().working_dir, None);
    }

    /// A path that isn't valid UTF-8 is refused at the edit, naming the path,
    /// rather than surfacing later as an opaque encode failure with nothing to
    /// identify the culprit. (Unix-only: `OsStr` cannot be built from
    /// arbitrary bytes portably.)
    #[cfg(unix)]
    #[tokio::test]
    async fn non_utf8_override_is_refused_by_name() {
        use std::os::unix::ffi::OsStrExt;

        let tmp = tempfile::tempdir().unwrap();
        let (mut config, projects, threads, id) = fixture(tmp.path());
        let task_mgr = std::sync::Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let threads_path = tmp.path().join("threads.json");
        let mut writer = CapturingWriter { frames: Vec::new() };

        let bad = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfeweird"));
        handle_thread_update(
            &mut writer,
            &mut config,
            &threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "Keep".to_string(),
            Some(bad),
            Some(id),
        )
        .await
        .unwrap();

        assert_eq!(writer.errors().len(), 1);
        assert!(
            writer.errors()[0].contains("not valid UTF-8"),
            "{:?}",
            writer.errors()
        );
        assert_eq!(
            threads.lock().await.get(id).unwrap().working_dir,
            None,
            "a refused path is not stored"
        );
    }

    /// Rejections are reported, not swallowed: a bad edit has to come back as
    /// an error frame or the dialog silently reverts with no explanation.
    #[tokio::test]
    async fn invalid_edits_send_error_frames() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut config, projects, threads, id) = fixture(tmp.path());
        let task_mgr = std::sync::Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let threads_path = tmp.path().join("threads.json");

        let mut writer = CapturingWriter { frames: Vec::new() };
        handle_thread_update(
            &mut writer,
            &mut config,
            &threads,
            &projects,
            &task_mgr,
            &threads_path,
            9_999,
            "Ghost".to_string(),
            None,
            Some(id),
        )
        .await
        .unwrap();
        assert_eq!(writer.errors().len(), 1);
        assert!(
            writer.errors()[0].contains("not found"),
            "{:?}",
            writer.errors()
        );

        let mut writer = CapturingWriter { frames: Vec::new() };
        handle_thread_update(
            &mut writer,
            &mut config,
            &threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "   ".to_string(),
            None,
            Some(id),
        )
        .await
        .unwrap();
        assert_eq!(writer.errors().len(), 1);
        assert!(writer.errors()[0].contains("cannot be empty"));
        assert_eq!(
            threads.lock().await.get(id).unwrap().label,
            "Original",
            "a rejected edit changes nothing"
        );
    }

    /// A writer that needs the thread manager, standing in for the running
    /// turn: it takes the same lock at every persistence point.
    struct ThreadTouchingWriter {
        thread_mgr: crate::SharedThreadMgr,
        frames: usize,
    }

    #[async_trait]
    impl transport::TransportWriter for ThreadTouchingWriter {
        async fn send_on_stream(&mut self, _stream_id: u64, _frame: &ServerFrame) -> Result<()> {
            let _ = self.thread_mgr.lock().await.list_info();
            self.frames += 1;
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// Switching threads must not hold the thread lock across its client
    /// writes — or, in the compaction path, across the model call between
    /// them. A turn running in parallel takes that same lock at every
    /// persistence point, so a guard held for the length of this handler
    /// freezes the answer in flight for as long as the provider takes.
    #[tokio::test]
    async fn switching_threads_does_not_hold_the_thread_lock() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let threads_path = tmp.path().join("threads.json");

        let mut manager = ThreadManager::new();
        let outgoing = manager.create_chat("outgoing");
        let target = manager.create_chat("target");
        manager.switch_foreground(outgoing);
        let fg = fg_cell(Some(outgoing));
        // Enough history that the compaction path is taken.
        for i in 0..5 {
            manager.add_message(
                outgoing,
                rustyclaw_core::threads::MessageRole::User,
                format!("message {i}"),
            );
        }

        let thread_mgr: crate::SharedThreadMgr = Arc::new(Mutex::new(manager));
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        // No model context: the provider round trip is skipped, but every
        // lock scope and client write around it still runs.
        let shared_model_ctx: SharedModelCtx = Arc::new(tokio::sync::RwLock::new(None));
        let mut writer = ThreadTouchingWriter {
            thread_mgr: thread_mgr.clone(),
            frames: 0,
        };

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            handle_thread_switch(
                &mut writer,
                &thread_mgr,
                &task_mgr,
                &threads_path,
                &shared_model_ctx,
                &reqwest::Client::new(),
                target.0,
                &fg,
                &[],
            ),
        )
        .await
        .expect("switching threads must not deadlock against a running turn")
        .expect("the switch should succeed");

        assert!(writer.frames > 0, "the switch should have told the client");
        assert_eq!(crate::foreground_of(&fg), Some(target));
    }

    /// A thread with a turn in flight is not summarised out from under it:
    /// the summary would miss the answer being written and drop the
    /// messages that answer is building on.
    #[tokio::test]
    async fn a_thread_with_a_turn_running_is_not_compacted() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let threads_path = tmp.path().join("threads.json");

        let mut manager = ThreadManager::new();
        let busy = manager.create_chat("busy");
        let target = manager.create_chat("target");
        manager.switch_foreground(busy);
        let fg = fg_cell(Some(busy));
        for i in 0..5 {
            manager.add_message(
                busy,
                rustyclaw_core::threads::MessageRole::User,
                format!("message {i}"),
            );
        }

        let thread_mgr: crate::SharedThreadMgr = Arc::new(Mutex::new(manager));
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let shared_model_ctx: SharedModelCtx = Arc::new(tokio::sync::RwLock::new(None));
        let mut writer = CapturingWriter { frames: Vec::new() };

        handle_thread_switch(
            &mut writer,
            &thread_mgr,
            &task_mgr,
            &threads_path,
            &shared_model_ctx,
            &reqwest::Client::new(),
            target.0,
            &fg,
            &[busy],
        )
        .await
        .expect("the switch should succeed");

        assert!(
            !writer
                .frames
                .iter()
                .any(|f| format!("{:?}", f.payload).contains("Compacting")),
            "the busy thread must not be compacted mid-turn"
        );
    }

    /// Opening a sub-agent/cron session row is a read-only preview: the
    /// transcript is served and the manager's foreground — where the next
    /// typed message goes — must not move. No thread-list frame may follow
    /// the transcript: a client that sees the foreground pointer move
    /// re-fetches that thread's history and repaints, which would wipe the
    /// preview moments after it appeared. And no separate info frame: the
    /// read-only note lives inside the transcript, where repaints keep it.
    #[tokio::test]
    async fn a_session_row_is_a_preview_that_leaves_the_foreground_alone() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let threads_path = tmp.path().join("threads.json");

        let mut manager = ThreadManager::new();
        let chat = manager.create_chat("chat");
        manager.switch_foreground(chat);
        let fg = fg_cell(Some(chat));
        let thread_mgr: crate::SharedThreadMgr = Arc::new(Mutex::new(manager));
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let shared_model_ctx: SharedModelCtx = Arc::new(tokio::sync::RwLock::new(None));

        let key = {
            let mut mgr = rustyclaw_core::sessions::session_manager().lock().unwrap();
            let key = mgr.spawn_subagent("main", "preview task", Some("previewer".into()), None);
            let session = mgr.get_mut(&key).unwrap();
            session.add_message("assistant", "background progress");
            key
        };
        let pseudo_id = crate::thread_updates::session_row_id(&key);

        let mut writer = CapturingWriter { frames: Vec::new() };
        handle_thread_switch(
            &mut writer,
            &thread_mgr,
            &task_mgr,
            &threads_path,
            &shared_model_ctx,
            &reqwest::Client::new(),
            pseudo_id,
            &fg,
            &[],
        )
        .await
        .expect("viewing a session row should succeed");

        assert!(writer.errors().is_empty(), "{:?}", writer.errors());
        assert_eq!(
            crate::foreground_of(&fg),
            Some(chat),
            "a read-only preview must not move where the next message goes"
        );

        let kinds: Vec<ServerFrameType> = writer.frames.iter().map(|f| f.frame_type).collect();
        assert_eq!(
            kinds,
            vec![
                ServerFrameType::ThreadSwitched,
                ServerFrameType::ThreadMessages
            ],
            "a preview is the switch and the transcript, nothing else: a \
             thread-list frame makes the client re-fetch the real foreground \
             and wipe the view, and a bare info frame is wiped by the next \
             repaint"
        );
        let transcript = writer
            .frames
            .iter()
            .find_map(|f| match &f.payload {
                ServerPayload::ThreadMessages { messages, .. } => Some(messages.clone()),
                _ => None,
            })
            .expect("transcript payload");
        let note = transcript.last().expect("transcript is never empty");
        assert_eq!(
            note.role, "info",
            "the read-only note is the transcript's own last message, so \
             every repaint of this view redraws it"
        );
        assert!(
            note.content.contains("read-only"),
            "the note explains the view: {}",
            note.content
        );
    }

    /// Peeking at a session row must cost the active chat nothing. The
    /// compaction step summarises and trims the outgoing foreground on the
    /// assumption that the user is leaving it — but a session preview leaves
    /// the foreground exactly where it is, so reaching compaction on the way
    /// to one burned a provider call and shrank the live chat's context for
    /// a switch that never happens.
    #[tokio::test]
    async fn peeking_at_a_session_row_does_not_compact_the_chat() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let threads_path = tmp.path().join("threads.json");

        let mut manager = ThreadManager::new();
        let chat = manager.create_chat("chat");
        manager.switch_foreground(chat);
        let fg = fg_cell(Some(chat));
        // Enough history that a real switch away would compact it.
        for i in 0..5 {
            manager.add_message(
                chat,
                rustyclaw_core::threads::MessageRole::User,
                format!("message {i}"),
            );
        }
        let thread_mgr: crate::SharedThreadMgr = Arc::new(Mutex::new(manager));
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let shared_model_ctx: SharedModelCtx = Arc::new(tokio::sync::RwLock::new(None));

        let key = {
            let mut mgr = rustyclaw_core::sessions::session_manager().lock().unwrap();
            mgr.spawn_subagent("main", "peek task", Some("peeked".into()), None)
        };
        let pseudo_id = crate::thread_updates::session_row_id(&key);

        let mut writer = CapturingWriter { frames: Vec::new() };
        handle_thread_switch(
            &mut writer,
            &thread_mgr,
            &task_mgr,
            &threads_path,
            &shared_model_ctx,
            &reqwest::Client::new(),
            pseudo_id,
            &fg,
            &[],
        )
        .await
        .expect("viewing a session row should succeed");

        assert!(
            !writer
                .frames
                .iter()
                .any(|f| format!("{:?}", f.payload).contains("Compacting")),
            "a peek must not announce (or perform) compaction of the chat"
        );
        let tm = thread_mgr.lock().await;
        let thread = tm.get(chat).expect("the chat still exists");
        assert!(
            thread.compact_summary.is_none(),
            "the chat's context is untouched by a peek"
        );
        assert_eq!(thread.messages.len(), 5, "no messages were folded away");
    }

    /// A summary that lands after the thread has moved on must not bury the
    /// messages it never described.
    ///
    /// Compaction moves a boundary rather than deleting anything, so the
    /// damage is invisible in the transcript: the client still shows every
    /// message while the model stops being shown some of them. That is the
    /// failure this reproduces — a turn completes on the thread while the
    /// summary is in flight, and those messages must stay on the visible
    /// side of the boundary.
    #[test]
    fn a_late_summary_does_not_bury_messages_it_never_saw() {
        use rustyclaw_core::threads::MessageRole;
        let mut manager = rustyclaw_core::threads::ThreadManager::new();
        let id = manager.create_chat("busy");
        for i in 0..8 {
            manager.add_message(id, MessageRole::User, format!("q{i}"));
        }
        // What the prompt was built from.
        let covered = manager.get(id).expect("thread").messages.len();

        // A whole turn starts and finishes while the summary is written, so
        // `open_turn` is clear again by the time it lands.
        manager.begin_turn(id);
        manager.add_message(id, MessageRole::User, "asked while summarising");
        manager.add_message(id, MessageRole::Assistant, "answered while summarising");
        if let Some(t) = manager.get_mut(id) {
            t.end_turn(true);
        }

        let now = manager.get(id).expect("thread").messages.len();
        let keep = keep_after_compaction(covered, now);
        if let Some(t) = manager.get_mut(id) {
            t.apply_compaction_keeping("a summary of q0..q7".to_string(), keep);
        }

        let thread = manager.get(id).expect("thread");
        let visible: Vec<&str> = thread
            .context_messages()
            .map(|m| m.content.as_str())
            .collect();
        assert!(
            visible.contains(&"asked while summarising")
                && visible.contains(&"answered while summarising"),
            "messages added after the prompt are not in the summary, so they \
             must still be in context: {visible:?}"
        );
        // The boundary lands where the summary's coverage ends, wherever the
        // thread has grown to since.
        assert_eq!(thread.compacted_up_to, covered - COMPACT_KEEP_RECENT);
        assert_eq!(
            thread.messages.len(),
            now,
            "compaction moves a boundary; it deletes nothing"
        );
    }

    /// Without growth the rule is the plain one, so a quiet thread compacts
    /// exactly as before.
    #[test]
    fn an_undisturbed_thread_keeps_the_usual_tail() {
        assert_eq!(keep_after_compaction(10, 10), COMPACT_KEEP_RECENT);
        assert_eq!(keep_after_compaction(10, 14), COMPACT_KEEP_RECENT + 4);
        // A thread cannot shrink, but the arithmetic must not panic if it did.
        assert_eq!(keep_after_compaction(10, 4), COMPACT_KEEP_RECENT);
    }
}

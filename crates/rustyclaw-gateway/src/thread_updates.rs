//! Client-bound thread/session update frames.
//!
//! Helpers that serialize the gateway's thread list, per-thread history, and
//! active sub-agent sessions into [`ServerFrame`]s for the connected client.

use anyhow::Result;
use tracing::info;

use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, protocol, transport};

use crate::{SharedTaskManager, SharedThreadMgr};
use protocol::server::send_frame;

/// The sidebar pseudo-id for a session row.
///
/// Sessions are not threads — they have no `ThreadId` — but they share the
/// sidebar, so each gets a stable id derived from its key. Anything that
/// hands these ids to a client must also be able to resolve them back; see
/// [`session_transcript`]. Handing out ids nothing could resolve was how the
/// sidebar showed rows with hundreds of messages that opened empty.
pub(crate) fn session_row_id(key: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

/// Resolve a sidebar pseudo-id back to its session's label and transcript.
///
/// `None` when the id names no known session (or a real thread — callers try
/// the thread manager first). Finished sessions resolve too: a row clicked
/// moments after its cron run completed should still open.
pub(crate) fn session_transcript(id: u64) -> Option<(String, Vec<protocol::types::ChatMessage>)> {
    use rustyclaw_core::sessions::{SessionKind, session_manager};
    let sess_mgr = session_manager().lock().ok()?;
    let kinds = [SessionKind::Subagent, SessionKind::Cron];
    let session = sess_mgr
        .list(Some(&kinds), false, usize::MAX)
        .into_iter()
        .find(|s| session_row_id(&s.key) == id)?;
    let label = session
        .label
        .clone()
        .unwrap_or_else(|| "Sub-agent".to_string());
    let mut messages: Vec<protocol::types::ChatMessage> = session
        .messages
        .iter()
        .map(|m| protocol::types::ChatMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            id: None,
            tool_calls: None,
            tool_call_id: None,
            media: None,
        })
        .collect();
    // The read-only note travels inside the transcript, as its last message,
    // rather than as a separate info frame: clients repaint this view from
    // whichever of `ThreadMessages`/`ThreadHistory` arrives last, and a note
    // sent outside the transcript is wiped by the very next repaint. Inside
    // it, every repaint redraws it.
    messages.push(protocol::types::ChatMessage {
        role: "info".to_string(),
        content: format!(
            "'{label}' is a background session — this transcript is read-only. \
             Select one of your threads in the sidebar to continue chatting."
        ),
        id: None,
        tool_calls: None,
        tool_call_id: None,
        media: None,
    });
    Some((label, messages))
}

/// The client-facing view of the thread list, taken from the manager in one
/// synchronous pass.
///
/// Split out so callers holding the shared manager can take this snapshot,
/// release the lock, and only then write to the client. Holding the thread
/// lock across a frame write would deadlock a model task against the
/// connection loop the moment the frame channel filled up.
///
/// `foreground` is the *connection's* focused thread, not the manager's. The
/// manager is shared by every window open on the agent, so taking the
/// foreground from it puts one window's cursor in another window's frame —
/// and clients read a changed `foreground_id` as "switch conversation and
/// fetch its history", so the other window jumps.
fn thread_dtos(
    thread_mgr: &rustyclaw_core::threads::ThreadManager,
    foreground: Option<rustyclaw_core::threads::ThreadId>,
) -> (Vec<protocol::ThreadInfoDto>, Option<u64>) {
    let foreground_id = foreground
        .and_then(|id| thread_mgr.get(id))
        .map(|t| t.task_id().0);
    let threads = thread_mgr
        .list_info()
        .into_iter()
        .map(|mut t| {
            // Same reason: `is_foreground` on the thread is shared state.
            // This connection's answer is whether it is *this* client's
            // focused thread. Fixed up before the filter below, which lets a
            // still-empty thread through on exactly this flag — read from the
            // manager it would keep another window's new thread listed here
            // and drop this window's.
            t.is_foreground = Some(t.id) == foreground;
            t
        })
        // A thread with no messages has nothing to open: it is a row whose
        // click shows an empty transcript. Existence in the sidebar should
        // mean there is a conversation behind it — the foreground stays
        // (that is where the user is about to type) and so does anything
        // mid-turn (its first messages are being written right now).
        .filter(|t| t.message_count > 0 || t.is_foreground || t.status == "Streaming")
        .map(|t| protocol::ThreadInfoDto {
            id: t.id.0,
            label: t.label.clone(),
            description: t.description.clone(),
            status: Some(t.status.clone()),
            kind_icon: Some(t.icon.clone()),
            status_icon: Some(t.status_icon.clone()),
            is_foreground: t.is_foreground,
            message_count: t.message_count,
            has_summary: t.has_summary,
            project_id: t.project_id.0,
            working_dir: t.working_dir.clone(),
            pinned: t.pinned,
        })
        .collect();
    (threads, foreground_id)
}

/// Send a threads update frame to the client: chat threads, running tasks,
/// and active sub-agent sessions.
///
/// Takes the shared manager rather than a borrow so the snapshot is taken
/// under the lock and the lock released before anything is written. A
/// caller holding a guard across the write would block every turn running
/// in parallel — they take the same lock at each persistence point — for
/// as long as the transport takes.
pub(crate) async fn send_threads_update_shared(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &SharedThreadMgr,
    task_mgr: &SharedTaskManager,
    session_key: Option<&str>,
    foreground: Option<rustyclaw_core::threads::ThreadId>,
) -> Result<()> {
    let (threads, foreground_id) = {
        let tm = thread_mgr.lock().await;
        thread_dtos(&tm, foreground)
    };
    send_threads_update_from(writer, threads, foreground_id, task_mgr, session_key).await
}

/// The rest of a threads update, once the thread snapshot is in hand.
async fn send_threads_update_from(
    writer: &mut dyn transport::TransportWriter,
    mut threads: Vec<protocol::ThreadInfoDto>,
    foreground_id: Option<u64>,
    task_mgr: &SharedTaskManager,
    session_key: Option<&str>,
) -> Result<()> {
    use rustyclaw_core::sessions::{SessionKind, SessionStatus, session_manager};

    // Add running tasks from TaskManager
    let tasks = if let Some(sess) = session_key {
        task_mgr.for_session(sess).await
    } else {
        task_mgr.active().await
    };

    for task in tasks {
        // Skip terminal tasks older than 5 minutes
        if task.status.is_terminal() {
            if let Some(finished) = task.finished_at {
                if finished
                    .elapsed()
                    .map(|d| d.as_secs() > 300)
                    .unwrap_or(false)
                {
                    continue;
                }
            }
        }

        let status_icon = if task.status.is_terminal() {
            "✓"
        } else {
            "▶"
        };
        threads.push(protocol::ThreadInfoDto {
            id: task.id.0,
            label: task.kind.display_name().to_string(),
            description: Some(task.kind.description()),
            status: Some(format!("{:?}", task.status)),
            kind_icon: Some("📋".to_string()),
            status_icon: Some(status_icon.to_string()),
            is_foreground: false,
            message_count: 0,
            has_summary: task.status.is_terminal(),
            // Ephemeral tasks aren't bound to a project; the client buckets
            // project_id == 0 under the active project.
            project_id: 0,
            working_dir: None,
            pinned: false,
        });
    }

    // Add active sub-agent sessions
    if let Ok(sess_mgr) = session_manager().lock() {
        let subagent_kinds = [SessionKind::Subagent, SessionKind::Cron];
        let active_sessions = sess_mgr.list(Some(&subagent_kinds), true, 50);

        for session in active_sessions {
            // Stable pseudo-id derived from the session key; resolvable back
            // through `session_transcript` when the client opens the row.
            let id = session_row_id(&session.key);

            let status_str = match session.status {
                SessionStatus::Active => "Running",
                SessionStatus::Completed => "Completed",
                SessionStatus::Error => "Failed",
                SessionStatus::Timeout => "Timeout",
                SessionStatus::Stopped => "Stopped",
            };

            let session_status_icon = match session.status {
                SessionStatus::Active => "▶",
                SessionStatus::Completed => "✓",
                SessionStatus::Error => "✗",
                SessionStatus::Timeout => "⊘",
                SessionStatus::Stopped => "⊘",
            };
            threads.push(protocol::ThreadInfoDto {
                id,
                label: session
                    .label
                    .clone()
                    .unwrap_or_else(|| "Sub-agent".to_string()),
                description: session.task.clone(),
                status: Some(status_str.to_string()),
                kind_icon: Some("🤖".to_string()),
                status_icon: Some(session_status_icon.to_string()),
                is_foreground: false,
                message_count: session.messages.len(),
                has_summary: session.status != SessionStatus::Active,
                project_id: 0,
                working_dir: None,
                pinned: false,
            });
        }
    }

    info!(
        total_threads = threads.len(),
        foreground_id = ?foreground_id,
        captions = ?threads
            .iter()
            .map(|t| format!("{}:{}", t.id, t.label))
            .collect::<Vec<_>>(),
        "Sending ThreadsUpdate"
    );

    let frame = ServerFrame {
        frame_type: ServerFrameType::ThreadsUpdate,
        payload: ServerPayload::ThreadsUpdate {
            threads,
            foreground_id,
        },
    };

    send_frame(writer, &frame).await
}

/// Send a projects update frame (the full project list + active project).
pub(crate) async fn send_projects_update(
    writer: &mut dyn transport::TransportWriter,
    project_mgr: &rustyclaw_core::projects::ProjectManager,
) -> Result<()> {
    let projects = project_mgr
        .list_info()
        .into_iter()
        .map(|p| protocol::ProjectInfoDto {
            id: p.id.0,
            name: p.name,
            path: p.path,
            pinned: p.pinned,
        })
        .collect();
    let frame = ServerFrame {
        frame_type: ServerFrameType::ProjectsUpdate,
        payload: ServerPayload::ProjectsUpdate {
            projects,
            active_id: project_mgr.active_id().0,
        },
    };
    send_frame(writer, &frame).await
}

pub(crate) fn thread_history_messages(
    thread: &rustyclaw_core::threads::AgentThread,
) -> Vec<protocol::types::ChatMessage> {
    thread
        .messages
        .iter()
        .map(|message| {
            let role = match message.role {
                rustyclaw_core::threads::MessageRole::User => "user",
                rustyclaw_core::threads::MessageRole::Assistant => "assistant",
                rustyclaw_core::threads::MessageRole::System => "system",
                rustyclaw_core::threads::MessageRole::Tool => "tool",
            };
            protocol::types::ChatMessage {
                role: role.to_string(),
                content: message.content.clone(),
                id: message.id.clone(),
                // Decoded here rather than passed through: see
                // `ToolCallRecord`. A `Value` on the bincode wire encodes and
                // then cannot be decoded, and this is the other frame that
                // carries a transcript.
                tool_calls: message
                    .tool_calls
                    .as_ref()
                    .map(rustyclaw_core::gateway::ToolCallRecord::from_stored_json),
                tool_call_id: message.tool_call_id.clone(),
                media: None,
            }
        })
        .collect()
}

/// Send one thread's transcript, on the same snapshot-then-write terms as
/// [`send_threads_update_shared`].
pub(crate) async fn send_thread_messages_update_shared(
    writer: &mut dyn transport::TransportWriter,
    thread_id: rustyclaw_core::threads::ThreadId,
    thread_mgr: &SharedThreadMgr,
) -> Result<()> {
    let messages = {
        let tm = thread_mgr.lock().await;
        tm.get(thread_id).map(thread_history_messages)
    };
    send_thread_messages_from(writer, thread_id, messages).await
}

/// Send a thread's transcript. `None` means the thread is not in this
/// session's manager at all, which is worth saying out loud.
async fn send_thread_messages_from(
    writer: &mut dyn transport::TransportWriter,
    thread_id: rustyclaw_core::threads::ThreadId,
    messages: Option<Vec<protocol::types::ChatMessage>>,
) -> Result<()> {
    // `unwrap_or_default()` here used to make "this thread does not exist"
    // indistinguishable from "this thread has no messages": both sent an
    // empty list, and `ThreadMessages` carries no status field, so the client
    // rendered a blank transcript either way with nothing to report. The
    // lookup failure is at least logged now — it means the client is showing
    // a thread this session's ThreadManager has never heard of.
    let messages = match messages {
        Some(messages) => messages,
        None => {
            tracing::error!(
                thread_id = thread_id.0,
                "Thread history requested for a thread this session does not have; \
                 sending an empty transcript"
            );
            Vec::new()
        }
    };
    if messages.is_empty() {
        tracing::info!(
            thread_id = thread_id.0,
            "Sending empty thread history to client"
        );
    }
    let frame = ServerFrame {
        frame_type: ServerFrameType::ThreadMessages,
        payload: ServerPayload::ThreadMessages {
            thread_id: thread_id.0,
            messages,
        },
    };

    send_frame(writer, &frame).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// A writer that needs the thread manager itself — standing in for the
    /// connection loop, which locks the manager to answer client frames while
    /// a model task is streaming.
    struct ThreadTouchingWriter {
        thread_mgr: SharedThreadMgr,
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

    fn shared_manager() -> SharedThreadMgr {
        Arc::new(Mutex::new(rustyclaw_core::threads::ThreadManager::default()))
    }

    /// The shared-handle senders must release the lock before they write.
    /// Holding it across the write deadlocks a model task against the
    /// connection loop as soon as the frame channel backs up — the failure
    /// mode is a wedged gateway, so it is worth a test that simply never
    /// finishes if the lock is held too long.
    #[tokio::test]
    async fn a_threads_update_does_not_hold_the_thread_lock_while_writing() {
        let thread_mgr = shared_manager();
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::default());
        let mut writer = ThreadTouchingWriter {
            thread_mgr: thread_mgr.clone(),
            frames: 0,
        };

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            send_threads_update_shared(&mut writer, &thread_mgr, &task_mgr, None, None),
        )
        .await
        .expect("sending a threads update must not deadlock on the thread lock")
        .expect("send should succeed");

        assert_eq!(writer.frames, 1);
    }

    /// Same for the per-thread transcript sender.
    #[tokio::test]
    async fn a_messages_update_does_not_hold_the_thread_lock_while_writing() {
        let thread_mgr = shared_manager();
        let mut writer = ThreadTouchingWriter {
            thread_mgr: thread_mgr.clone(),
            frames: 0,
        };

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            send_thread_messages_update_shared(
                &mut writer,
                rustyclaw_core::threads::ThreadId(1),
                &thread_mgr,
            ),
        )
        .await
        .expect("sending a transcript must not deadlock on the thread lock")
        .expect("send should succeed");

        assert_eq!(writer.frames, 1);
    }

    #[test]
    fn empty_threads_stay_out_of_the_sidebar() {
        let mut tm = rustyclaw_core::threads::ThreadManager::new();
        let foreground = tm.create_chat("Fresh");
        let abandoned = tm.create_chat("Abandoned");
        let used = tm.create_chat("Used");
        if let Some(t) = tm.get_mut(used) {
            t.add_message(rustyclaw_core::threads::MessageRole::User, "hello");
        }
        tm.switch_foreground(foreground);

        // The connection's foreground, which is what the filter now consults.
        let (threads, _) = thread_dtos(&tm, Some(foreground));
        let listed: Vec<u64> = threads.iter().map(|t| t.id).collect();
        assert!(
            listed.contains(&foreground.0),
            "the foreground is where the user types next; it stays even when empty"
        );
        assert!(
            listed.contains(&used.0),
            "a thread with messages is a conversation and must be listed"
        );
        assert!(
            !listed.contains(&abandoned.0),
            "an empty background thread is a row that opens onto nothing"
        );
    }

    #[test]
    fn a_session_row_id_resolves_back_to_its_transcript() {
        use rustyclaw_core::sessions::session_manager;
        let key = {
            let mut mgr = session_manager().lock().unwrap();
            let key = mgr.spawn_subagent(
                "main",
                "long research task",
                Some("researcher".into()),
                None,
            );
            let session = mgr.get_mut(&key).unwrap();
            session.add_message("user", "start digging");
            session.add_message("assistant", "found three leads");
            key
        };

        let (label, messages) = session_transcript(session_row_id(&key))
            .expect("the id the sidebar hands out must resolve to the transcript behind it");
        assert_eq!(label, "researcher");
        assert_eq!(
            messages.len(),
            3,
            "the session's two messages plus the trailing read-only note"
        );
        assert_eq!(messages[1].content, "found three leads");
        assert_eq!(
            messages[2].role, "info",
            "the note rides inside the transcript so repaints keep it"
        );

        assert!(
            session_transcript(0xDEAD_BEEF).is_none(),
            "an unknown id is not a session"
        );
    }
}

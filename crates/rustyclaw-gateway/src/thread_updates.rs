//! Client-bound thread/session update frames.
//!
//! Helpers that serialize the gateway's thread list, per-thread history, and
//! active sub-agent sessions into [`ServerFrame`]s for the connected client.

use anyhow::Result;
use tracing::info;

use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, protocol, transport};

use crate::{SharedTaskManager, SharedThreadMgr};
use protocol::server::send_frame;

/// The client-facing view of the thread list, taken from the manager in one
/// synchronous pass.
///
/// Split out so callers holding the shared manager can take this snapshot,
/// release the lock, and only then write to the client. Holding the thread
/// lock across a frame write would deadlock a model task against the
/// connection loop the moment the frame channel filled up.
fn thread_dtos(
    thread_mgr: &rustyclaw_core::threads::ThreadManager,
) -> (Vec<protocol::ThreadInfoDto>, Option<u64>) {
    let foreground_id = thread_mgr.foreground().map(|t| t.task_id().0);
    let threads = thread_mgr
        .list_info()
        .iter()
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
) -> Result<()> {
    let (threads, foreground_id) = {
        let tm = thread_mgr.lock().await;
        thread_dtos(&tm)
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
        });
    }

    // Add active sub-agent sessions
    if let Ok(sess_mgr) = session_manager().lock() {
        let subagent_kinds = [SessionKind::Subagent, SessionKind::Cron];
        let active_sessions = sess_mgr.list(Some(&subagent_kinds), true, 50);

        for session in active_sessions {
            // Generate a unique ID based on session key hash
            let id = {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                session.key.hash(&mut hasher);
                hasher.finish()
            };

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
                tool_calls: message.tool_calls.clone(),
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
            send_threads_update_shared(&mut writer, &thread_mgr, &task_mgr, None),
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
}

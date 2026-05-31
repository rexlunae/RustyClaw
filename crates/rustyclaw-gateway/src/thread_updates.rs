//! Client-bound thread/session update frames.
//!
//! Helpers that serialize the gateway's thread list, per-thread history, and
//! active sub-agent sessions into [`ServerFrame`]s for the connected client.

use anyhow::Result;
use tracing::info;

use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, protocol, transport};

use crate::SharedTaskManager;
use protocol::server::send_frame;

/// Send a threads update frame to the client.
///
/// This includes:
/// - Chat threads (from ThreadManager)
/// - Running tasks (from TaskManager)
/// - Active sub-agent sessions (from SessionManager)
pub(crate) async fn send_threads_update(
    writer: &mut dyn transport::TransportWriter,
    thread_mgr: &rustyclaw_core::threads::ThreadManager,
    task_mgr: &SharedTaskManager,
    session_key: Option<&str>,
) -> Result<()> {
    use rustyclaw_core::sessions::{SessionKind, SessionStatus, session_manager};

    let thread_list = thread_mgr.list_info();
    let foreground_id = thread_mgr.foreground().map(|t| t.task_id().0);

    // Collect chat threads
    let mut threads: Vec<protocol::ThreadInfoDto> = thread_list
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
        })
        .collect();

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
            protocol::types::ChatMessage::text(role, &message.content)
        })
        .collect()
}

pub(crate) async fn send_thread_messages_update(
    writer: &mut dyn transport::TransportWriter,
    thread_id: rustyclaw_core::threads::ThreadId,
    thread_mgr: &rustyclaw_core::threads::ThreadManager,
) -> Result<()> {
    let messages = thread_mgr
        .get(thread_id)
        .map(thread_history_messages)
        .unwrap_or_default();
    let frame = ServerFrame {
        frame_type: ServerFrameType::ThreadMessages,
        payload: ServerPayload::ThreadMessages {
            thread_id: thread_id.0,
            messages,
        },
    };

    send_frame(writer, &frame).await
}

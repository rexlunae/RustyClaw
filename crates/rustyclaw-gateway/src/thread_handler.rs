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
    crate::helpers::persist_threads(thread_mgr, threads_path);
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
        crate::helpers::persist_threads(thread_mgr, threads_path);
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

                        let summary_result =
                            providers::call_with_tools(http, &summary_req, None).await;

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
        crate::helpers::persist_threads(thread_mgr, threads_path);
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
    // Closing the foreground thread hands the foreground to another one, so
    // follow up with that thread's history — otherwise the client's sidebar
    // highlight moves while its transcript still shows the closed thread.
    if let Some(fg) = thread_mgr.foreground().map(|t| t.id) {
        send_thread_messages_update(writer, fg, thread_mgr).await?;
    }
    // Persist thread state
    crate::helpers::persist_threads(thread_mgr, threads_path);
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
    thread_mgr: &mut rustyclaw_core::threads::ThreadManager,
    project_mgr: &rustyclaw_core::projects::ProjectManager,
    task_mgr: &SharedTaskManager,
    threads_path: &std::path::Path,
    thread_id: u64,
    label: String,
    working_dir: Option<std::path::PathBuf>,
) -> Result<()> {
    debug!(
        thread_id,
        label = %label,
        working_dir = ?working_dir,
        "Thread update request"
    );
    let id = ThreadId(thread_id);

    if thread_mgr.get(id).is_none() {
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

    if let Some(ref dir) = working_dir {
        if let Err(message) = crate::helpers::reject_non_utf8_path(dir) {
            return send_error(writer, message).await;
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            return send_error(
                writer,
                format!(
                    "Could not use '{}' as the thread's working directory: {e}",
                    dir.display()
                ),
            )
            .await;
        }
    }

    thread_mgr.rename(id, &label);
    thread_mgr.set_working_dir(id, working_dir);
    crate::helpers::persist_threads(thread_mgr, threads_path);

    // Editing the foreground thread's directory has to take effect right away,
    // otherwise the next tool call still runs in the old one.
    if thread_mgr.foreground_id() == Some(id) {
        crate::project_handler::repoint_workspace(config, project_mgr, thread_mgr);
    }

    send_threads_update(writer, thread_mgr, task_mgr, None).await
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
        crate::helpers::persist_threads(thread_mgr, threads_path);
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
    fn fixture(tmp: &std::path::Path) -> (Config, ProjectManager, ThreadManager, ThreadId) {
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

        (config, projects, threads, id)
    }

    #[tokio::test]
    async fn thread_update_sets_the_caption_and_override_and_repoints() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut config, projects, mut threads, id) = fixture(tmp.path());
        let task_mgr = std::sync::Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let threads_path = tmp.path().join("threads.json");
        let override_dir = tmp.path().join("worktree");
        let mut writer = CapturingWriter { frames: Vec::new() };

        handle_thread_update(
            &mut writer,
            &mut config,
            &mut threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "  Renamed  ".to_string(),
            Some(override_dir.clone()),
        )
        .await
        .unwrap();

        assert!(writer.errors().is_empty(), "{:?}", writer.errors());
        assert_eq!(threads.get(id).unwrap().label, "Renamed", "caption trimmed");
        assert_eq!(
            threads.get(id).unwrap().working_dir,
            Some(override_dir.clone())
        );
        assert!(override_dir.is_dir(), "the override directory is created");
        // The edited thread is the foreground one, so tools run there now.
        assert_eq!(config.workspace_dir(), override_dir);
        assert!(threads_path.is_file(), "the edit is persisted");

        // Clearing the override hands the thread back to its project.
        handle_thread_update(
            &mut writer,
            &mut config,
            &mut threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "Renamed".to_string(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(threads.get(id).unwrap().working_dir, None);
        assert_eq!(config.workspace_dir(), tmp.path().join("api"));
    }

    /// An empty path is an empty override, not a directory with no name.
    #[tokio::test]
    async fn blank_override_means_inherit() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut config, projects, mut threads, id) = fixture(tmp.path());
        let task_mgr = std::sync::Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let threads_path = tmp.path().join("threads.json");
        let mut writer = CapturingWriter { frames: Vec::new() };

        handle_thread_update(
            &mut writer,
            &mut config,
            &mut threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "Keep".to_string(),
            Some(std::path::PathBuf::new()),
        )
        .await
        .unwrap();

        assert!(writer.errors().is_empty());
        assert_eq!(threads.get(id).unwrap().working_dir, None);
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
        let (mut config, projects, mut threads, id) = fixture(tmp.path());
        let task_mgr = std::sync::Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let threads_path = tmp.path().join("threads.json");
        let mut writer = CapturingWriter { frames: Vec::new() };

        let bad = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfeweird"));
        handle_thread_update(
            &mut writer,
            &mut config,
            &mut threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "Keep".to_string(),
            Some(bad),
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
            threads.get(id).unwrap().working_dir,
            None,
            "a refused path is not stored"
        );
    }

    /// Rejections are reported, not swallowed: a bad edit has to come back as
    /// an error frame or the dialog silently reverts with no explanation.
    #[tokio::test]
    async fn invalid_edits_send_error_frames() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut config, projects, mut threads, id) = fixture(tmp.path());
        let task_mgr = std::sync::Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let threads_path = tmp.path().join("threads.json");

        let mut writer = CapturingWriter { frames: Vec::new() };
        handle_thread_update(
            &mut writer,
            &mut config,
            &mut threads,
            &projects,
            &task_mgr,
            &threads_path,
            9_999,
            "Ghost".to_string(),
            None,
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
            &mut threads,
            &projects,
            &task_mgr,
            &threads_path,
            id.0,
            "   ".to_string(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(writer.errors().len(), 1);
        assert!(writer.errors()[0].contains("cannot be empty"));
        assert_eq!(
            threads.get(id).unwrap().label,
            "Original",
            "a rejected edit changes nothing"
        );
    }
}

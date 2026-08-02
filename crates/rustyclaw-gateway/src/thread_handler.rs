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
        crate::helpers::persist_threads(&tm, threads_path);
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
    // Send updated thread list
    send_threads_update_shared(writer, thread_mgr, task_mgr, None).await?;
    Ok(())
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
        // Clear foreground — no thread is active
        thread_mgr.lock().await.clear_foreground();
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadSwitched,
            payload: ServerPayload::ThreadSwitched {
                thread_id: 0,
                context_summary: None,
            },
        };
        send_frame(writer, &frame).await?;
        send_threads_update_shared(writer, thread_mgr, task_mgr, None).await?;
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadMessages,
            payload: ServerPayload::ThreadMessages {
                thread_id: 0,
                messages: Vec::new(),
            },
        };
        send_frame(writer, &frame).await?;
        crate::helpers::persist_threads(&*thread_mgr.lock().await, threads_path);
        return Ok(());
    }

    let target_id = ThreadId(thread_id);

    // Compact the outgoing thread if it has enough history to be worth
    // summarising. The prompt is taken under the lock and the summary
    // applied under it again; the provider round trip in between holds
    // nothing.
    let to_compact = {
        let tm = thread_mgr.lock().await;
        tm.foreground()
            .map(|t| t.task_id())
            .filter(|fg_id| *fg_id != target_id && !busy_threads.contains(fg_id))
            .and_then(|fg_id| tm.get(fg_id))
            .filter(|thread| thread.messages.len() > 3 && thread.compact_summary.is_none())
            .map(|thread| (thread.id, thread.label.clone(), thread.compaction_prompt()))
    };
    if let Some((fg_id, label, prompt)) = to_compact {
        // Notify client about compaction
        send_info(writer, &format!("Compacting thread '{}'...", label)).await?;

        let current_model_ctx = shared_model_ctx.read().await.clone();
        if let Some(ref ctx) = current_model_ctx {
            let summary_req = ProviderRequest {
                messages: vec![ChatMessage::text("user", &prompt)],
                model: ctx.model.clone(),
                provider: ctx.provider.clone(),
                base_url: ctx.base_url.clone(),
                api_key: ctx.api_key.clone(),
                // Summarisation never needs tools.
                allowed_tools: Some(Vec::new()),
            };

            match providers::call_with_tools(http, &summary_req, None).await {
                Ok(resp) if !resp.text.is_empty() => {
                    // Scoped explicitly: a guard taken in an `if let`
                    // scrutinee outlives the block, and this mutex is not
                    // reentrant.
                    let mut tm = thread_mgr.lock().await;
                    if let Some(thread) = tm.get_mut(fg_id) {
                        thread.apply_compaction(resp.text);
                        debug!(thread = %label, "Thread compacted");
                    }
                }
                Ok(_) => {
                    debug!(thread = %label, "Empty summary from LLM");
                }
                Err(e) => {
                    debug!(thread = %label, error = %e, "Compaction failed");
                }
            }
        }
    }

    // Perform the switch (use switch_foreground which returns bool,
    // not switch_to which returns old foreground ID — the latter
    // returns None when there is no previous foreground, e.g. after /thread bg)
    let switched = {
        let mut tm = thread_mgr.lock().await;
        // Get summary of thread being switched to
        let context_summary = tm.get(target_id).and_then(|t| t.compact_summary.clone());
        tm.switch_foreground(target_id).then_some(context_summary)
    };
    if let Some(context_summary) = switched {
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadSwitched,
            payload: ServerPayload::ThreadSwitched {
                thread_id,
                context_summary,
            },
        };
        send_frame(writer, &frame).await?;
        // Send updated thread list
        send_threads_update_shared(writer, thread_mgr, task_mgr, None).await?;
        send_thread_messages_update_shared(writer, target_id, thread_mgr).await?;
        // Persist thread state (includes compaction summary)
        crate::helpers::persist_threads(&*thread_mgr.lock().await, threads_path);
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
    thread_mgr: &crate::SharedThreadMgr,
    task_mgr: &SharedTaskManager,
) -> Result<()> {
    debug!("Thread list request");
    send_threads_update_shared(writer, thread_mgr, task_mgr, None).await?;
    let fg_id = thread_mgr.lock().await.foreground().map(|t| t.id);
    if let Some(id) = fg_id {
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
) -> Result<()> {
    debug!("Thread close request: {}", thread_id);
    let task_id = ThreadId(thread_id);
    let foreground = {
        let mut tm = thread_mgr.lock().await;
        tm.remove(task_id);
        // Persist thread state
        crate::helpers::persist_threads(&tm, threads_path);
        tm.foreground().map(|t| t.id)
    };
    // Send updated thread list
    send_threads_update_shared(writer, thread_mgr, task_mgr, None).await?;
    // Closing the foreground thread hands the foreground to another one, so
    // follow up with that thread's history — otherwise the client's sidebar
    // highlight moves while its transcript still shows the closed thread.
    if let Some(fg) = foreground {
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
        crate::helpers::persist_threads(&tm, threads_path);

        // Editing the foreground thread's directory has to take effect right
        // away, otherwise the next tool call still runs in the old one.
        if tm.foreground_id() == Some(id) {
            crate::project_handler::repoint_workspace(config, project_mgr, &tm);
        }
    }

    send_threads_update_shared(writer, thread_mgr, task_mgr, None).await
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
) -> Result<()> {
    debug!("Thread rename request: {} -> {}", thread_id, new_label);
    let task_id = ThreadId(thread_id);
    let renamed = {
        let mut tm = thread_mgr.lock().await;
        let renamed = tm.rename(task_id, &new_label);
        if renamed {
            // Persist thread state
            crate::helpers::persist_threads(&tm, threads_path);
        }
        renamed
    };
    if renamed {
        // Send updated thread list
        send_threads_update_shared(writer, thread_mgr, task_mgr, None).await?;
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
        assert!(threads_path.is_file(), "the edit is persisted");

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
                &[],
            ),
        )
        .await
        .expect("switching threads must not deadlock against a running turn")
        .expect("the switch should succeed");

        assert!(writer.frames > 0, "the switch should have told the client");
        assert_eq!(thread_mgr.lock().await.foreground_id(), Some(target));
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
}

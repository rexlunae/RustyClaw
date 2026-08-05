//! Agent client-frame handlers (list / switch / create / delete).
//!
//! An installation can host multiple agents (see
//! [`rustyclaw_core::agents::AgentRegistry`]). Each connection has one
//! *active* agent; switching swaps in that agent's thread and project
//! managers (persisted under `<settings_dir>/agents/<id>/sessions/`) and
//! repoints the workspace so tools and the system prompt reflect the
//! selected agent.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

use rustyclaw_core::agents::MAIN_AGENT_ID;
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, transport};
use rustyclaw_core::projects::ProjectManager;

use crate::project_handler;
use crate::thread_updates::{send_projects_update, send_threads_update_shared};
use crate::{SharedTaskManager, SharedThreadMgr};

/// Everything about the connection that is scoped to one agent. Swapped
/// wholesale on agent switch.
pub(crate) struct AgentSession {
    pub agent_id: String,
    pub threads_path: PathBuf,
    pub projects_path: PathBuf,
    /// Shared with the running model task — see [`crate::SharedThreadMgr`].
    pub thread_mgr: SharedThreadMgr,
    pub project_mgr: ProjectManager,
    /// The thread *this connection* is looking at.
    ///
    /// Per-connection, deliberately. A foreground is a statement about one
    /// client's view — which transcript is on screen, where the next typed
    /// message goes — and the gateway serves several clients per agent: a
    /// desktop window and a TUI, or two desktop windows. The manager beneath
    /// them is shared (see [`rustyclaw_core::threads::manager_for`]), so a
    /// foreground kept *there* is one pointer for all of them: one window
    /// opening a thread drags the others onto it, because clients treat a
    /// changed `foreground_id` as an instruction to switch conversation and
    /// fetch its history.
    ///
    /// `None` means nothing is focused — either the connection has not
    /// settled on a thread yet, or the client asked to background the
    /// current one. Both resolve the same way at the moment a thread is
    /// actually needed: see [`Self::ensure_foreground`].
    ///
    /// [`rustyclaw_core::threads::ThreadManager`] keeps its own foreground
    /// for the CLI, which is genuinely single-viewer, and as the persisted
    /// "where this agent was last left" hint a new connection starts from.
    ///
    /// Shared with this agent's running turns, which report sidebar updates
    /// and must read the *current* value — see [`crate::ForegroundCell`].
    pub foreground: crate::ForegroundCell,
}

impl AgentSession {
    /// Load an agent's persisted thread/project state, creating the
    /// directory skeleton on first use.
    pub fn load(config: &Config, agent_id: &str) -> Self {
        let sessions_dir = config.sessions_dir_for(agent_id);
        let _ = std::fs::create_dir_all(&sessions_dir);
        let threads_path = sessions_dir.join("threads.json");
        let projects_path = sessions_dir.join("projects.json");
        let thread_mgr = rustyclaw_core::threads::manager_for(&threads_path);
        let mut project_mgr = ProjectManager::load_or_new(&projects_path);
        project_mgr.ensure_default(config.workspace_dir_for(agent_id));
        crate::helpers::persist_projects(&project_mgr, &projects_path);
        Self {
            agent_id: agent_id.to_string(),
            threads_path,
            projects_path,
            thread_mgr,
            project_mgr,
            // Seeded by `restore_foreground`, which needs the manager lock
            // and so cannot run here. Until then nothing is focused, which
            // is the honest answer for a connection that has not asked for
            // a thread yet.
            foreground: Default::default(),
        }
    }

    /// Adopt the agent's persisted foreground as this connection's, so a
    /// window opens where the agent was last left rather than wherever the
    /// election rule happens to land.
    ///
    /// Only meaningful as the *initial* value: from here the connection's
    /// pointer moves on its own and the manager's is not consulted again.
    /// A stale persisted id — the thread was closed by another window — is
    /// ignored, leaving the election to [`Self::ensure_foreground`].
    pub async fn restore_foreground(&self) {
        let tm = self.thread_mgr.lock().await;
        let restored = tm.foreground_id().filter(|id| tm.get(*id).is_some());
        drop(tm);
        crate::set_foreground(&self.foreground, restored);
    }

    /// The thread this connection is looking at, if any.
    pub fn foreground_id(&self) -> Option<rustyclaw_core::threads::ThreadId> {
        crate::foreground_of(&self.foreground)
    }

    /// Focus `id`, if it names a thread that still exists. Returns whether it
    /// did — callers report a vanished thread rather than silently focusing
    /// something else.
    pub async fn switch_foreground(&self, id: rustyclaw_core::threads::ThreadId) -> bool {
        if self.thread_mgr.lock().await.get(id).is_none() {
            return false;
        }
        crate::set_foreground(&self.foreground, Some(id));
        true
    }

    /// The thread a turn from this connection should be filed under, electing
    /// one when nothing is focused or the focused thread has gone away.
    ///
    /// `None` only when the agent has no threads at all. Election reads the
    /// shared manager but does not move it, so filing a turn here cannot
    /// disturb another window's view.
    pub async fn ensure_foreground(&self) -> Option<rustyclaw_core::threads::ThreadId> {
        let tm = self.thread_mgr.lock().await;
        if let Some(id) = self.foreground_id()
            && tm.get(id).is_some()
        {
            return Some(id);
        }
        let elected = tm.elect_foreground();
        drop(tm);
        crate::set_foreground(&self.foreground, elected);
        elected
    }

    /// Persist thread and project state.
    ///
    /// Hands this connection's foreground to the manager on the way out, so
    /// the agent reopens where this window actually was. Quietly: the other
    /// windows on this agent are watching the manager's events and have their
    /// own view to keep.
    pub async fn save(&self) {
        let mut tm = self.thread_mgr.lock().await;
        if let Some(id) = self.foreground_id() {
            tm.set_foreground_quietly(Some(id));
        }
        crate::helpers::persist_threads(&mut tm, &self.threads_path);
        drop(tm);
        crate::helpers::persist_projects(&self.project_mgr, &self.projects_path);
    }
}

async fn send_error(writer: &mut dyn transport::TransportWriter, message: String) -> Result<()> {
    send_frame(
        writer,
        &ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error { ok: false, message },
        },
    )
    .await
}

/// Broadcast the agent list plus this connection's active agent.
pub(crate) async fn send_agents_update(
    writer: &mut dyn transport::TransportWriter,
    config: &Config,
    active_id: &str,
) -> Result<()> {
    let agents = config
        .agent_registry()
        .list()
        .into_iter()
        .map(Into::into)
        .collect();
    send_frame(
        writer,
        &ServerFrame {
            frame_type: ServerFrameType::AgentsUpdate,
            payload: ServerPayload::AgentsUpdate {
                agents,
                active_id: active_id.to_string(),
            },
        },
    )
    .await
}

/// Handle an `AgentListRequest`.
pub(crate) async fn handle_agent_list(
    writer: &mut dyn transport::TransportWriter,
    config: &Config,
    active_id: &str,
) -> Result<()> {
    send_agents_update(writer, config, active_id).await
}

/// Handle an `AgentSwitch`: persist the outgoing agent's state, load the
/// target agent's state, repoint the workspace and system prompt, and
/// notify the client. Returns `true` when the switch happened (the caller
/// must then re-subscribe to the new thread manager's events).
///
/// `thread_mgr_cell` is the store the connection's reader answers history
/// from, and it is repointed here rather than by the caller: everything below
/// the swap tells the client about the new agent, and the client answers a
/// thread list by asking for a transcript. Publishing after the announcement
/// would leave a window where the new agent's ids are resolved against the old
/// agent's store — which resolves, because ids restart low in each.
pub(crate) async fn handle_agent_switch(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    base_system_prompt: &Option<String>,
    session: &mut AgentSession,
    thread_mgr_cell: &Arc<std::sync::RwLock<SharedThreadMgr>>,
    task_mgr: &SharedTaskManager,
    agent_id: String,
) -> Result<bool> {
    let registry = config.agent_registry();
    let Some(info) = registry.get(&agent_id) else {
        send_error(writer, format!("Unknown agent '{}'", agent_id)).await?;
        return Ok(false);
    };
    if agent_id == session.agent_id {
        // Already active — just refresh the client's view.
        send_agents_update(writer, config, &agent_id).await?;
        return Ok(false);
    }
    debug!(from = %session.agent_id, to = %agent_id, "Switching active agent");

    session.save().await;
    *session = AgentSession::load(config, &agent_id);
    // Before a single frame about the new agent goes out — see the note on
    // this function. The reader answers history without passing through here,
    // so this is the moment the two must agree.
    *thread_mgr_cell
        .write()
        .expect("thread manager cell poisoned") = session.thread_mgr.clone();

    // Non-main agents may carry their own base system prompt; main gets
    // the connection's original prompt back.
    config.system_prompt = registry
        .manifest(&agent_id)
        .and_then(|m| m.system_prompt)
        .or_else(|| base_system_prompt.clone());

    rustyclaw_core::runtime_ctx::set_active_agent(&agent_id);

    send_frame(
        writer,
        &ServerFrame {
            frame_type: ServerFrameType::AgentSwitched,
            payload: ServerPayload::AgentSwitched {
                agent_id: agent_id.clone(),
                name: info.name,
            },
        },
    )
    .await?;
    send_agents_update(writer, config, &agent_id).await?;

    // Repoint the workspace at the new agent's active project and push the
    // new sidebar state.
    let active_project = session.project_mgr.active_id();
    // The new agent's own last-left thread, adopted as this connection's:
    // the outgoing agent's foreground names a thread in a different store.
    session.restore_foreground().await;
    let foreground = session.foreground_id();
    project_handler::activate_project(
        writer,
        config,
        &mut session.project_mgr,
        &session.thread_mgr,
        &session.projects_path,
        active_project,
        foreground,
    )
    .await?;
    send_threads_update_shared(writer, &session.thread_mgr, task_mgr, None, foreground).await?;
    send_projects_update(writer, &session.project_mgr).await?;

    Ok(true)
}

/// Handle an `AgentCreate`: register the agent and broadcast the new list.
pub(crate) async fn handle_agent_create(
    writer: &mut dyn transport::TransportWriter,
    config: &Config,
    active_id: &str,
    name: String,
    agent_id: Option<String>,
    description: Option<String>,
) -> Result<()> {
    match config.agent_registry().create(
        agent_id.as_deref(),
        &name,
        description.as_deref(),
        None,
        None,
    ) {
        Ok(info) => {
            debug!(id = %info.id, "Agent created via gateway");
            send_agents_update(writer, config, active_id).await
        }
        Err(e) => send_error(writer, format!("Could not create agent: {}", e)).await,
    }
}

/// Handle an `AgentDelete`. The active agent and `main` are protected.
pub(crate) async fn handle_agent_delete(
    writer: &mut dyn transport::TransportWriter,
    config: &Config,
    active_id: &str,
    agent_id: String,
) -> Result<()> {
    if agent_id == MAIN_AGENT_ID {
        return send_error(writer, "The main agent cannot be deleted".to_string()).await;
    }
    if agent_id == active_id {
        return send_error(
            writer,
            "Cannot delete the active agent — switch to another agent first".to_string(),
        )
        .await;
    }
    match config.agent_registry().delete(&agent_id) {
        Ok(()) => send_agents_update(writer, config, active_id).await,
        Err(e) => send_error(writer, format!("Could not delete agent: {}", e)).await,
    }
}

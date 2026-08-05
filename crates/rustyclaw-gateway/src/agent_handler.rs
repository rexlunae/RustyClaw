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
use tokio::sync::Mutex;
use tracing::debug;

use rustyclaw_core::agents::MAIN_AGENT_ID;
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, transport};
use rustyclaw_core::projects::ProjectManager;

use crate::project_handler;
use crate::thread_updates::{send_projects_update, send_threads_update_shared};
use crate::{SharedTaskManager, SharedThreadMgr};

/// One thread manager per agent, shared by every connection using it.
///
/// A manager is the authority on which threads exist, and `ThreadStore::
/// persist` acts on that authority: it deletes the files of every thread the
/// manager does not contain. Building one per connection therefore made two
/// windows on the same agent mutually destructive — each held a snapshot of
/// the store taken when it connected, and the first write after the other
/// created a thread deleted it from disk. Not a race in the narrow sense
/// either: the loser is whatever the other window did *at any point* since
/// the connection opened.
///
/// Keyed by the store's path rather than the agent id, because the path is
/// what a manager is the authority *for*. Two agents have separate
/// directories and cannot reconcile each other away; so do two installations
/// pointed at different settings directories, which an agent id alone would
/// have collapsed into one.
///
/// Entries live for the process. A manager is a few hundred bytes plus its
/// threads, an installation has a handful of agents, and dropping one while
/// a connection still held it would put us straight back to two authorities
/// for one store.
static THREAD_MANAGERS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, SharedThreadMgr>>,
> = std::sync::LazyLock::new(Default::default);

/// The shared manager for the store at `threads_path`, loading it from disk
/// on first use.
fn thread_manager_for(threads_path: &std::path::Path) -> SharedThreadMgr {
    let mut managers = THREAD_MANAGERS
        .lock()
        .expect("thread manager registry poisoned");
    managers
        .entry(threads_path.to_path_buf())
        .or_insert_with(|| {
            Arc::new(Mutex::new(
                rustyclaw_core::threads::ThreadStore::load_or_migrate(threads_path),
            ))
        })
        .clone()
}

/// Forget the cached manager for a store that no longer exists.
///
/// Keeping a manager for the life of the process is what makes it the single
/// authority — but an agent's directory can be removed out from under it.
/// Deleting an agent and creating another with the same id would otherwise
/// hand the new one the old manager, and its first write would put the
/// deleted agent's conversations back on disk. Reloading from disk on every
/// connection used to make that impossible; caching is what introduces it.
fn forget_thread_manager(threads_path: &std::path::Path) {
    THREAD_MANAGERS
        .lock()
        .expect("thread manager registry poisoned")
        .remove(threads_path);
}

/// Everything about the connection that is scoped to one agent. Swapped
/// wholesale on agent switch.
pub(crate) struct AgentSession {
    pub agent_id: String,
    pub threads_path: PathBuf,
    pub projects_path: PathBuf,
    /// Shared with the running model task — see [`crate::SharedThreadMgr`].
    pub thread_mgr: SharedThreadMgr,
    pub project_mgr: ProjectManager,
}

impl AgentSession {
    /// Load an agent's persisted thread/project state, creating the
    /// directory skeleton on first use.
    pub fn load(config: &Config, agent_id: &str) -> Self {
        let sessions_dir = config.sessions_dir_for(agent_id);
        let _ = std::fs::create_dir_all(&sessions_dir);
        let threads_path = sessions_dir.join("threads.json");
        let projects_path = sessions_dir.join("projects.json");
        let thread_mgr = thread_manager_for(&threads_path);
        let mut project_mgr = ProjectManager::load_or_new(&projects_path);
        project_mgr.ensure_default(config.workspace_dir_for(agent_id));
        crate::helpers::persist_projects(&project_mgr, &projects_path);
        Self {
            agent_id: agent_id.to_string(),
            threads_path,
            projects_path,
            thread_mgr,
            project_mgr,
        }
    }

    /// Persist thread and project state.
    pub async fn save(&self) {
        crate::helpers::persist_threads(&mut *self.thread_mgr.lock().await, &self.threads_path);
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
    project_handler::activate_project(
        writer,
        config,
        &mut session.project_mgr,
        &session.thread_mgr,
        &session.projects_path,
        active_project,
    )
    .await?;
    send_threads_update_shared(writer, &session.thread_mgr, task_mgr, None).await?;
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
        Ok(()) => {
            // The directory is gone, so the manager cached for it must go
            // too — otherwise an agent recreated under this id inherits the
            // deleted one's conversations and writes them back out.
            forget_thread_manager(&config.sessions_dir_for(&agent_id).join("threads.json"));
            send_agents_update(writer, config, active_id).await
        }
        Err(e) => send_error(writer, format!("Could not delete agent: {}", e)).await,
    }
}

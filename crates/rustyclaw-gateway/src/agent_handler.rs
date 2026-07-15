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
use tracing::debug;

use rustyclaw_core::agents::MAIN_AGENT_ID;
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, transport};
use rustyclaw_core::projects::ProjectManager;
use rustyclaw_core::threads::ThreadManager;

use crate::SharedTaskManager;
use crate::project_handler;
use crate::thread_updates::{send_projects_update, send_threads_update};

/// Everything about the connection that is scoped to one agent. Swapped
/// wholesale on agent switch.
pub(crate) struct AgentSession {
    pub agent_id: String,
    pub threads_path: PathBuf,
    pub projects_path: PathBuf,
    pub thread_mgr: ThreadManager,
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
        let thread_mgr = ThreadManager::load_or_default(&threads_path);
        let mut project_mgr = ProjectManager::load_or_new(&projects_path);
        project_mgr.ensure_default(config.workspace_dir_for(agent_id));
        let _ = project_mgr.save_to_file(&projects_path);
        Self {
            agent_id: agent_id.to_string(),
            threads_path,
            projects_path,
            thread_mgr,
            project_mgr,
        }
    }

    /// Persist thread and project state.
    pub fn save(&self) {
        let _ = self.thread_mgr.save_to_file(&self.threads_path);
        let _ = self.project_mgr.save_to_file(&self.projects_path);
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
pub(crate) async fn handle_agent_switch(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    base_system_prompt: &Option<String>,
    session: &mut AgentSession,
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

    session.save();
    *session = AgentSession::load(config, &agent_id);

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
        &session.projects_path,
        active_project,
    )
    .await?;
    send_threads_update(writer, &session.thread_mgr, task_mgr, None).await?;
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

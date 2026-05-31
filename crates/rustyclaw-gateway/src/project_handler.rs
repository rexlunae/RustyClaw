//! Project client-frame handlers (list / create / rename / delete / switch).
//!
//! A project is a named working directory that groups threads. The *active*
//! project's [`path`](rustyclaw_core::projects::Project::path) is the agent's
//! working directory: whenever the active project changes (here, or via a
//! thread switch in [`crate::thread_handler`]), [`activate_project`] repoints
//! `config.workspace_dir` so tool execution runs in that directory.

use anyhow::Result;
use std::path::Path;
use tracing::debug;

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, transport};
use rustyclaw_core::projects::{ProjectId, ProjectManager};

use crate::admin;
use crate::thread_updates::send_projects_update;

/// Make `project_id` the active project: repoint the workspace dir (so tools
/// run in the project's directory), persist, and broadcast `ProjectsUpdate`.
/// No-op if the project doesn't exist.
pub(crate) async fn activate_project(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    project_mgr: &mut ProjectManager,
    projects_path: &Path,
    project_id: ProjectId,
) -> Result<()> {
    if let Some(path) = project_mgr.path_of(project_id) {
        project_mgr.set_active(project_id);
        admin::handle_set_working_directory(config, path.display().to_string());
        let _ = project_mgr.save_to_file(projects_path);
        send_projects_update(writer, project_mgr).await?;
    }
    Ok(())
}

/// Handle a `ProjectList`: broadcast the current project list.
pub(crate) async fn handle_project_list(
    writer: &mut dyn transport::TransportWriter,
    project_mgr: &ProjectManager,
) -> Result<()> {
    send_projects_update(writer, project_mgr).await
}

/// Handle a `ProjectCreate`: register a new project (creating its directory),
/// make it active, and broadcast the updated list.
pub(crate) async fn handle_project_create(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    project_mgr: &mut ProjectManager,
    projects_path: &Path,
    name: String,
    path: String,
) -> Result<()> {
    debug!("Project create request: {} @ {}", name, path);
    if let Err(e) = std::fs::create_dir_all(&path) {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: format!("Could not create project directory '{path}': {e}"),
            },
        };
        return send_frame(writer, &frame).await;
    }
    let id = project_mgr.create(name, path);
    activate_project(writer, config, project_mgr, projects_path, id).await
}

/// Handle a `ProjectRename`.
pub(crate) async fn handle_project_rename(
    writer: &mut dyn transport::TransportWriter,
    project_mgr: &mut ProjectManager,
    projects_path: &Path,
    project_id: u64,
    new_name: String,
) -> Result<()> {
    if project_mgr.rename(ProjectId(project_id), new_name) {
        let _ = project_mgr.save_to_file(projects_path);
    }
    send_projects_update(writer, project_mgr).await
}

/// Handle a `ProjectDelete`. Refuses to delete the Default or last project
/// (enforced by [`ProjectManager::remove`]). If the active project was
/// deleted, falls back to Default and repoints the workspace.
pub(crate) async fn handle_project_delete(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    project_mgr: &mut ProjectManager,
    projects_path: &Path,
    project_id: u64,
) -> Result<()> {
    if project_mgr.remove(ProjectId(project_id)).is_some() {
        let _ = project_mgr.save_to_file(projects_path);
        // `remove` may have changed the active project; re-point the workspace.
        let active = project_mgr.active_id();
        return activate_project(writer, config, project_mgr, projects_path, active).await;
    }
    let frame = ServerFrame {
        frame_type: ServerFrameType::Error,
        payload: ServerPayload::Error {
            ok: false,
            message: "Cannot delete the default or last remaining project".to_string(),
        },
    };
    send_frame(writer, &frame).await
}

/// Handle a `ProjectSwitch`: make the project active and repoint the workspace.
pub(crate) async fn handle_project_switch(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    project_mgr: &mut ProjectManager,
    projects_path: &Path,
    project_id: u64,
) -> Result<()> {
    activate_project(
        writer,
        config,
        project_mgr,
        projects_path,
        ProjectId(project_id),
    )
    .await
}

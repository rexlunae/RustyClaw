//! Project client-frame handlers (list / create / rename / update / delete /
//! switch).
//!
//! A project is a named working directory that groups threads, and its threads
//! run in that directory unless they pin one of their own. [`repoint_workspace`]
//! is the single place that rule reaches `config.workspace_dir`: every path
//! that can change the effective directory — a project switch, a thread switch,
//! a project or thread edit — goes through it, so an override is never quietly
//! overwritten.

use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::debug;

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, transport};
use rustyclaw_core::projects::{ProjectId, ProjectManager};
use rustyclaw_core::threads::ThreadManager;

use crate::admin;
use crate::thread_updates::send_projects_update;

/// Point `config.workspace_dir` at the directory the agent should actually run
/// in: the foreground thread's own override when it has one, otherwise that
/// thread's project directory, otherwise the active project's directory.
///
/// This is the only place the workspace is derived from project/thread state,
/// so a thread override cannot be silently clobbered by a project switch.
pub(crate) fn repoint_workspace(
    config: &mut Config,
    project_mgr: &ProjectManager,
    thread_mgr: &ThreadManager,
) {
    let dir = thread_mgr
        .foreground()
        .and_then(|t| project_mgr.effective_dir_for(t))
        .or_else(|| project_mgr.path_of(project_mgr.active_id()));
    if let Some(dir) = dir {
        admin::handle_set_working_directory(config, dir);
    }
}

/// Make `project_id` the active project: repoint the workspace dir (so tools
/// run in the right directory), persist, and broadcast `ProjectsUpdate`.
/// No-op if the project doesn't exist.
pub(crate) async fn activate_project(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    project_mgr: &mut ProjectManager,
    thread_mgr: &ThreadManager,
    projects_path: &Path,
    project_id: ProjectId,
) -> Result<()> {
    if project_mgr.contains(project_id) {
        project_mgr.set_active(project_id);
        repoint_workspace(config, project_mgr, thread_mgr);
        crate::helpers::persist_projects(project_mgr, projects_path);
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
    thread_mgr: &ThreadManager,
    projects_path: &Path,
    name: String,
    path: PathBuf,
) -> Result<()> {
    debug!("Project create request: {} @ {}", name, path.display());
    if let Err(message) = crate::helpers::reject_non_utf8_path(&path) {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error { ok: false, message },
        };
        return send_frame(writer, &frame).await;
    }
    if let Err(e) = std::fs::create_dir_all(&path) {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: format!(
                    "Could not create project directory '{}': {e}",
                    path.display()
                ),
            },
        };
        return send_frame(writer, &frame).await;
    }
    let id = project_mgr.create(name, path);
    activate_project(writer, config, project_mgr, thread_mgr, projects_path, id).await
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
        crate::helpers::persist_projects(project_mgr, projects_path);
    }
    send_projects_update(writer, project_mgr).await
}

/// Handle a `ProjectUpdate`: set a project's name and working directory.
///
/// The directory is created if missing, matching `ProjectCreate` — a project
/// is defined by its directory, so pointing one at a path that does not exist
/// yet is a reasonable thing to do. When the edited project is the active one
/// the agent's workspace is repointed immediately, so the change takes effect
/// without a thread switch.
pub(crate) async fn handle_project_update(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    project_mgr: &mut ProjectManager,
    thread_mgr: &ThreadManager,
    projects_path: &Path,
    project_id: u64,
    name: String,
    path: PathBuf,
) -> Result<()> {
    debug!(
        "Project update request: {} -> {} @ {}",
        project_id,
        name,
        path.display()
    );
    let id = ProjectId(project_id);

    if !project_mgr.contains(id) {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: format!("Cannot edit project {project_id}: not found"),
            },
        };
        return send_frame(writer, &frame).await;
    }

    let name = name.trim().to_string();
    if name.is_empty() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: "Project name cannot be empty".to_string(),
            },
        };
        return send_frame(writer, &frame).await;
    }

    // Only emptiness is rejected. Whitespace is *not* trimmed here: a
    // directory whose name legitimately ends in a space is a real thing on
    // every platform, and the client already trims its text field, so
    // trimming again server-side could only corrupt a deliberate path.
    if path.as_os_str().is_empty() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: "Project working directory cannot be empty".to_string(),
            },
        };
        return send_frame(writer, &frame).await;
    }

    if let Err(message) = crate::helpers::reject_non_utf8_path(&path) {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error { ok: false, message },
        };
        return send_frame(writer, &frame).await;
    }

    if let Err(e) = std::fs::create_dir_all(&path) {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: format!(
                    "Could not use '{}' as the project directory: {e}",
                    path.display()
                ),
            },
        };
        return send_frame(writer, &frame).await;
    }

    project_mgr.rename(id, name);
    project_mgr.set_path(id, &path);
    crate::helpers::persist_projects(project_mgr, projects_path);

    // Moving a project moves the threads that inherit from it, so re-derive
    // the workspace. `repoint_workspace` keeps a thread override in place.
    repoint_workspace(config, project_mgr, thread_mgr);

    send_projects_update(writer, project_mgr).await
}

/// Handle a `ProjectDelete`. Refuses to delete the Default or last project
/// (enforced by [`ProjectManager::remove`]). If the active project was
/// deleted, falls back to Default and repoints the workspace.
pub(crate) async fn handle_project_delete(
    writer: &mut dyn transport::TransportWriter,
    config: &mut Config,
    project_mgr: &mut ProjectManager,
    thread_mgr: &ThreadManager,
    projects_path: &Path,
    project_id: u64,
) -> Result<()> {
    if project_mgr.remove(ProjectId(project_id)).is_some() {
        crate::helpers::persist_projects(project_mgr, projects_path);
        // `remove` may have changed the active project; re-point the workspace.
        let active = project_mgr.active_id();
        return activate_project(
            writer,
            config,
            project_mgr,
            thread_mgr,
            projects_path,
            active,
        )
        .await;
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
    thread_mgr: &ThreadManager,
    projects_path: &Path,
    project_id: u64,
) -> Result<()> {
    activate_project(
        writer,
        config,
        project_mgr,
        thread_mgr,
        projects_path,
        ProjectId(project_id),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::threads::ThreadManager;

    /// The workspace follows the foreground thread's *effective* directory,
    /// not the active project's path. Before `effective_dir_for` existed the
    /// two were the same thing, and a thread override would have been
    /// overwritten by the next project switch without anything reporting it.
    #[test]
    fn workspace_follows_the_foreground_thread_override() {
        let mut config = Config::default();
        let mut projects = ProjectManager::new();
        let api = projects.create("Api", "/srv/api");
        projects.set_active(api);

        let mut threads = ThreadManager::new();
        let pinned = threads.create_chat("pinned");
        threads.set_project(pinned, api);
        threads.set_working_dir(pinned, Some("/tmp/worktree".into()));

        repoint_workspace(&mut config, &projects, &threads);
        assert_eq!(
            config.workspace_dir(),
            std::path::PathBuf::from("/tmp/worktree"),
            "the thread's override wins over its project's directory"
        );

        // Clearing the override hands the thread back to the project.
        threads.set_working_dir(pinned, None);
        repoint_workspace(&mut config, &projects, &threads);
        assert_eq!(config.workspace_dir(), std::path::PathBuf::from("/srv/api"));
    }

    /// A pin survives a restart.
    ///
    /// Connection setup used to read the active *project's* path directly,
    /// which silently dropped a restored thread's override until some later
    /// event happened to repoint — so the agent's first tool calls after a
    /// gateway restart ran in the wrong directory. This walks the real
    /// restore path: persist, reload from disk, derive the workspace.
    #[test]
    fn a_restored_pin_survives_reconnecting() {
        let tmp = tempfile::tempdir().unwrap();
        let threads_path = tmp.path().join("threads.json");
        let projects_path = tmp.path().join("projects.json");

        {
            let mut projects = ProjectManager::new();
            let api = projects.create("Api", "/srv/api");
            projects.set_active(api);

            let mut threads = ThreadManager::new();
            let pinned = threads.create_chat("pinned");
            threads.set_project(pinned, api);
            threads.set_working_dir(pinned, Some("/tmp/worktree".into()));
            assert_eq!(threads.foreground_id(), Some(pinned));

            projects.save_to_file(&projects_path).unwrap();
            threads.save_to_file(&threads_path).unwrap();
        }

        // A fresh connection: reload both managers, then derive the workspace
        // exactly as `handle_connection` does.
        let projects = ProjectManager::load_or_new(&projects_path);
        let threads = ThreadManager::load_or_default(&threads_path);
        let mut config = Config::default();
        repoint_workspace(&mut config, &projects, &threads);

        assert_eq!(
            config.workspace_dir(),
            std::path::PathBuf::from("/tmp/worktree"),
            "the restored thread's pin must survive the reconnect"
        );
    }

    /// With no foreground thread there is nothing to inherit from, so the
    /// active project's directory is the answer.
    #[test]
    fn workspace_falls_back_to_the_active_project() {
        let mut config = Config::default();
        let mut projects = ProjectManager::new();
        let api = projects.create("Api", "/srv/api");
        projects.set_active(api);
        let threads = ThreadManager::new();

        repoint_workspace(&mut config, &projects, &threads);
        assert_eq!(config.workspace_dir(), std::path::PathBuf::from("/srv/api"));
    }
}

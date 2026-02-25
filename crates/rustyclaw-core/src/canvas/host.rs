//! Canvas host server for serving content and handling A2UI.

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::a2ui::{A2UIMessage, A2UISurface};
use super::config::CanvasConfig;

/// Canvas host server.
///
/// Serves HTML/CSS/JS content and handles A2UI updates.
pub struct CanvasHost {
    /// Configuration
    config: CanvasConfig,

    /// Workspace root
    workspace: PathBuf,

    /// Active A2UI surfaces by session
    surfaces: Arc<RwLock<HashMap<String, HashMap<String, A2UISurface>>>>,

    /// Whether the host is running
    running: Arc<RwLock<bool>>,
}

impl CanvasHost {
    /// Create a new canvas host.
    pub fn new(config: CanvasConfig, workspace: PathBuf) -> Self {
        Self {
            config,
            workspace,
            surfaces: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the canvas host server.
    pub async fn start(&self) -> Result<()> {
        if !self.config.enabled {
            info!("Canvas is disabled");
            return Ok(());
        }

        // Ensure canvas root exists
        let root = self.config.canvas_root(&self.workspace);
        tokio::fs::create_dir_all(&root).await?;

        *self.running.write().await = true;
        info!(port = self.config.port, root = %root.display(), "Canvas host started");

        // Note: Full HTTP server implementation would go here
        // For now, this is a placeholder for the API surface

        Ok(())
    }

    /// Stop the canvas host server.
    pub async fn stop(&self) -> Result<()> {
        *self.running.write().await = false;
        info!("Canvas host stopped");
        Ok(())
    }

    /// Check if the host is running.
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// Get the canvas URL for a session.
    pub fn canvas_url(&self, session: &str) -> String {
        format!("http://localhost:{}/canvas/{}/", self.config.port, session)
    }

    /// Get the A2UI URL for a session.
    pub fn a2ui_url(&self, session: &str) -> String {
        format!("http://localhost:{}/__rustyclaw__/a2ui/{}/", self.config.port, session)
    }

    // ── Session management ──────────────────────────────────────────────────

    /// Ensure a session canvas directory exists.
    pub async fn ensure_session(&self, session: &str) -> Result<PathBuf> {
        let dir = self.config.session_dir(&self.workspace, session);
        tokio::fs::create_dir_all(&dir).await?;
        Ok(dir)
    }

    /// Write a file to a session's canvas directory.
    pub async fn write_file(&self, session: &str, path: &str, content: &[u8]) -> Result<PathBuf> {
        let dir = self.ensure_session(session).await?;
        let file_path = dir.join(path);

        // Ensure parent directories exist
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&file_path, content).await?;
        debug!(session, path, "Canvas file written");

        Ok(file_path)
    }

    /// Read a file from a session's canvas directory.
    pub async fn read_file(&self, session: &str, path: &str) -> Result<Vec<u8>> {
        let dir = self.config.session_dir(&self.workspace, session);
        let file_path = dir.join(path);
        let content = tokio::fs::read(&file_path).await?;
        Ok(content)
    }

    // ── A2UI support ────────────────────────────────────────────────────────

    /// Push A2UI messages to a session.
    pub async fn push_a2ui(&self, session: &str, messages: Vec<A2UIMessage>) -> Result<()> {
        let mut surfaces = self.surfaces.write().await;
        let session_surfaces = surfaces.entry(session.to_string()).or_default();

        for msg in messages {
            match &msg {
                A2UIMessage::BeginRendering { surface_id, .. } |
                A2UIMessage::SurfaceUpdate { surface_id, .. } |
                A2UIMessage::DataModelUpdate { surface_id, .. } => {
                    let surface = session_surfaces
                        .entry(surface_id.clone())
                        .or_insert_with(|| A2UISurface::new(surface_id));
                    surface.apply(&msg);
                }
                A2UIMessage::DeleteSurface { surface_id } => {
                    session_surfaces.remove(surface_id);
                }
            }
        }

        debug!(session, count = session_surfaces.len(), "A2UI surfaces updated");
        Ok(())
    }

    /// Push simple text to A2UI.
    pub async fn push_text(&self, session: &str, text: &str) -> Result<()> {
        use super::a2ui::{A2UIComponent, A2UIComponentDef, A2UITextValue, A2UIChildren};

        let messages = vec![
            A2UIMessage::SurfaceUpdate {
                surface_id: "main".to_string(),
                components: vec![
                    A2UIComponentDef {
                        id: "root".to_string(),
                        component: A2UIComponent::Column {
                            children: A2UIChildren::ExplicitList(vec!["content".to_string()]),
                            spacing: None,
                        },
                    },
                    A2UIComponentDef {
                        id: "content".to_string(),
                        component: A2UIComponent::Text {
                            text: A2UITextValue::literal(text),
                            usage_hint: Some("body".to_string()),
                        },
                    },
                ],
            },
            A2UIMessage::BeginRendering {
                surface_id: "main".to_string(),
                root: "root".to_string(),
            },
        ];

        self.push_a2ui(session, messages).await
    }

    /// Get all surfaces for a session.
    pub async fn get_surfaces(&self, session: &str) -> HashMap<String, A2UISurface> {
        self.surfaces
            .read()
            .await
            .get(session)
            .cloned()
            .unwrap_or_default()
    }

    /// Reset A2UI state for a session.
    pub async fn reset_a2ui(&self, session: &str) -> Result<()> {
        self.surfaces.write().await.remove(session);
        debug!(session, "A2UI state reset");
        Ok(())
    }

    // ── Snapshot ────────────────────────────────────────────────────────────

    /// Capture a snapshot of the canvas (placeholder for browser-based capture).
    pub async fn snapshot(&self, session: &str) -> Result<Vec<u8>> {
        // This would require browser automation to capture
        // For now, return an error indicating it's not implemented
        anyhow::bail!("Canvas snapshot requires browser automation (not yet implemented)")
    }
}

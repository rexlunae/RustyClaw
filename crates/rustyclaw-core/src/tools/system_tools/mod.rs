//! System-level tools: disk analysis, monitoring, app management,
//! GUI automation, security auditing, and file summarization.
//!
//! Split into submodules for maintainability.

mod disk;
mod monitor;
mod apps;
mod media;
mod security;
mod text;

// Re-export sync functions
pub use disk::{exec_disk_usage, exec_classify_files};
pub use monitor::{exec_system_monitor, exec_battery_health};
pub use apps::{exec_app_index, exec_cloud_browse, exec_browser_cache};
pub use media::{exec_screenshot, exec_clipboard};
pub use security::{exec_audit_sensitive, exec_secure_delete};
pub use text::exec_summarize_file;

// Re-export async functions
pub use disk::{exec_disk_usage_async, exec_classify_files_async};
pub use monitor::{exec_system_monitor_async, exec_battery_health_async};
pub use apps::{exec_app_index_async, exec_cloud_browse_async, exec_browser_cache_async};
pub use media::{exec_screenshot_async, exec_clipboard_async};
pub use security::{exec_audit_sensitive_async, exec_secure_delete_async};
pub use text::exec_summarize_file_async;

use std::path::Path;

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Run a shell pipeline via `sh -c` (sync).
pub(crate) fn sh(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run a shell pipeline via `sh -c` (async).
pub(crate) async fn sh_async(script: &str) -> Result<String, String> {
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .await
        .map_err(|e| format!("shell error: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Resolve a path relative to workspace (shared helper).
pub(crate) fn resolve_path(workspace_dir: &Path, path_str: &str) -> std::path::PathBuf {
    crate::tools::helpers::resolve_path(workspace_dir, path_str)
}

/// Expand tilde in paths (shared helper).
pub(crate) fn expand_tilde(path_str: &str) -> std::path::PathBuf {
    crate::tools::helpers::expand_tilde(path_str)
}

/// Check if a command exists (sync).
pub(crate) fn has_command(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a command exists (async).
pub(crate) async fn has_command_async(cmd: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(cmd)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

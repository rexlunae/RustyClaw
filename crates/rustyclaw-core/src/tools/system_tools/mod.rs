//! System-level tools: disk analysis, monitoring, app management,
//! GUI automation, security auditing, and file summarization.
//!
//! Split into submodules for maintainability.

mod apps;
mod disk;
mod media;
mod monitor;
mod security;
mod text;

// Re-export sync functions
pub use apps::{exec_app_index, exec_browser_cache, exec_cloud_browse};
pub use disk::{exec_classify_files, exec_disk_usage};
pub use media::{exec_clipboard, exec_screenshot};
pub use monitor::{exec_battery_health, exec_system_monitor};
pub use security::{exec_audit_sensitive, exec_secure_delete};
pub use text::exec_summarize_file;

// Re-export async functions
pub use apps::{exec_app_index_async, exec_browser_cache_async, exec_cloud_browse_async};
pub use disk::{exec_classify_files_async, exec_disk_usage_async};
pub use media::{exec_clipboard_async, exec_screenshot_async};
pub use monitor::{exec_battery_health_async, exec_system_monitor_async};
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
#[allow(dead_code)]
pub(crate) fn has_command(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a command exists (async).
#[allow(dead_code)]
pub(crate) async fn has_command_async(cmd: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(cmd)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

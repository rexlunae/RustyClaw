//! Helper functions and global state for the tools system.

use crate::process_manager::{ProcessManager, SharedProcessManager};
use crate::sandbox::{Sandbox, SandboxMode, SandboxPolicy};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::{debug, warn};

// ── Global process manager ──────────────────────────────────────────────────

/// Global process manager for background exec sessions.
static PROCESS_MANAGER: OnceLock<SharedProcessManager> = OnceLock::new();

/// Get the global process manager instance.
pub fn process_manager() -> &'static SharedProcessManager {
    PROCESS_MANAGER.get_or_init(|| Arc::new(Mutex::new(ProcessManager::new())))
}

// ── Global sandbox configuration ────────────────────────────────────────────

/// Global sandbox instance, initialized once at gateway startup.
static SANDBOX: OnceLock<Sandbox> = OnceLock::new();

/// Called once from the gateway to initialize the sandbox.
pub fn init_sandbox(mode: SandboxMode, workspace: PathBuf, credentials_dir: PathBuf, deny_paths: Vec<PathBuf>) {
    debug!(?mode, ?workspace, "Initializing sandbox");
    let mut policy = SandboxPolicy::protect_credentials(&credentials_dir, &workspace);
    for path in deny_paths {
        policy = policy.deny_read(path.clone()).deny_write(path);
    }
    let sandbox = Sandbox::with_mode(mode, policy);
    let _ = SANDBOX.set(sandbox);
}

/// Get the global sandbox instance, if initialized.
pub fn sandbox() -> Option<&'static Sandbox> {
    SANDBOX.get()
}

/// Run a command through the sandbox (or unsandboxed if not initialized).
pub fn run_sandboxed_command(command: &str, cwd: &Path) -> Result<std::process::Output, String> {
    if let Some(sb) = SANDBOX.get() {
        debug!(mode = ?sb.mode, cwd = %cwd.display(), "Running sandboxed command");
        // Update policy workspace to the actual cwd for this command
        let mut policy = sb.policy.clone();
        policy.workspace = cwd.to_path_buf();
        crate::sandbox::run_sandboxed(command, &policy, sb.mode)
    } else {
        debug!(cwd = %cwd.display(), "Running unsandboxed command (no sandbox configured)");
        // No sandbox configured, run directly
        std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .output()
            .map_err(|e| format!("Command failed: {}", e))
    }
}

// ── Credentials directory protection ────────────────────────────────────────

/// Absolute path of the credentials directory, set once at gateway startup.
static CREDENTIALS_DIR: OnceLock<PathBuf> = OnceLock::new();

// ── Global vault for cookie jar access ──────────────────────────────────────

use crate::secrets::SecretsManager;

/// Shared vault type for thread-safe access (uses tokio::sync::Mutex for async).
pub type SharedVault = Arc<tokio::sync::Mutex<SecretsManager>>;

/// Global vault instance, set once at gateway startup.
static VAULT: OnceLock<SharedVault> = OnceLock::new();

/// Called once from the gateway to register the vault for tool access.
pub fn set_vault(vault: SharedVault) {
    let _ = VAULT.set(vault);
}

/// Get the global vault instance, if initialized.
pub fn vault() -> Option<&'static SharedVault> {
    VAULT.get()
}

/// Called once from the gateway to register the credentials path.
pub fn set_credentials_dir(path: PathBuf) {
    let _ = CREDENTIALS_DIR.set(path);
}

/// Returns `true` when a command string references the credentials directory.
pub fn command_references_credentials(command: &str) -> bool {
    if let Some(cred_dir) = CREDENTIALS_DIR.get() {
        let cred_str = cred_dir.to_string_lossy();
        command.contains(cred_str.as_ref())
    } else {
        false
    }
}

/// Returns `true` when `path` falls inside the credentials directory.
pub fn is_protected_path(path: &Path) -> bool {
    if let Some(cred_dir) = CREDENTIALS_DIR.get() {
        // Canonicalise both so symlinks / ".." can't bypass the check.
        let canon_cred = match cred_dir.canonicalize() {
            Ok(p) => p,
            Err(_) => return false, // dir doesn't exist yet – nothing to protect
        };
        let canon_path = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // File may not exist yet (write_file).  Fall back to
                // starts_with on the raw absolute path.
                return path.starts_with(cred_dir);
            }
        };
        canon_path.starts_with(&canon_cred)
    } else {
        false
    }
}

/// Standard denial message when a tool tries to touch the vault.
pub const VAULT_ACCESS_DENIED: &str =
    "Access denied: the credentials directory is protected. Use the secrets_list / secrets_get / secrets_store tools instead.";

// ── Path helpers ────────────────────────────────────────────────────────────

/// Resolve a path argument against the workspace root.
/// Absolute paths are used as-is; relative paths are joined to `workspace_dir`.
pub fn resolve_path(workspace_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_dir.join(p)
    }
}

/// Expand a leading `~` to the user's home directory.
pub fn expand_tilde(p: &str) -> PathBuf {
    if p.starts_with('~') {
        dirs::home_dir()
            .map(|h| h.join(p.strip_prefix("~/").unwrap_or(&p[1..])))
            .unwrap_or_else(|| PathBuf::from(p))
    } else {
        PathBuf::from(p)
    }
}

/// Decide how to present a path found during a search.
///
/// If `found` lives inside `workspace_dir`, return a workspace-relative path
/// so the model can pass it directly to `read_file` (which will resolve it
/// back against `workspace_dir`).  Otherwise return the **absolute** path so
/// the model can still use it with tools that accept absolute paths.
pub fn display_path(found: &Path, workspace_dir: &Path) -> String {
    if let Ok(rel) = found.strip_prefix(workspace_dir) {
        rel.display().to_string()
    } else {
        found.display().to_string()
    }
}

/// Filter for `walkdir` — skip common non-content directories.
pub fn should_visit(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    if entry.file_type().is_dir() {
        if matches!(
            name.as_ref(),
            ".git" | "node_modules" | "target" | ".hg" | ".svn"
                | "__pycache__" | "dist" | "build"
        ) {
            return false;
        }
        // Never recurse into the credentials directory.
        if is_protected_path(entry.path()) {
            return false;
        }
        true
    } else {
        true
    }
}

// ── Tool output sanitization ────────────────────────────────────────────────

/// Maximum size for tool output before truncation (50 KB).
const MAX_TOOL_OUTPUT_BYTES: usize = 50_000;

/// Detect if content looks like HTML or encoded binary data.
fn is_likely_garbage(s: &str) -> bool {
    // Check for HTML markers
    let lower = s.to_lowercase();
    if lower.contains("<!doctype") || lower.contains("<html") {
        return true;
    }
    
    // Check for base64-encoded data URIs
    if s.contains("data:image/") || s.contains("data:application/") {
        return true;
    }
    
    // Check for excessive base64-like content (long strings without spaces)
    let lines: Vec<&str> = s.lines().collect();
    let long_dense_lines = lines.iter().filter(|line| {
        line.len() > 500 && !line.contains(' ')
    }).count();
    if long_dense_lines > 3 {
        return true;
    }
    
    false
}

/// Sanitize tool output: truncate if too large, warn if garbage detected.
pub fn sanitize_tool_output(output: String) -> String {
    // Check for garbage content first
    if is_likely_garbage(&output) {
        let preview_len = output.len().min(500);
        let preview: String = output.chars().take(preview_len).collect();
        warn!(bytes = output.len(), "Tool returned HTML/binary content");
        return format!(
            "[Warning: Tool returned HTML/binary content ({} bytes) — likely not useful]\n\nPreview:\n{}...",
            output.len(),
            preview
        );
    }
    
    // Truncate if too large
    if output.len() > MAX_TOOL_OUTPUT_BYTES {
        debug!(bytes = output.len(), max = MAX_TOOL_OUTPUT_BYTES, "Truncating large tool output");
        let truncated: String = output.chars().take(MAX_TOOL_OUTPUT_BYTES).collect();
        format!(
            "{}...\n\n[Truncated: {} bytes total, showing first {}]",
            truncated,
            output.len(),
            MAX_TOOL_OUTPUT_BYTES
        )
    } else {
        output
    }
}

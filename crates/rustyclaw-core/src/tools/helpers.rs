//! Helper functions and global state for the tools system.

use crate::process_manager::{ProcessManager, SharedProcessManager};
use crate::sandbox::{Sandbox, SandboxMode, SandboxPolicy};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::{debug, error, warn};

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
pub fn init_sandbox(
    mode: SandboxMode,
    workspace: PathBuf,
    credentials_dir: PathBuf,
    deny_paths: Vec<PathBuf>,
) {
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
///
/// Uses double-canonicalize to detect symlink races (TOCTOU).
pub fn is_protected_path(path: &Path) -> bool {
    if let Some(cred_dir) = CREDENTIALS_DIR.get() {
        let canon_cred = match cred_dir.canonicalize() {
            Ok(p) => p,
            Err(_) => return false,
        };

        let canon_path = match resolve_path_no_race(path) {
            Ok(p) => p,
            Err(_) => {
                // File may not exist yet (write_file). Fall back to raw check.
                return path.starts_with(cred_dir);
            }
        };

        canon_path.starts_with(&canon_cred)
    } else {
        false
    }
}

/// Try to resolve a path and double-canonicalize to detect symlink swaps.
///
/// Returns `Ok(canonical)` if the path resolves consistently twice,
/// or `Err` if the path changed between resolutions (possible symlink race).
pub fn resolve_path_no_race(path: &Path) -> Result<PathBuf, String> {
    // Unless the source file exists, there's nothing to check for races.
    if !path.exists() {
        // For non-existent paths, just canonicalize what we can of the parent,
        // then reattach the filename.
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => return Ok(path.to_path_buf()),
        };
        let filename = path
            .file_name()
            .map(|n| Path::new(n).to_path_buf())
            .unwrap_or_default();
        match parent.canonicalize() {
            Ok(canon_parent) => Ok(canon_parent.join(filename)),
            Err(_) => Ok(path.to_path_buf()),
        }
    } else {
        // Double-canonicalize to catch symlink swaps.
        let canon1 = path
            .canonicalize()
            .map_err(|e| format!("Path resolution failed: {}", e))?;
        let canon2 = path
            .canonicalize()
            .map_err(|e| format!("Path resolution failed (retry): {}", e))?;

        if canon1 != canon2 {
            error!(
                path1 = %canon1.display(),
                path2 = %canon2.display(),
                "Path changed between resolutions — possible symlink race attack"
            );
            return Err(
                "Access denied: path changed between resolutions — possible symlink race attack"
                    .to_string(),
            );
        }

        Ok(canon2)
    }
}

/// Open a file for reading with O_NOFOLLOW on Linux and a final path-ownership check.
///
/// Returns `(File, canonical_path)` so the caller can use the fd without
/// worrying about the path changing under them.
pub fn open_file_read_safe(path: &Path) -> std::io::Result<(std::fs::File, PathBuf)> {
    // Step 1: resolve path safely, catching symlink races before opening.
    let canonical = resolve_path_no_race(path).map_err(std::io::Error::other)?;

    // Step 2: open with O_NOFOLLOW on Linux (fails if final component is a symlink).
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&canonical)?;

        // Step 3: verify the opened fd still points where we expect.
        let fd_path = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        if fd_path != canonical {
            return Err(std::io::Error::other(format!(
                "Symlink race detected: opened fd points to {}, expected {}",
                fd_path.display(),
                canonical.display()
            )));
        }

        Ok((file, canonical))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let file = std::fs::File::open(&canonical)?;
        Ok((file, canonical))
    }
}

/// Open a file for writing with O_NOFOLLOW on Linux and TOCTOU protection.
pub fn open_file_write_safe(path: &Path) -> std::io::Result<(std::fs::File, PathBuf)> {
    // For writes, the file may not exist yet — resolve what we can.
    let canonical = if path.exists() {
        resolve_path_no_race(path).map_err(std::io::Error::other)?
    } else {
        // File doesn't exist — canonicalize the parent directory.
        let parent = path.parent().unwrap_or(Path::new("."));
        let filename = path
            .file_name()
            .map(|n| Path::new(n).to_path_buf())
            .unwrap_or_default();
        let canon_parent = resolve_path_no_race(parent).map_err(std::io::Error::other)?;
        canon_parent.join(filename)
    };

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&canonical)?;

        let fd_path = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        if fd_path != canonical {
            return Err(std::io::Error::other(format!(
                "Symlink race detected: opened fd points to {}, expected {}",
                fd_path.display(),
                canonical.display()
            )));
        }

        Ok((file, canonical))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&canonical)?;
        Ok((file, canonical))
    }
}

/// Standard denial message when a tool tries to touch the vault.
pub const VAULT_ACCESS_DENIED: &str = "Access denied: the credentials directory is protected. Use the secrets_list / secrets_get / secrets_store tools instead.";

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

/// Blocked shell metacharacter patterns for command validation.
const BLOCKED_COMMAND_PATTERNS: &[&str] = &["$(<", "${HOME", "${cred", "${CRED"];

/// Blocked substrings in command strings indicating credential access attempts.
const BLOCKED_CRED_SUBSTRINGS: &[&str] = &[
    "/secrets.json",
    "/secrets.key",
    "/authorized_clients",
    "/client_ed25519_key",
    "/credentials/",
    "/.openclaw/",
];

/// Check a command string for direct credential-exfiltration patterns.
///
/// This supplements `command_references_credentials` by catching patterns
/// that the simple substring check misses.  Returns `true` if the command
/// should be blocked.
pub fn command_has_exfiltration_patterns(command: &str) -> bool {
    let lower = command.to_lowercase();

    // Block commands that read /proc/self/fd or /proc/<pid>/mem
    if lower.contains("/proc/") && (lower.contains("mem") || lower.contains("fd/")) {
        return true;
    }

    // Block sensitive patterns.
    for &pat in BLOCKED_COMMAND_PATTERNS {
        if command.contains(pat) {
            return true;
        }
    }

    // Block known sensitive file patterns.
    for &pat in BLOCKED_CRED_SUBSTRINGS {
        if command.contains(pat) {
            return true;
        }
    }

    // Block `~/.rustyclaw` or `$HOME/.rustyclaw` references
    if lower.contains("~/.rustyclaw") || lower.contains("$home/.rustyclaw") {
        return true;
    }

    // Block `ln -s` that target protected dirs
    if lower.starts_with("ln -s") || lower.contains("; ln -s") || lower.contains("&& ln -s") {
        if let Some(cred_dir) = CREDENTIALS_DIR.get() {
            let cred_str = cred_dir.to_string_lossy().to_lowercase();
            if lower.contains(&cred_str) {
                return true;
            }
        }
    }

    false
}

/// Validate a command string for basic safety.
/// Returns an error message for unsafe commands.
pub fn validate_command_safe(command: &str) -> Result<(), String> {
    // Null bytes are always blocked.
    if command.contains('\0') {
        return Err("Command contains null byte — blocked for security".to_string());
    }

    // Check command length.
    if command.len() > 4096 {
        return Err("Command too long (max 4096 characters) — blocked for security".to_string());
    }

    // Check for credential exfiltration patterns.
    if command_has_exfiltration_patterns(command) {
        return Err("Command blocked: contains credential exfiltration pattern".to_string());
    }

    Ok(())
}

/// Redact sensitive HTTP header values from web fetch results.
/// Currently strips Authorization, Cookie, Set-Cookie, and X-API-Key headers.
#[allow(dead_code)]
pub fn redact_sensitive_headers(headers: &str) -> String {
    let mut result = String::new();
    for line in headers.lines() {
        if line.to_lowercase().starts_with("authorization:")
            || line.to_lowercase().starts_with("cookie:")
            || line.to_lowercase().starts_with("set-cookie:")
            || line.to_lowercase().starts_with("x-api-key:")
        {
            let colon_idx = line.find(':').unwrap_or(0);
            let header_name = &line[..=colon_idx];
            result.push_str(&format!("{} [REDACTED]\n", header_name));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    result.trim_end().to_string()
}
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
            ".git" | "node_modules" | "target" | ".hg" | ".svn" | "__pycache__" | "dist" | "build"
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
    let long_dense_lines = lines
        .iter()
        .filter(|line| line.len() > 500 && !line.contains(' '))
        .count();
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
        debug!(
            bytes = output.len(),
            max = MAX_TOOL_OUTPUT_BYTES,
            "Truncating large tool output"
        );
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

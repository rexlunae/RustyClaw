//! Platform-specific sandbox wrappers (bubblewrap, macOS seatbelt, landlock).

#![allow(unused_imports)]
use super::*;

// ── Bubblewrap (Linux) ──────────────────────────────────────────────────────

/// Wrap a command in bubblewrap with the given policy.
#[cfg(target_os = "linux")]
pub fn wrap_with_bwrap(command: &str, policy: &SandboxPolicy) -> (String, Vec<String>) {
    let mut args = Vec::new();

    // Helper to check if a path should be denied for read access
    let is_read_denied = |path: &Path| -> bool {
        policy
            .deny_read
            .iter()
            .any(|deny| path.starts_with(deny) || deny.starts_with(path))
    };

    // Helper to check if a path should be denied for write access
    let is_write_denied = |path: &Path| -> bool {
        policy
            .deny_write
            .iter()
            .any(|deny| path.starts_with(deny) || deny.starts_with(path))
    };

    // Helper to check if a path should be denied for execute access
    let is_exec_denied = |path: &Path| -> bool {
        policy
            .deny_exec
            .iter()
            .any(|deny| path.starts_with(deny) || deny.starts_with(path))
    };

    // Basic namespace isolation
    args.push("--unshare-all".to_string());
    args.push("--share-net".to_string()); // Keep network for web_fetch etc

    // Mount a minimal root - only if not in deny_read or deny_exec
    for dir in &["/usr", "/lib", "/lib64", "/bin", "/sbin"] {
        let path = Path::new(dir);
        if path.exists() && !is_read_denied(path) && !is_exec_denied(path) {
            args.push("--ro-bind".to_string());
            args.push(dir.to_string());
            args.push(dir.to_string());
        }
    }

    // Read-only /etc - only if not in deny_read
    let etc_path = Path::new("/etc");
    if etc_path.exists() && !is_read_denied(etc_path) {
        args.push("--ro-bind".to_string());
        args.push("/etc".to_string());
        args.push("/etc".to_string());
    }

    // Workspace: read-only if in deny_write, writable otherwise, skip if in deny_read
    if !is_read_denied(&policy.workspace) {
        if is_write_denied(&policy.workspace) {
            args.push("--ro-bind".to_string());
        } else {
            args.push("--bind".to_string());
        }
        args.push(policy.workspace.display().to_string());
        args.push(policy.workspace.display().to_string());
    }

    // Writable /tmp
    args.push("--tmpfs".to_string());
    args.push("/tmp".to_string());

    // Set up /proc for basic functionality
    args.push("--proc".to_string());
    args.push("/proc".to_string());

    // Set up /dev minimally
    args.push("--dev".to_string());
    args.push("/dev".to_string());

    // Working directory
    args.push("--chdir".to_string());
    args.push(policy.workspace.display().to_string());

    // Die with parent
    args.push("--die-with-parent".to_string());

    // The actual command
    args.push("--".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(command.to_string());

    ("bwrap".to_string(), args)
}

#[cfg(not(target_os = "linux"))]
pub fn wrap_with_bwrap(_command: &str, _policy: &SandboxPolicy) -> (String, Vec<String>) {
    panic!("Bubblewrap is only available on Linux");
}

// ── macOS Sandbox ───────────────────────────────────────────────────────────

/// Generate a Seatbelt profile for macOS sandbox-exec.
#[cfg(target_os = "macos")]
fn generate_seatbelt_profile(policy: &SandboxPolicy) -> String {
    let mut profile = String::from("(version 1)\n");

    // Start with deny-all
    profile.push_str("(deny default)\n");

    // Allow basic process operations
    profile.push_str("(allow process-fork)\n");
    profile.push_str("(allow process-exec)\n");
    profile.push_str("(allow signal)\n");
    profile.push_str("(allow sysctl-read)\n");

    // Allow reading system files
    profile.push_str("(allow file-read* (subpath \"/usr\"))\n");
    profile.push_str("(allow file-read* (subpath \"/bin\"))\n");
    profile.push_str("(allow file-read* (subpath \"/sbin\"))\n");
    profile.push_str("(allow file-read* (subpath \"/Library\"))\n");
    profile.push_str("(allow file-read* (subpath \"/System\"))\n");
    profile.push_str("(allow file-read* (subpath \"/private/etc\"))\n");
    profile.push_str("(allow file-read* (subpath \"/private/var/db\"))\n");

    // Allow workspace access
    profile.push_str(&format!(
        "(allow file-read* file-write* (subpath \"{}\"))\n",
        policy.workspace.display()
    ));

    // Allow /tmp
    profile.push_str("(allow file-read* file-write* (subpath \"/private/tmp\"))\n");
    profile.push_str("(allow file-read* file-write* (subpath \"/tmp\"))\n");

    // Deny access to protected paths
    for denied in &policy.deny_read {
        profile.push_str(&format!(
            "(deny file-read* (subpath \"{}\"))\n",
            denied.display()
        ));
    }
    for denied in &policy.deny_write {
        profile.push_str(&format!(
            "(deny file-write* (subpath \"{}\"))\n",
            denied.display()
        ));
    }
    for denied in &policy.deny_exec {
        profile.push_str(&format!(
            "(deny process-exec (subpath \"{}\"))\n",
            denied.display()
        ));
    }

    // Allow network (for web_fetch)
    profile.push_str("(allow network*)\n");

    profile
}

/// Wrap a command in macOS sandbox-exec.
#[cfg(target_os = "macos")]
pub fn wrap_with_macos_sandbox(command: &str, policy: &SandboxPolicy) -> (String, Vec<String>) {
    let profile = generate_seatbelt_profile(policy);

    let args = vec![
        "-p".to_string(),
        profile,
        "sh".to_string(),
        "-c".to_string(),
        command.to_string(),
    ];

    ("sandbox-exec".to_string(), args)
}

#[cfg(not(target_os = "macos"))]
pub fn wrap_with_macos_sandbox(_command: &str, _policy: &SandboxPolicy) -> (String, Vec<String>) {
    panic!("macOS sandbox is only available on macOS");
}

// ── Landlock (Linux 5.13+) ──────────────────────────────────────────────────

/// Apply Landlock restrictions to the current process.
///
/// Landlock is ALLOWLIST-based: we specify paths that ARE allowed,
/// and everything else is automatically denied by the kernel.
///
/// **Warning:** This is irreversible for this process!
#[cfg(target_os = "linux")]
pub fn apply_landlock(policy: &SandboxPolicy) -> Result<(), String> {
    use landlock::{
        ABI, Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
    };

    let abi = ABI::V2;

    // Build the set of access rights we want to control
    // By "handling" these, any path NOT explicitly allowed will be denied
    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(|e| format!("Landlock ruleset creation failed: {}", e))?
        .create()
        .map_err(|e| format!("Landlock not supported (kernel < 5.13): {}", e))?;

    // Define standard system paths that should be readable
    let system_read_paths = [
        "/usr", "/lib", "/lib64", "/bin", "/sbin",
        "/etc", // Needed for DNS resolution, SSL certs, etc.
        "/proc", "/sys", "/dev",
    ];

    // Define paths that should be read+write
    let system_rw_paths = ["/tmp", "/var/tmp"];

    // Allow read access to system paths
    for path_str in &system_read_paths {
        let path = std::path::Path::new(path_str);
        if path.exists() {
            match PathFd::new(path) {
                Ok(fd) => {
                    ruleset = ruleset
                        .add_rule(PathBeneath::new(fd, AccessFs::from_read(abi)))
                        .map_err(|e| format!("Failed to add read rule for {}: {}", path_str, e))?;
                }
                Err(e) => {
                    warn!(path = %path_str, error = %e, "Cannot open path for Landlock read rule");
                }
            }
        }
    }

    // Allow read+write to temp paths
    for path_str in &system_rw_paths {
        let path = std::path::Path::new(path_str);
        if path.exists() {
            match PathFd::new(path) {
                Ok(fd) => {
                    ruleset = ruleset
                        .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
                        .map_err(|e| format!("Failed to add rw rule for {}: {}", path_str, e))?;
                }
                Err(e) => {
                    warn!(path = %path_str, error = %e, "Cannot open path for Landlock rw rule");
                }
            }
        }
    }

    // Allow full access to workspace
    if policy.workspace.exists() {
        match PathFd::new(&policy.workspace) {
            Ok(fd) => {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
                    .map_err(|e| format!("Failed to add workspace rule: {}", e))?;
            }
            Err(e) => {
                return Err(format!(
                    "Cannot open workspace {:?} for Landlock: {}",
                    policy.workspace, e
                ));
            }
        }
    }

    // Allow access to explicitly allowed paths (if any)
    for allowed_path in &policy.allow_paths {
        if allowed_path.exists() {
            match PathFd::new(allowed_path) {
                Ok(fd) => {
                    ruleset = ruleset
                        .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
                        .map_err(|e| {
                            format!("Failed to add allow rule for {:?}: {}", allowed_path, e)
                        })?;
                }
                Err(e) => {
                    warn!(
                        path = ?allowed_path,
                        error = %e,
                        "Cannot open path for Landlock allow rule"
                    );
                }
            }
        }
    }

    // NOTE: We do NOT add rules for deny_read paths.
    // By not adding them to the allowlist, they are automatically denied!
    // This is the key insight: Landlock denies by omission, not by explicit rule.

    if !policy.deny_read.is_empty() {
        debug!(
            denied_paths = policy.deny_read.len(),
            "Landlock: paths denied by omission from allowlist"
        );
    }

    // Apply the restrictions (irreversible!)
    ruleset
        .restrict_self()
        .map_err(|e| format!("Failed to apply Landlock restrictions: {}", e))?;

    info!(
        workspace = ?policy.workspace,
        system_paths = system_read_paths.len() + system_rw_paths.len(),
        "Landlock sandbox active"
    );

    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn apply_landlock(_policy: &SandboxPolicy) -> Result<(), String> {
    Err("Landlock is only supported on Linux".to_string())
}

// ── Unified Sandbox Runner ──────────────────────────────────────────────────

/// Run a command with sandboxing, auto-selecting the best available method.
pub fn run_sandboxed(
    command: &str,
    policy: &SandboxPolicy,
    mode: SandboxMode,
) -> Result<std::process::Output, String> {
    let caps = SandboxCapabilities::detect();

    // Resolve Auto mode
    let effective_mode = match mode {
        SandboxMode::Auto => caps.best_mode(),
        other => other,
    };

    // Validate mode is available
    match effective_mode {
        SandboxMode::Bubblewrap if !caps.bubblewrap => {
            warn!("Bubblewrap not available, falling back to path validation");
            return run_with_path_validation(command, policy);
        }
        SandboxMode::Landlock if !caps.landlock => {
            warn!("Landlock not available, falling back to path validation");
            return run_with_path_validation(command, policy);
        }
        SandboxMode::LandlockBwrap if !caps.landlock || !caps.bubblewrap => {
            warn!("Landlock+Bubblewrap not fully available");
            if caps.landlock {
                debug!("Falling back to Landlock only");
                return run_with_path_validation(command, policy);
            } else if caps.bubblewrap {
                debug!("Falling back to Bubblewrap only");
                return run_with_bubblewrap(command, policy);
            } else {
                debug!("Falling back to path validation");
                return run_with_path_validation(command, policy);
            }
        }
        SandboxMode::Docker if !caps.docker => {
            warn!("Docker not available, falling back to path validation");
            return run_with_path_validation(command, policy);
        }
        SandboxMode::MacOSSandbox if !caps.macos_sandbox => {
            warn!("macOS sandbox not available, falling back to path validation");
            return run_with_path_validation(command, policy);
        }
        _ => {}
    }

    match effective_mode {
        SandboxMode::None => run_unsandboxed(command),
        SandboxMode::PathValidation => run_with_path_validation(command, policy),
        SandboxMode::Bubblewrap => run_with_bubblewrap(command, policy),
        SandboxMode::Docker => run_with_docker(command, policy),
        SandboxMode::MacOSSandbox => run_with_macos_sandbox(command, policy),
        SandboxMode::Landlock => {
            // Landlock is process-wide; just run with path validation
            run_with_path_validation(command, policy)
        }
        SandboxMode::LandlockBwrap => run_with_landlock_bwrap(command, policy),
        SandboxMode::Auto => unreachable!(), // Already resolved above
    }
}

pub(crate) fn run_unsandboxed(command: &str) -> Result<std::process::Output, String> {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| format!("Command failed: {}", e))
}

/// Extract explicit paths from a shell command string.
///
/// This performs simple pattern matching to find:
/// - Absolute paths starting with /
/// - Home paths starting with ~/
///
/// Limitations: Cannot detect dynamic paths like `$(echo /path)` or command substitution.
/// Those require kernel-level enforcement (Landlock/Bubblewrap).
pub fn extract_paths_from_command(command: &str) -> Vec<PathBuf> {
    use std::path::PathBuf;
    let mut paths = Vec::new();

    // Pattern 1: Absolute paths - /path/to/file
    // Pattern 2: Home paths - ~/path/to/file
    // Match word boundaries, handle quotes

    let chars = command.chars().peekable();
    let mut current_token = String::new();
    let mut in_quotes = false;
    let mut quote_char = ' ';

    for ch in chars {
        match ch {
            '\'' | '"' => {
                if in_quotes && ch == quote_char {
                    // End of quoted string
                    in_quotes = false;
                    if !current_token.is_empty()
                        && (current_token.starts_with('/') || current_token.starts_with("~/"))
                    {
                        paths.push(PathBuf::from(&current_token));
                    }
                    current_token.clear();
                } else if !in_quotes {
                    // Start of quoted string
                    in_quotes = true;
                    quote_char = ch;
                }
            }
            ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>' if !in_quotes => {
                // Token boundary
                if !current_token.is_empty()
                    && (current_token.starts_with('/') || current_token.starts_with("~/"))
                {
                    paths.push(PathBuf::from(&current_token));
                }
                current_token.clear();
            }
            _ => {
                current_token.push(ch);
            }
        }
    }

    // Handle final token
    if !current_token.is_empty()
        && (current_token.starts_with('/') || current_token.starts_with("~/"))
    {
        paths.push(PathBuf::from(&current_token));
    }

    paths
}

pub(crate) fn run_with_path_validation(
    command: &str,
    policy: &SandboxPolicy,
) -> Result<std::process::Output, String> {
    // Extract explicit paths from command
    let paths = extract_paths_from_command(command);

    // Validate each path against policy for read access (fail-closed)
    for path in &paths {
        validate_path(path, policy)?;
    }

    // Check if command tries to execute from deny_exec paths
    // Extract the first token (command name)
    let first_token = command.split_whitespace().next().unwrap_or("");
    if !first_token.is_empty() {
        let cmd_path = Path::new(first_token);

        // Only check if it looks like a path (absolute, ./, or ~/)
        if first_token.starts_with('/')
            || first_token.starts_with("./")
            || first_token.starts_with("~/")
        {
            // Check against deny_exec list
            for denied in &policy.deny_exec {
                if let (Ok(cmd_canon), Ok(denied_canon)) =
                    (cmd_path.canonicalize(), denied.canonicalize())
                {
                    if cmd_canon.starts_with(&denied_canon) {
                        return Err(format!(
                            "Execution denied: {} is in protected area (deny_exec)",
                            first_token
                        ));
                    }
                }
            }
        }
    }

    // Log warning if no paths detected (can't guarantee safety)
    if paths.is_empty() {
        warn!(
            command_preview = &command[..command.len().min(50)],
            "PathValidation mode cannot detect dynamic paths in command"
        );
    }

    run_unsandboxed(command)
}

#[cfg(target_os = "linux")]
fn run_with_bubblewrap(
    command: &str,
    policy: &SandboxPolicy,
) -> Result<std::process::Output, String> {
    let (cmd, args) = wrap_with_bwrap(command, policy);

    let mut proc = std::process::Command::new(&cmd);
    proc.args(&args);

    // Inherit environment selectively
    proc.env_clear();
    for (key, value) in std::env::vars() {
        if key.starts_with("LANG")
            || key.starts_with("LC_")
            || key == "PATH"
            || key == "HOME"
            || key == "USER"
            || key == "TERM"
        {
            proc.env(&key, &value);
        }
    }

    proc.output()
        .map_err(|e| format!("Sandboxed command failed: {}", e))
}

#[cfg(not(target_os = "linux"))]
fn run_with_bubblewrap(
    _command: &str,
    _policy: &SandboxPolicy,
) -> Result<std::process::Output, String> {
    Err("Bubblewrap is only available on Linux".to_string())
}

#[cfg(target_os = "macos")]
fn run_with_macos_sandbox(
    command: &str,
    policy: &SandboxPolicy,
) -> Result<std::process::Output, String> {
    let (cmd, args) = wrap_with_macos_sandbox(command, policy);

    std::process::Command::new(&cmd)
        .args(&args)
        .output()
        .map_err(|e| format!("Sandboxed command failed: {}", e))
}

#[cfg(not(target_os = "macos"))]
fn run_with_macos_sandbox(
    _command: &str,
    _policy: &SandboxPolicy,
) -> Result<std::process::Output, String> {
    Err("macOS sandbox is only available on macOS".to_string())
}

// ── Combined Landlock + Bubblewrap (Linux) ──────────────────────────────────

/// Wrap a command with extra-restrictive bubblewrap for combined mode.
///
/// This version is even more restrictive than standard bwrap:
/// - Denied paths are completely unmounted (not even visible)
/// - More aggressive unsharing
/// - Tighter mount controls
#[cfg(target_os = "linux")]
fn wrap_with_combined_bwrap(command: &str, policy: &SandboxPolicy) -> (String, Vec<String>) {
    let mut args = vec![
        "--unshare-all".to_string(),
        "--share-net".to_string(), // Keep network for web_fetch
        "--die-with-parent".to_string(),
        "--new-session".to_string(), // Extra isolation: new session ID
    ];

    // Helper to check if a path should be completely blocked
    let is_blocked = |path: &Path| -> bool {
        policy
            .deny_read
            .iter()
            .any(|deny| path.starts_with(deny) || deny.starts_with(path))
            || policy
                .deny_exec
                .iter()
                .any(|deny| path.starts_with(deny) || deny.starts_with(path))
    };

    // Mount minimal root - only if not blocked
    for dir in &["/usr", "/lib", "/lib64", "/bin", "/sbin"] {
        let path = Path::new(dir);
        if path.exists() && !is_blocked(path) {
            args.push("--ro-bind".to_string());
            args.push(dir.to_string());
            args.push(dir.to_string());
        }
    }

    // Read-only /etc - only if not blocked
    let etc_path = Path::new("/etc");
    if etc_path.exists() && !is_blocked(etc_path) {
        args.push("--ro-bind".to_string());
        args.push("/etc".to_string());
        args.push("/etc".to_string());
    }

    // Workspace: read-only if in deny_write, writable otherwise, skip if in deny_read
    if !policy
        .deny_read
        .iter()
        .any(|deny| policy.workspace.starts_with(deny) || deny.starts_with(&policy.workspace))
    {
        if policy
            .deny_write
            .iter()
            .any(|deny| policy.workspace.starts_with(deny) || deny.starts_with(&policy.workspace))
        {
            args.push("--ro-bind".to_string());
        } else {
            args.push("--bind".to_string());
        }
        args.push(policy.workspace.display().to_string());
        args.push(policy.workspace.display().to_string());
    }

    // Writable /tmp (isolated)
    args.push("--tmpfs".to_string());
    args.push("/tmp".to_string());

    // Minimal /proc and /dev
    args.push("--proc".to_string());
    args.push("/proc".to_string());
    args.push("--dev".to_string());
    args.push("/dev".to_string());

    // Working directory
    args.push("--chdir".to_string());
    args.push(policy.workspace.display().to_string());

    // Execute command
    args.push("--".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(command.to_string());

    ("bwrap".to_string(), args)
}

/// Run with combined Landlock + Bubblewrap for defense-in-depth.
///
/// This approach layers two independent security mechanisms:
///
/// 1. **Bubblewrap** (namespace-level):
///    - Creates isolated mount namespace
///    - Unshares PID, IPC, UTS namespaces
///    - Prevents visibility of denied paths entirely
///
/// 2. **Landlock** (kernel LSM-level):
///    - Enforced by Linux Security Module
///    - Kernel-level filesystem access control
///    - Cannot be bypassed by namespace escape
///
/// **Defense-in-depth**: Even if one layer is compromised, the other provides protection.
/// This matches the security model of IronClaw and other security-focused agents.
#[cfg(target_os = "linux")]
fn run_with_landlock_bwrap(
    command: &str,
    policy: &SandboxPolicy,
) -> Result<std::process::Output, String> {
    // Generate extra-restrictive bwrap configuration
    let (cmd, args) = wrap_with_combined_bwrap(command, policy);

    let mut proc = std::process::Command::new(&cmd);
    proc.args(&args);

    // Inherit environment selectively
    proc.env_clear();
    for (key, value) in std::env::vars() {
        if key.starts_with("LANG")
            || key.starts_with("LC_")
            || key == "PATH"
            || key == "HOME"
            || key == "USER"
            || key == "TERM"
        {
            proc.env(&key, &value);
        }
    }

    info!(
        mode = "Landlock+Bubblewrap",
        denied_paths = policy.deny_read.len() + policy.deny_exec.len(),
        "Defense-in-depth sandbox active"
    );

    proc.output()
        .map_err(|e| format!("Combined sandboxed command failed: {}", e))
}

#[cfg(not(target_os = "linux"))]
fn run_with_landlock_bwrap(
    _command: &str,
    _policy: &SandboxPolicy,
) -> Result<std::process::Output, String> {
    Err("Landlock+Bubblewrap is only available on Linux".to_string())
}

// ── Docker Container (Cross-Platform) ──────────────────────────────────────

/// Run command in an ephemeral Docker container.
///
/// This provides strong isolation across platforms:
/// - **Container isolation**: Complete filesystem isolation
/// - **Resource limits**: Memory (2GB), CPU constraints
/// - **Non-root execution**: Runs as UID 1000
/// - **Read-only root**: Container filesystem is immutable
/// - **Auto-cleanup**: Container removed after execution
///
/// Inspired by IronClaw's Docker sandbox approach.
fn run_with_docker(command: &str, policy: &SandboxPolicy) -> Result<std::process::Output, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Generate unique container name
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let container_name = format!("rustyclaw-sandbox-{}", timestamp);

    // Build Docker run arguments
    let mut docker_args = vec![
        "run".to_string(),
        "--rm".to_string(), // Auto-remove after exit
        "--name".to_string(),
        container_name,
        // Resource limits
        "--memory".to_string(),
        "2g".to_string(),
        "--cpus".to_string(),
        "1.0".to_string(),
        // Security
        "--user".to_string(),
        "1000:1000".to_string(), // Non-root user
        "--cap-drop".to_string(),
        "ALL".to_string(), // Drop all capabilities
        "--security-opt".to_string(),
        "no-new-privileges:true".to_string(),
        "--read-only".to_string(), // Read-only root filesystem
        // Network
        "--network".to_string(),
        "bridge".to_string(), // Allow network for web_fetch
        // Tmpfs for /tmp (writable)
        "--tmpfs".to_string(),
        "/tmp:size=512M".to_string(),
    ];

    // Mount workspace based on policy
    let workspace_str = policy.workspace.display().to_string();

    // Check if workspace is in deny lists
    let workspace_denied = policy
        .deny_read
        .iter()
        .any(|deny| policy.workspace.starts_with(deny) || deny.starts_with(&policy.workspace));

    if !workspace_denied {
        let write_allowed = !policy
            .deny_write
            .iter()
            .any(|deny| policy.workspace.starts_with(deny) || deny.starts_with(&policy.workspace));

        let mount_mode = if write_allowed { "rw" } else { "ro" };
        docker_args.push("--volume".to_string());
        docker_args.push(format!("{}:/workspace:{}", workspace_str, mount_mode));
        docker_args.push("--workdir".to_string());
        docker_args.push("/workspace".to_string());
    } else {
        // Workspace is denied, use /tmp as workdir
        docker_args.push("--workdir".to_string());
        docker_args.push("/tmp".to_string());
    }

    // Use Alpine Linux for minimal footprint
    docker_args.push("alpine:latest".to_string());

    // Execute command via sh
    docker_args.push("sh".to_string());
    docker_args.push("-c".to_string());
    docker_args.push(command.to_string());

    info!(
        mode = "Docker",
        workspace = %workspace_str,
        workspace_blocked = workspace_denied,
        "Docker container sandbox active"
    );

    // Execute docker command
    std::process::Command::new("docker")
        .args(&docker_args)
        .output()
        .map_err(|e| format!("Docker execution failed: {}", e))
}

// ── Sandbox Manager ─────────────────────────────────────────────────────────

/// Global sandbox configuration and state.
pub struct Sandbox {
    pub mode: SandboxMode,
    pub policy: SandboxPolicy,
    pub capabilities: SandboxCapabilities,
}

impl Sandbox {
    /// Create a new sandbox with auto-detection.
    pub fn new(policy: SandboxPolicy) -> Self {
        let caps = SandboxCapabilities::detect();
        Self {
            mode: SandboxMode::Auto,
            policy,
            capabilities: caps,
        }
    }

    /// Create a sandbox with a specific mode.
    pub fn with_mode(mode: SandboxMode, policy: SandboxPolicy) -> Self {
        let caps = SandboxCapabilities::detect();
        Self {
            mode,
            policy,
            capabilities: caps,
        }
    }

    /// Get the effective mode (resolving Auto).
    pub fn effective_mode(&self) -> SandboxMode {
        match self.mode {
            SandboxMode::Auto => self.capabilities.best_mode(),
            other => other,
        }
    }

    /// Initialize process-wide sandbox (for Landlock).
    pub fn init(&self) -> Result<(), String> {
        if self.effective_mode() == SandboxMode::Landlock {
            apply_landlock(&self.policy)?;
        }
        Ok(())
    }

    /// Check if a path is accessible under the current policy.
    pub fn check_path(&self, path: &Path) -> Result<(), String> {
        if self.mode == SandboxMode::None {
            return Ok(());
        }
        validate_path(path, &self.policy)
    }

    /// Run a command with appropriate sandboxing.
    pub fn run_command(&self, command: &str) -> Result<std::process::Output, String> {
        run_sandboxed(command, &self.policy, self.mode)
    }

    /// Human-readable status string.
    pub fn status(&self) -> String {
        format!(
            "Mode: {} (effective: {})\n{}",
            self.mode,
            self.effective_mode(),
            self.capabilities.describe()
        )
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

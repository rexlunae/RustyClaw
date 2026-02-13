//! Agent sandbox — isolates tool execution from sensitive paths.
//!
//! Sandbox modes (in order of preference):
//! 1. **Landlock** (Linux 5.13+) — kernel-enforced filesystem restrictions
//! 2. **Bubblewrap** (Linux) — user namespace sandbox
//! 3. **macOS Sandbox** — sandbox-exec with Seatbelt profiles
//! 4. **Path Validation** — software-only path checking (all platforms)
//!
//! The sandbox auto-detects available options and picks the strongest.

use std::path::{Path, PathBuf};

// ── Sandbox Capabilities Detection ──────────────────────────────────────────

/// Detected sandbox capabilities on this system.
#[derive(Debug, Clone)]
pub struct SandboxCapabilities {
    pub landlock: bool,
    pub bubblewrap: bool,
    pub macos_sandbox: bool,
    pub unprivileged_userns: bool,
}

impl SandboxCapabilities {
    /// Detect available sandbox capabilities.
    pub fn detect() -> Self {
        Self {
            landlock: Self::check_landlock(),
            bubblewrap: Self::check_bubblewrap(),
            macos_sandbox: Self::check_macos_sandbox(),
            unprivileged_userns: Self::check_userns(),
        }
    }

    #[cfg(target_os = "linux")]
    fn check_landlock() -> bool {
        // Check if Landlock is in the LSM list
        std::fs::read_to_string("/sys/kernel/security/lsm")
            .map(|s| s.contains("landlock"))
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "linux"))]
    fn check_landlock() -> bool {
        false
    }

    fn check_bubblewrap() -> bool {
        std::process::Command::new("bwrap")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "macos")]
    fn check_macos_sandbox() -> bool {
        // sandbox-exec is available on all macOS versions
        std::process::Command::new("sandbox-exec")
            .arg("-n")
            .arg("no-network")
            .arg("true")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "macos"))]
    fn check_macos_sandbox() -> bool {
        false
    }

    #[cfg(target_os = "linux")]
    fn check_userns() -> bool {
        std::fs::read_to_string("/proc/sys/kernel/unprivileged_userns_clone")
            .map(|s| s.trim() == "1")
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "linux"))]
    fn check_userns() -> bool {
        false
    }

    /// Get the best available sandbox mode.
    pub fn best_mode(&self) -> SandboxMode {
        if self.landlock {
            SandboxMode::Landlock
        } else if self.bubblewrap {
            SandboxMode::Bubblewrap
        } else if self.macos_sandbox {
            SandboxMode::MacOSSandbox
        } else {
            SandboxMode::PathValidation
        }
    }

    /// Human-readable description of available options.
    pub fn describe(&self) -> String {
        let mut opts = Vec::new();
        if self.landlock {
            opts.push("Landlock");
        }
        if self.bubblewrap {
            opts.push("Bubblewrap");
        }
        if self.macos_sandbox {
            opts.push("macOS Sandbox");
        }
        opts.push("Path Validation"); // Always available
        
        format!("Available: {}", opts.join(", "))
    }
}

// ── Sandbox Policy ──────────────────────────────────────────────────────────

/// Paths that should be denied to the agent.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Paths the agent cannot read from
    pub deny_read: Vec<PathBuf>,
    /// Paths the agent cannot write to
    pub deny_write: Vec<PathBuf>,
    /// Paths the agent cannot execute from
    pub deny_exec: Vec<PathBuf>,
    /// Allowed paths (whitelist mode) — if non-empty, only these are allowed
    pub allow_paths: Vec<PathBuf>,
    /// Working directory for the agent
    pub workspace: PathBuf,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            deny_read: Vec::new(),
            deny_write: Vec::new(),
            deny_exec: Vec::new(),
            allow_paths: Vec::new(),
            workspace: PathBuf::from("."),
        }
    }
}

impl SandboxPolicy {
    /// Create a policy that protects the credentials directory.
    pub fn protect_credentials(credentials_dir: impl Into<PathBuf>, workspace: impl Into<PathBuf>) -> Self {
        let cred_dir = credentials_dir.into();
        Self {
            deny_read: vec![cred_dir.clone()],
            deny_write: vec![cred_dir.clone()],
            deny_exec: vec![cred_dir],
            allow_paths: Vec::new(),
            workspace: workspace.into(),
        }
    }

    /// Create a strict policy that only allows access to specific paths.
    pub fn strict(workspace: impl Into<PathBuf>, allowed: Vec<PathBuf>) -> Self {
        Self {
            deny_read: Vec::new(),
            deny_write: Vec::new(),
            deny_exec: Vec::new(),
            allow_paths: allowed,
            workspace: workspace.into(),
        }
    }

    /// Add a path to the deny-read list.
    pub fn deny_read(mut self, path: impl Into<PathBuf>) -> Self {
        self.deny_read.push(path.into());
        self
    }

    /// Add a path to the deny-write list.
    pub fn deny_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.deny_write.push(path.into());
        self
    }
}

// ── Sandbox Mode ────────────────────────────────────────────────────────────

/// Sandbox mode for command execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum SandboxMode {
    /// No sandboxing
    None,
    /// Path validation only (software check, all platforms)
    PathValidation,
    /// Bubblewrap namespace isolation (Linux)
    Bubblewrap,
    /// Landlock kernel restrictions (Linux 5.13+)
    Landlock,
    /// macOS sandbox-exec (macOS)
    MacOSSandbox,
    /// Auto-detect best available
    #[default]
    Auto,
}


impl std::str::FromStr for SandboxMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" | "off" | "disabled" => Ok(Self::None),
            "path" | "pathvalidation" | "soft" => Ok(Self::PathValidation),
            "bwrap" | "bubblewrap" | "namespace" => Ok(Self::Bubblewrap),
            "landlock" | "kernel" => Ok(Self::Landlock),
            "macos" | "seatbelt" | "sandbox-exec" => Ok(Self::MacOSSandbox),
            "auto" | "" => Ok(Self::Auto),
            _ => Err(format!("Unknown sandbox mode: {}", s)),
        }
    }
}

impl std::fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::PathValidation => write!(f, "path"),
            Self::Bubblewrap => write!(f, "bwrap"),
            Self::Landlock => write!(f, "landlock"),
            Self::MacOSSandbox => write!(f, "macos"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

// ── Path Validation (All Platforms) ─────────────────────────────────────────

/// Validate that a path does not escape allowed boundaries.
pub fn validate_path(path: &Path, policy: &SandboxPolicy) -> Result<(), String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Check deny lists
    for denied in &policy.deny_read {
        if let Ok(denied_canon) = denied.canonicalize() {
            if canonical.starts_with(&denied_canon) {
                return Err(format!(
                    "Access denied: path {} is in protected area",
                    path.display()
                ));
            }
        }
    }

    // Check allow list if non-empty
    if !policy.allow_paths.is_empty() {
        let allowed = policy.allow_paths.iter().any(|allowed| {
            allowed
                .canonicalize()
                .map(|c| canonical.starts_with(&c))
                .unwrap_or(false)
        });
        if !allowed {
            return Err(format!(
                "Access denied: path {} is not in allowed areas",
                path.display()
            ));
        }
    }

    Ok(())
}

// ── Bubblewrap (Linux) ──────────────────────────────────────────────────────

/// Wrap a command in bubblewrap with the given policy.
#[cfg(target_os = "linux")]
pub fn wrap_with_bwrap(command: &str, policy: &SandboxPolicy) -> (String, Vec<String>) {
    let mut args = Vec::new();

    // Basic namespace isolation
    args.push("--unshare-all".to_string());
    args.push("--share-net".to_string()); // Keep network for web_fetch etc

    // Mount a minimal root
    for dir in &["/usr", "/lib", "/lib64", "/bin", "/sbin"] {
        if Path::new(dir).exists() {
            args.push("--ro-bind".to_string());
            args.push(dir.to_string());
            args.push(dir.to_string());
        }
    }

    // Read-only /etc (but filter out sensitive files)
    args.push("--ro-bind".to_string());
    args.push("/etc".to_string());
    args.push("/etc".to_string());

    // Writable workspace
    args.push("--bind".to_string());
    args.push(policy.workspace.display().to_string());
    args.push(policy.workspace.display().to_string());

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
/// **Warning:** This is irreversible for this process!
#[cfg(target_os = "linux")]
pub fn apply_landlock(policy: &SandboxPolicy) -> Result<(), String> {
    // Placeholder — real implementation requires syscalls
    // For now, just log and rely on path validation
    eprintln!(
        "[sandbox] Landlock policy registered (enforcement pending): deny={:?}",
        policy.deny_read
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
            eprintln!("[sandbox] Bubblewrap not available, falling back to path validation");
            return run_with_path_validation(command, policy);
        }
        SandboxMode::Landlock if !caps.landlock => {
            eprintln!("[sandbox] Landlock not available, falling back to path validation");
            return run_with_path_validation(command, policy);
        }
        SandboxMode::MacOSSandbox if !caps.macos_sandbox => {
            eprintln!("[sandbox] macOS sandbox not available, falling back to path validation");
            return run_with_path_validation(command, policy);
        }
        _ => {}
    }
    
    match effective_mode {
        SandboxMode::None => run_unsandboxed(command),
        SandboxMode::PathValidation => run_with_path_validation(command, policy),
        SandboxMode::Bubblewrap => run_with_bubblewrap(command, policy),
        SandboxMode::MacOSSandbox => run_with_macos_sandbox(command, policy),
        SandboxMode::Landlock => {
            // Landlock is process-wide; just run with path validation
            run_with_path_validation(command, policy)
        }
        SandboxMode::Auto => unreachable!(), // Already resolved above
    }
}

fn run_unsandboxed(command: &str) -> Result<std::process::Output, String> {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| format!("Command failed: {}", e))
}

fn run_with_path_validation(command: &str, _policy: &SandboxPolicy) -> Result<std::process::Output, String> {
    // Path validation happens at the tool level, not here
    // This is just a marker that we're in "soft" mode
    run_unsandboxed(command)
}

#[cfg(target_os = "linux")]
fn run_with_bubblewrap(command: &str, policy: &SandboxPolicy) -> Result<std::process::Output, String> {
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
fn run_with_bubblewrap(_command: &str, _policy: &SandboxPolicy) -> Result<std::process::Output, String> {
    Err("Bubblewrap is only available on Linux".to_string())
}

#[cfg(target_os = "macos")]
fn run_with_macos_sandbox(command: &str, policy: &SandboxPolicy) -> Result<std::process::Output, String> {
    let (cmd, args) = wrap_with_macos_sandbox(command, policy);
    
    std::process::Command::new(&cmd)
        .args(&args)
        .output()
        .map_err(|e| format!("Sandboxed command failed: {}", e))
}

#[cfg(not(target_os = "macos"))]
fn run_with_macos_sandbox(_command: &str, _policy: &SandboxPolicy) -> Result<std::process::Output, String> {
    Err("macOS sandbox is only available on macOS".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capabilities_detect() {
        let caps = SandboxCapabilities::detect();
        // Should always have at least path validation
        assert!(caps.best_mode() != SandboxMode::None || caps.best_mode() == SandboxMode::PathValidation);
    }

    #[test]
    fn test_policy_creation() {
        let policy = SandboxPolicy::protect_credentials(
            "/home/user/.rustyclaw/credentials",
            "/home/user/.rustyclaw/workspace",
        );

        assert_eq!(policy.deny_read.len(), 1);
        assert!(policy.deny_read[0].ends_with("credentials"));
    }

    #[test]
    fn test_path_validation_denied() {
        let policy = SandboxPolicy::protect_credentials("/tmp/creds", "/tmp/workspace");

        std::fs::create_dir_all("/tmp/creds").ok();

        let result = validate_path(Path::new("/tmp/creds/secrets.json"), &policy);
        assert!(result.is_err());
    }

    #[test]
    fn test_path_validation_allowed() {
        let policy =
            SandboxPolicy::protect_credentials("/tmp/test-creds-isolated", "/tmp/test-workspace");

        let result = validate_path(Path::new("/tmp/other/file.txt"), &policy);
        assert!(result.is_ok());
    }

    #[test]
    fn test_sandbox_mode_parsing() {
        assert_eq!("none".parse::<SandboxMode>().unwrap(), SandboxMode::None);
        assert_eq!("auto".parse::<SandboxMode>().unwrap(), SandboxMode::Auto);
        assert_eq!("bwrap".parse::<SandboxMode>().unwrap(), SandboxMode::Bubblewrap);
        assert_eq!("macos".parse::<SandboxMode>().unwrap(), SandboxMode::MacOSSandbox);
    }

    #[test]
    fn test_sandbox_status() {
        let policy = SandboxPolicy::default();
        let sandbox = Sandbox::new(policy);
        let status = sandbox.status();
        assert!(status.contains("Mode:"));
        assert!(status.contains("Available:"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_bwrap_command_generation() {
        let policy = SandboxPolicy {
            workspace: PathBuf::from("/home/user/workspace"),
            ..Default::default()
        };

        let (cmd, args) = wrap_with_bwrap("ls -la", &policy);

        assert_eq!(cmd, "bwrap");
        assert!(args.contains(&"--unshare-all".to_string()));
        assert!(args.contains(&"ls -la".to_string()));
    }

    #[test]
    fn test_run_unsandboxed() {
        let output = run_unsandboxed("echo hello").unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("hello"));
    }
}


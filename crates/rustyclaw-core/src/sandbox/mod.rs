//! Agent sandbox — isolates tool execution from sensitive paths.
//!
//! Sandbox modes (in order of preference):
//! 1. **Landlock + Bubblewrap** (Linux 5.13+) — combined defense-in-depth
//! 2. **Landlock** (Linux 5.13+) — kernel-enforced filesystem restrictions
//! 3. **Bubblewrap** (Linux) — user namespace sandbox
//! 4. **Docker** (cross-platform) — container isolation with resource limits
//! 5. **macOS Sandbox** — sandbox-exec with Seatbelt profiles
//! 6. **Path Validation** — software-only path checking (all platforms)
//!
//! The sandbox auto-detects available options and picks the strongest.

use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// ── Sandbox Capabilities Detection ──────────────────────────────────────────

/// Detected sandbox capabilities on this system.
#[derive(Debug, Clone)]
pub struct SandboxCapabilities {
    pub landlock: bool,
    pub bubblewrap: bool,
    pub macos_sandbox: bool,
    pub unprivileged_userns: bool,
    pub docker: bool,
}

impl SandboxCapabilities {
    /// Detect available sandbox capabilities, once per process.
    ///
    /// [`Self::detect`] shells out three times, including `docker info`, which
    /// takes seconds against an unreachable daemon. Callers on an async
    /// runtime were paying that on every MCP server connection, blocking a
    /// worker thread each time, for an answer that cannot change while the
    /// process runs.
    pub fn cached() -> &'static Self {
        static CAPABILITIES: std::sync::OnceLock<SandboxCapabilities> = std::sync::OnceLock::new();
        CAPABILITIES.get_or_init(Self::detect)
    }

    /// Detect available sandbox capabilities.
    pub fn detect() -> Self {
        Self {
            landlock: Self::check_landlock(),
            bubblewrap: Self::check_bubblewrap(),
            macos_sandbox: Self::check_macos_sandbox(),
            unprivileged_userns: Self::check_userns(),
            docker: Self::check_docker(),
        }
    }

    fn check_docker() -> bool {
        std::process::Command::new("docker")
            .arg("info")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
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

    /// Whether bubblewrap can actually build a sandbox here.
    ///
    /// `bwrap --version` only says the binary exists. It succeeds on hosts
    /// where every real invocation then fails — user namespaces disabled,
    /// seccomp policy, a container without the right capabilities. Confinement
    /// is a security control, so "installed" is not the question worth
    /// answering; "does it work" is. This runs a trivial sandbox and looks at
    /// the exit status.
    fn check_bubblewrap() -> bool {
        std::process::Command::new("bwrap")
            .args([
                "--unshare-user",
                "--unshare-pid",
                "--ro-bind",
                "/",
                "/",
                "--",
                "true",
            ])
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
        userns_allowed(
            std::fs::read_to_string("/proc/sys/kernel/unprivileged_userns_clone").ok(),
            std::fs::read_to_string("/proc/sys/user/max_user_namespaces").ok(),
        )
    }

    #[cfg(not(target_os = "linux"))]
    fn check_userns() -> bool {
        false
    }

    /// Get the best available sandbox mode.
    pub fn best_mode(&self) -> SandboxMode {
        // Prefer combined Landlock + Bubblewrap for maximum security on Linux
        if self.landlock && self.bubblewrap {
            SandboxMode::LandlockBwrap
        } else if self.landlock {
            SandboxMode::Landlock
        } else if self.bubblewrap {
            SandboxMode::Bubblewrap
        } else if self.docker {
            // Docker is cross-platform and provides good isolation
            SandboxMode::Docker
        } else if self.macos_sandbox {
            SandboxMode::MacOSSandbox
        } else {
            SandboxMode::PathValidation
        }
    }

    /// Human-readable description of available options.
    pub fn describe(&self) -> String {
        let mut opts = Vec::new();
        if self.landlock && self.bubblewrap {
            opts.push("Landlock+Bubblewrap");
        }
        if self.landlock {
            opts.push("Landlock");
        }
        if self.bubblewrap {
            opts.push("Bubblewrap");
        }
        if self.docker {
            opts.push("Docker");
        }
        if self.macos_sandbox {
            opts.push("macOS Sandbox");
        }
        opts.push("Path Validation"); // Always available

        format!("Available: {}", opts.join(", "))
    }
}

/// Whether unprivileged user namespaces are permitted, from the two kernel
/// knobs that can say otherwise.
///
/// The previous version read only `/proc/sys/kernel/unprivileged_userns_clone`
/// and treated a failed read as "denied". That sysctl is a Debian/Ubuntu
/// downstream patch — on Fedora, Arch, RHEL and mainline the file does not
/// exist, so every one of those hosts reported user namespaces as unavailable.
/// Nothing consumed the field until MCP confinement did, at which point the
/// inaccuracy became a silent fail-open on most of Linux.
///
/// Absent now means permitted, which is what mainline means by it. Only an
/// explicit `0` from either knob is a denial.
#[cfg(target_os = "linux")]
pub(crate) fn userns_allowed(clone_sysctl: Option<String>, max_namespaces: Option<String>) -> bool {
    if clone_sysctl.is_some_and(|v| v.trim() == "0") {
        return false;
    }
    if max_namespaces.is_some_and(|v| v.trim().parse::<u64>() == Ok(0)) {
        return false;
    }
    true
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
    /// Working directory the sandbox grants access to.
    ///
    /// In the global policy this is only a default: every execution site
    /// clones the policy and substitutes the actual per-command working
    /// directory before wrapping, so confinement follows the turn that is
    /// running, not the directory the gateway happened to start in.
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
    pub fn protect_credentials(
        credentials_dir: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
    ) -> Self {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxMode {
    /// No sandboxing
    None,
    /// Path validation only (software check, all platforms)
    PathValidation,
    /// Bubblewrap namespace isolation (Linux)
    Bubblewrap,
    /// Landlock kernel restrictions (Linux 5.13+)
    Landlock,
    /// Combined Landlock + Bubblewrap (Linux, defense-in-depth)
    LandlockBwrap,
    /// Docker container isolation (cross-platform)
    Docker,
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
            "landlock+bwrap" | "landlock-bwrap" | "combined" | "lockwrap" => {
                Ok(Self::LandlockBwrap)
            }
            "docker" | "container" => Ok(Self::Docker),
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
            Self::LandlockBwrap => write!(f, "landlock+bwrap"),
            Self::Docker => write!(f, "docker"),
            Self::MacOSSandbox => write!(f, "macos"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Typed sandbox errors.
///
/// Policy verdicts (`ReadDenied`, `NotAllowed`, `ExecDenied`, `Command*`,
/// `SymlinkRace`) are deliberate security decisions; the remaining variants
/// are environment or execution failures.  Callers at the AI-tool boundary
/// flatten these with `.to_string()`; internal callers can match on the
/// distinction.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// A path is inside a `deny_read` protected area.
    #[error("Access denied: path {} is in protected area", path.display())]
    ReadDenied { path: PathBuf },
    /// A path falls outside a non-empty allow-list.
    #[error("Access denied: path {} is not in allowed areas", path.display())]
    NotAllowed { path: PathBuf },
    /// Executing a binary from a `deny_exec` protected area.
    #[error("Execution denied: {} is in protected area (deny_exec)", path.display())]
    ExecDenied { path: PathBuf },
    /// The command string contains a null byte.
    #[error("Command contains null byte — blocked for security")]
    CommandNullByte,
    /// The command string exceeds the maximum allowed length.
    #[error("Command too long (max 4096 characters) — blocked for security")]
    CommandTooLong,
    /// The command string matches a credential-exfiltration pattern.
    #[error("Command blocked: contains credential exfiltration pattern")]
    CommandExfiltration,
    /// A path resolved differently on consecutive lookups.
    #[error("Access denied: path changed between resolutions — possible symlink race attack")]
    SymlinkRace,
    /// Canonicalizing a path failed.
    #[error("Path resolution failed for {}: {source}", path.display())]
    PathResolution {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Spawning the (sandboxed) process failed.
    #[error("Sandboxed command failed: {0}")]
    Spawn(#[from] std::io::Error),
    /// The requested sandbox backend is not available on this platform.
    #[error("{0}")]
    Unsupported(&'static str),
    /// Landlock setup failed.
    #[cfg(target_os = "linux")]
    #[error("Landlock error while {context}: {source}")]
    Landlock {
        context: String,
        #[source]
        source: landlock::RulesetError,
    },
    /// A path needed for a Landlock rule could not be opened.
    #[cfg(target_os = "linux")]
    #[error("Cannot open {} for Landlock: {source}", path.display())]
    LandlockPath {
        path: PathBuf,
        #[source]
        source: landlock::PathFdError,
    },
}

// ── Path Validation (All Platforms) ─────────────────────────────────────────

/// Validate that a path does not escape allowed boundaries.
pub fn validate_path(path: &Path, policy: &SandboxPolicy) -> Result<(), SandboxError> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Check deny lists
    for denied in &policy.deny_read {
        if let Ok(denied_canon) = denied.canonicalize() {
            if canonical.starts_with(&denied_canon) {
                return Err(SandboxError::ReadDenied {
                    path: path.to_path_buf(),
                });
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
            return Err(SandboxError::NotAllowed {
                path: path.to_path_buf(),
            });
        }
    }

    Ok(())
}

pub(crate) mod platform;
pub use platform::*;

#[cfg(test)]
mod tests;

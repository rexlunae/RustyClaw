//! System administration tools: package management, network diagnostics,
//! network scanning, service management, user/group management, and firewall control.
//!
//! Split into submodules for maintainability.

mod firewall;
mod net;
mod pkg;
mod service;
mod user;

// Re-export sync functions
pub use firewall::exec_firewall;
pub use net::{exec_net_info, exec_net_scan};
pub use pkg::exec_pkg_manage;
pub use service::exec_service_manage;
pub use user::exec_user_manage;

// Re-export async functions
pub use firewall::exec_firewall_async;
pub use net::{exec_net_info_async, exec_net_scan_async};
pub use pkg::exec_pkg_manage_async;
pub use service::exec_service_manage_async;
pub use user::exec_user_manage_async;

// ── Shared helpers ──────────────────────────────────────────────────────────

use std::process::Command;

/// Run a shell pipeline via `sh -c` and return stdout (trimmed) - sync version.
pub(crate) fn sh(script: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Command exited with {}", output.status)
        } else {
            stderr
        });
    }

    if !stderr.is_empty() && !stdout.is_empty() {
        Ok(format!("{}\n[stderr] {}", stdout, stderr))
    } else if !stdout.is_empty() {
        Ok(stdout)
    } else {
        Ok(stderr)
    }
}

/// Run a shell pipeline via `sh -c` - async version.
pub(crate) async fn sh_async(script: &str) -> Result<String, String> {
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .await
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Command exited with {}", output.status)
        } else {
            stderr
        });
    }

    if !stderr.is_empty() && !stdout.is_empty() {
        Ok(format!("{}\n[stderr] {}", stdout, stderr))
    } else if !stdout.is_empty() {
        Ok(stdout)
    } else {
        Ok(stderr)
    }
}

/// Detect which command is available from a list (sync).
pub(crate) fn which_first(cmds: &[&str]) -> Option<String> {
    for cmd in cmds {
        if Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some((*cmd).to_string());
        }
    }
    None
}

/// Detect which command is available from a list (async).
pub(crate) async fn which_first_async(cmds: &[&str]) -> Option<String> {
    for cmd in cmds {
        if tokio::process::Command::new("which")
            .arg(cmd)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some((*cmd).to_string());
        }
    }
    None
}

/// Detect the package manager on this system (sync).
pub(crate) fn detect_pkg_manager() -> (&'static str, &'static str) {
    let managers: &[(&str, &str)] = &[
        ("brew", "Homebrew"),
        ("apt", "APT"),
        ("apt-get", "APT"),
        ("dnf", "DNF"),
        ("yum", "YUM"),
        ("pacman", "Pacman"),
        ("zypper", "Zypper"),
        ("apk", "Alpine APK"),
        ("pkg", "FreeBSD pkg"),
        ("nix-env", "Nix"),
        ("snap", "Snap"),
        ("flatpak", "Flatpak"),
        ("port", "MacPorts"),
    ];

    for (cmd, name) in managers {
        if Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return (cmd, name);
        }
    }
    ("", "none")
}

/// Detect the package manager on this system (async).
pub(crate) async fn detect_pkg_manager_async() -> (&'static str, &'static str) {
    let managers: &[(&str, &str)] = &[
        ("brew", "Homebrew"),
        ("apt", "APT"),
        ("apt-get", "APT"),
        ("dnf", "DNF"),
        ("yum", "YUM"),
        ("pacman", "Pacman"),
        ("zypper", "Zypper"),
        ("apk", "Alpine APK"),
        ("pkg", "FreeBSD pkg"),
        ("nix-env", "Nix"),
        ("snap", "Snap"),
        ("flatpak", "Flatpak"),
        ("port", "MacPorts"),
    ];

    for (cmd, name) in managers {
        if tokio::process::Command::new("which")
            .arg(cmd)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return (cmd, name);
        }
    }
    ("", "none")
}

/// Detect the init/service system (sync).
pub(crate) fn detect_service_manager() -> &'static str {
    if Command::new("which")
        .arg("systemctl")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "systemd";
    }
    if Command::new("which")
        .arg("launchctl")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "launchd";
    }
    if std::path::Path::new("/etc/init.d").exists() {
        return "sysvinit";
    }
    "unknown"
}

/// Detect the init/service system (async).
pub(crate) async fn detect_service_manager_async() -> &'static str {
    if tokio::process::Command::new("which")
        .arg("systemctl")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "systemd";
    }
    if tokio::process::Command::new("which")
        .arg("launchctl")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "launchd";
    }
    if std::path::Path::new("/etc/init.d").exists() {
        return "sysvinit";
    }
    "unknown"
}

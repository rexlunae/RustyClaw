//! System service management for gateway lifecycle.
//!
//! Provides systemd (Linux) and launchd (macOS) service file generation
//! and installation for persistent gateway operation.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Service management platform
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePlatform {
    Systemd,  // Linux
    Launchd,  // macOS
    Unsupported,
}

impl ServicePlatform {
    /// Detect the current platform's service manager
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            // Check if systemd is available
            if Command::new("systemctl")
                .arg("--version")
                .output()
                .is_ok()
            {
                return Self::Systemd;
            }
        }

        #[cfg(target_os = "macos")]
        {
            return Self::Launchd;
        }

        Self::Unsupported
    }
}

/// Service installation result
#[derive(Debug)]
pub struct InstallResult {
    pub platform: ServicePlatform,
    pub service_path: PathBuf,
    pub message: String,
}

/// Generate systemd unit file content
fn generate_systemd_unit(
    binary_path: &Path,
    settings_dir: &Path,
    port: u16,
    bind: &str,
) -> String {
    format!(
        r#"[Unit]
Description=RustyClaw Gateway
After=network.target

[Service]
Type=simple
ExecStart={binary} gateway run --port {port} --bind {bind} --settings-dir {settings_dir}
Restart=always
RestartSec=10
StandardOutput=append:{log_path}
StandardError=append:{log_path}
SyslogIdentifier=rustyclaw-gateway

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths={settings_dir}

[Install]
WantedBy=multi-user.target
"#,
        binary = binary_path.display(),
        port = port,
        bind = bind,
        settings_dir = settings_dir.display(),
        log_path = settings_dir.join("logs").join("gateway.log").display(),
    )
}

/// Generate launchd plist file content
fn generate_launchd_plist(
    binary_path: &Path,
    settings_dir: &Path,
    port: u16,
    bind: &str,
) -> String {
    let log_path = settings_dir.join("logs").join("gateway.log");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.rustyclaw.gateway</string>

    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>gateway</string>
        <string>run</string>
        <string>--port</string>
        <string>{port}</string>
        <string>--bind</string>
        <string>{bind}</string>
        <string>--settings-dir</string>
        <string>{settings_dir}</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>{log_path}</string>

    <key>StandardErrorPath</key>
    <string>{log_path}</string>

    <key>WorkingDirectory</key>
    <string>{settings_dir}</string>

    <key>ThrottleInterval</key>
    <integer>10</integer>
</dict>
</plist>
"#,
        binary = binary_path.display(),
        port = port,
        bind = bind,
        settings_dir = settings_dir.display(),
        log_path = log_path.display(),
    )
}

/// Install gateway as a system service
pub fn install_service(
    settings_dir: &Path,
    port: u16,
    bind: &str,
    enable: bool,
    start: bool,
) -> Result<InstallResult> {
    let platform = ServicePlatform::detect();

    if platform == ServicePlatform::Unsupported {
        anyhow::bail!(
            "System service installation is not supported on this platform. \
             Use 'rustyclaw gateway start' for daemon mode instead."
        );
    }

    // Find rustyclaw binary
    let binary_path = std::env::current_exe()
        .context("Failed to determine rustyclaw binary path")?;

    // Ensure log directory exists
    let log_dir = settings_dir.join("logs");
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("Failed to create log directory: {}", log_dir.display()))?;

    match platform {
        ServicePlatform::Systemd => install_systemd(&binary_path, settings_dir, port, bind, enable, start),
        ServicePlatform::Launchd => install_launchd(&binary_path, settings_dir, port, bind, enable, start),
        ServicePlatform::Unsupported => unreachable!(),
    }
}

/// Install systemd service (Linux)
fn install_systemd(
    binary_path: &Path,
    settings_dir: &Path,
    port: u16,
    bind: &str,
    enable: bool,
    start: bool,
) -> Result<InstallResult> {
    let unit_content = generate_systemd_unit(binary_path, settings_dir, port, bind);

    // Determine service file location (user service)
    let systemd_dir = dirs::home_dir()
        .context("Failed to determine home directory")?
        .join(".config")
        .join("systemd")
        .join("user");

    fs::create_dir_all(&systemd_dir)
        .with_context(|| format!("Failed to create systemd user directory: {}", systemd_dir.display()))?;

    let service_path = systemd_dir.join("rustyclaw-gateway.service");

    fs::write(&service_path, unit_content)
        .with_context(|| format!("Failed to write systemd unit file: {}", service_path.display()))?;

    // Reload systemd daemon
    Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .context("Failed to reload systemd daemon")?;

    let mut actions = Vec::new();

    // Enable service if requested
    if enable {
        let output = Command::new("systemctl")
            .args(["--user", "enable", "rustyclaw-gateway.service"])
            .output()
            .context("Failed to enable service")?;

        if output.status.success() {
            actions.push("enabled");
        } else {
            eprintln!("Warning: Failed to enable service: {}",
                String::from_utf8_lossy(&output.stderr));
        }
    }

    // Start service if requested
    if start {
        let output = Command::new("systemctl")
            .args(["--user", "start", "rustyclaw-gateway.service"])
            .output()
            .context("Failed to start service")?;

        if output.status.success() {
            actions.push("started");
        } else {
            eprintln!("Warning: Failed to start service: {}",
                String::from_utf8_lossy(&output.stderr));
        }
    }

    let message = if actions.is_empty() {
        format!(
            "Service installed at: {}\n\
             \n\
             To enable: systemctl --user enable rustyclaw-gateway.service\n\
             To start:  systemctl --user start rustyclaw-gateway.service\n\
             To check:  systemctl --user status rustyclaw-gateway.service",
            service_path.display()
        )
    } else {
        format!(
            "Service installed and {} at: {}\n\
             \n\
             To check status: systemctl --user status rustyclaw-gateway.service\n\
             To view logs:    journalctl --user -u rustyclaw-gateway.service -f",
            actions.join(" and "),
            service_path.display()
        )
    };

    Ok(InstallResult {
        platform: ServicePlatform::Systemd,
        service_path,
        message,
    })
}

/// Install launchd service (macOS)
fn install_launchd(
    binary_path: &Path,
    settings_dir: &Path,
    port: u16,
    bind: &str,
    enable: bool,
    start: bool,
) -> Result<InstallResult> {
    let plist_content = generate_launchd_plist(binary_path, settings_dir, port, bind);

    // Determine plist location (user agent)
    let launchd_dir = dirs::home_dir()
        .context("Failed to determine home directory")?
        .join("Library")
        .join("LaunchAgents");

    fs::create_dir_all(&launchd_dir)
        .with_context(|| format!("Failed to create LaunchAgents directory: {}", launchd_dir.display()))?;

    let service_path = launchd_dir.join("com.rustyclaw.gateway.plist");

    fs::write(&service_path, plist_content)
        .with_context(|| format!("Failed to write launchd plist: {}", service_path.display()))?;

    let mut actions = Vec::new();

    // Load service if enable or start is requested
    if enable || start {
        let output = Command::new("launchctl")
            .args(["load", service_path.to_str().unwrap()])
            .output()
            .context("Failed to load service")?;

        if output.status.success() {
            actions.push("loaded");
        } else {
            eprintln!("Warning: Failed to load service: {}",
                String::from_utf8_lossy(&output.stderr));
        }
    }

    let message = if actions.is_empty() {
        format!(
            "Service installed at: {}\n\
             \n\
             To load:   launchctl load {}\n\
             To check:  launchctl list | grep rustyclaw",
            service_path.display(),
            service_path.display()
        )
    } else {
        format!(
            "Service installed and {} at: {}\n\
             \n\
             To check status: launchctl list | grep rustyclaw\n\
             To view logs:    tail -f {}",
            actions.join(" and "),
            service_path.display(),
            settings_dir.join("logs").join("gateway.log").display()
        )
    };

    Ok(InstallResult {
        platform: ServicePlatform::Launchd,
        service_path,
        message,
    })
}

/// Uninstall gateway system service
pub fn uninstall_service() -> Result<String> {
    let platform = ServicePlatform::detect();

    if platform == ServicePlatform::Unsupported {
        anyhow::bail!("System service management is not supported on this platform");
    }

    match platform {
        ServicePlatform::Systemd => uninstall_systemd(),
        ServicePlatform::Launchd => uninstall_launchd(),
        ServicePlatform::Unsupported => unreachable!(),
    }
}

/// Uninstall systemd service
fn uninstall_systemd() -> Result<String> {
    let systemd_dir = dirs::home_dir()
        .context("Failed to determine home directory")?
        .join(".config")
        .join("systemd")
        .join("user");

    let service_path = systemd_dir.join("rustyclaw-gateway.service");

    if !service_path.exists() {
        return Ok("Service is not installed".to_string());
    }

    // Stop service
    let _ = Command::new("systemctl")
        .args(["--user", "stop", "rustyclaw-gateway.service"])
        .output();

    // Disable service
    let _ = Command::new("systemctl")
        .args(["--user", "disable", "rustyclaw-gateway.service"])
        .output();

    // Remove service file
    fs::remove_file(&service_path)
        .with_context(|| format!("Failed to remove service file: {}", service_path.display()))?;

    // Reload systemd daemon
    Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .context("Failed to reload systemd daemon")?;

    Ok(format!("Service uninstalled from: {}", service_path.display()))
}

/// Uninstall launchd service
fn uninstall_launchd() -> Result<String> {
    let launchd_dir = dirs::home_dir()
        .context("Failed to determine home directory")?
        .join("Library")
        .join("LaunchAgents");

    let service_path = launchd_dir.join("com.rustyclaw.gateway.plist");

    if !service_path.exists() {
        return Ok("Service is not installed".to_string());
    }

    // Unload service
    let _ = Command::new("launchctl")
        .args(["unload", service_path.to_str().unwrap()])
        .output();

    // Remove plist file
    fs::remove_file(&service_path)
        .with_context(|| format!("Failed to remove plist file: {}", service_path.display()))?;

    Ok(format!("Service uninstalled from: {}", service_path.display()))
}

/// Read the last N lines from the gateway log
pub fn read_log_lines(settings_dir: &Path, lines: usize) -> Result<Vec<String>> {
    let log_path = settings_dir.join("logs").join("gateway.log");

    if !log_path.exists() {
        return Ok(vec!["No log file found".to_string()]);
    }

    let content = fs::read_to_string(&log_path)
        .with_context(|| format!("Failed to read log file: {}", log_path.display()))?;

    let all_lines: Vec<String> = content.lines().map(String::from).collect();
    let start = all_lines.len().saturating_sub(lines);

    Ok(all_lines[start..].to_vec())
}

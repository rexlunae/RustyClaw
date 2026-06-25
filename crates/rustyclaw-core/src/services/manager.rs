//! Service lifecycle manager.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::Instant;

use tokio::process::{Child, Command};
use tracing::{info, warn};

use super::types::*;

/// A running service instance tracked by the manager.
struct RunningService {
    def: ServiceDef,
    child: Child,
    status: ServiceStatus,
    started_at: Instant,
    restart_count: u32,
    exit_code: Option<i32>,
    health_ok: Option<bool>,
    health_failures: u32,
    log_lines: VecDeque<String>,
    mcp_tool_count: u32,
}

impl RunningService {
    fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    fn append_log(&mut self, line: &str) {
        if self.log_lines.len() >= self.def.max_log_lines {
            self.log_lines.pop_front();
        }
        self.log_lines.push_back(line.to_string());
    }

    fn to_info(&self, name: &str) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            service_type: self.def.service_type,
            status: self.status,
            pid: self.child.id(),
            uptime_secs: if self.status.is_active() {
                Some(self.uptime_secs())
            } else {
                None
            },
            restart_count: self.restart_count,
            exit_code: self.exit_code,
            health_ok: self.health_ok,
            mcp_tools: self.mcp_tool_count,
        }
    }
}

/// Manages the lifecycle of backend services.
pub struct ServiceManager {
    config: ServicesConfig,
    running: HashMap<String, RunningService>,
    /// Services that exited and are not running (retained for status queries).
    stopped: HashMap<String, StoppedEntry>,
}

/// Entry for a service that was stopped or failed.
struct StoppedEntry {
    def: ServiceDef,
    status: ServiceStatus,
    exit_code: Option<i32>,
    restart_count: u32,
    log_lines: VecDeque<String>,
}

impl StoppedEntry {
    fn to_info(&self, name: &str) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            service_type: self.def.service_type,
            status: self.status,
            pid: None,
            uptime_secs: None,
            restart_count: self.restart_count,
            exit_code: self.exit_code,
            health_ok: None,
            mcp_tools: 0,
        }
    }
}

impl ServiceManager {
    /// Create a new service manager with the given configuration.
    pub fn new(config: ServicesConfig) -> Self {
        Self {
            config,
            running: HashMap::new(),
            stopped: HashMap::new(),
        }
    }

    /// Start all services marked with `auto_start = true`.
    pub async fn auto_start_all(&mut self) {
        let names: Vec<String> = self
            .config
            .auto_start_services()
            .map(|(n, _)| n.clone())
            .collect();

        for name in names {
            if let Err(e) = self.start(&name).await {
                warn!(service = %name, error = %e, "Failed to auto-start service");
            }
        }
    }

    /// Start a service by name.
    pub async fn start(&mut self, name: &str) -> Result<ServiceInfo, String> {
        if self.running.contains_key(name) {
            return Err(format!("Service '{}' is already running", name));
        }

        let def = self
            .config
            .services
            .get(name)
            .cloned()
            .ok_or_else(|| format!("Unknown service: '{}'", name))?;

        let child = self.spawn_process(&def).await?;

        // Clear any stopped entry
        self.stopped.remove(name);

        let svc = RunningService {
            def,
            child,
            status: ServiceStatus::Starting,
            started_at: Instant::now(),
            restart_count: 0,
            exit_code: None,
            health_ok: None,
            health_failures: 0,
            log_lines: VecDeque::new(),
            mcp_tool_count: 0,
        };

        let info = svc.to_info(name);
        self.running.insert(name.to_string(), svc);
        info!(service = %name, "Service started");

        Ok(info)
    }

    /// Stop a running service.
    pub async fn stop(&mut self, name: &str) -> Result<ServiceInfo, String> {
        let mut svc = self
            .running
            .remove(name)
            .ok_or_else(|| format!("Service '{}' is not running", name))?;

        svc.status = ServiceStatus::Stopping;
        let _ = svc.child.kill().await;

        let info = ServiceInfo {
            name: name.to_string(),
            service_type: svc.def.service_type,
            status: ServiceStatus::Stopped,
            pid: None,
            uptime_secs: None,
            restart_count: svc.restart_count,
            exit_code: None,
            health_ok: None,
            mcp_tools: 0,
        };

        self.stopped.insert(
            name.to_string(),
            StoppedEntry {
                def: svc.def,
                status: ServiceStatus::Stopped,
                exit_code: None,
                restart_count: svc.restart_count,
                log_lines: svc.log_lines,
            },
        );

        info!(service = %name, "Service stopped");
        Ok(info)
    }

    /// Restart a service (stop then start).
    pub async fn restart(&mut self, name: &str) -> Result<ServiceInfo, String> {
        let prev_restarts = if let Some(svc) = self.running.get(name) {
            svc.restart_count
        } else {
            0
        };

        // Stop if running
        if self.running.contains_key(name) {
            self.stop(name).await?;
        }

        // Start fresh
        let mut info = self.start(name).await?;

        // Preserve restart count
        if let Some(svc) = self.running.get_mut(name) {
            svc.restart_count = prev_restarts + 1;
            info.restart_count = svc.restart_count;
        }

        info!(service = %name, restarts = prev_restarts + 1, "Service restarted");
        Ok(info)
    }

    /// List all known services and their statuses.
    pub fn list(&self) -> Vec<ServiceInfo> {
        let mut out: Vec<ServiceInfo> = self
            .running
            .iter()
            .map(|(name, svc)| svc.to_info(name))
            .collect();

        // Include stopped/failed services not currently running
        for (name, entry) in &self.stopped {
            if !self.running.contains_key(name) {
                out.push(entry.to_info(name));
            }
        }

        // Include configured but never-started services
        for name in self.config.services.keys() {
            if !self.running.contains_key(name) && !self.stopped.contains_key(name) {
                out.push(ServiceInfo {
                    name: name.clone(),
                    service_type: self.config.services[name].service_type,
                    status: ServiceStatus::Stopped,
                    pid: None,
                    uptime_secs: None,
                    restart_count: 0,
                    exit_code: None,
                    health_ok: None,
                    mcp_tools: 0,
                });
            }
        }

        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Get info for a specific service.
    pub fn get(&self, name: &str) -> Option<ServiceInfo> {
        if let Some(svc) = self.running.get(name) {
            return Some(svc.to_info(name));
        }
        if let Some(entry) = self.stopped.get(name) {
            return Some(entry.to_info(name));
        }
        if self.config.services.contains_key(name) {
            return Some(ServiceInfo {
                name: name.to_string(),
                service_type: self.config.services[name].service_type,
                status: ServiceStatus::Stopped,
                pid: None,
                uptime_secs: None,
                restart_count: 0,
                exit_code: None,
                health_ok: None,
                mcp_tools: 0,
            });
        }
        None
    }

    /// Get recent log lines for a service.
    pub fn logs(&self, name: &str, tail: Option<usize>) -> Result<Vec<String>, String> {
        let lines = if let Some(svc) = self.running.get(name) {
            &svc.log_lines
        } else if let Some(entry) = self.stopped.get(name) {
            &entry.log_lines
        } else {
            return Err(format!("Unknown service: '{}'", name));
        };

        let tail = tail.unwrap_or(50);
        let start = lines.len().saturating_sub(tail);
        Ok(lines.iter().skip(start).cloned().collect())
    }

    /// Poll running services for status changes.
    ///
    /// Reads output, checks if processes have exited, runs health checks,
    /// and handles restart policies. Returns names of services whose status
    /// changed.
    pub async fn poll(&mut self) -> Vec<String> {
        let mut changed = Vec::new();
        let mut to_restart = Vec::new();
        let mut to_remove = Vec::new();

        for (name, svc) in &mut self.running {
            // Try to read stdout/stderr
            Self::read_child_output(svc);

            // Check if process exited
            match svc.child.try_wait() {
                Ok(Some(exit_status)) => {
                    let code = exit_status.code();
                    svc.exit_code = code;
                    let failed = code.map(|c| c != 0).unwrap_or(true);

                    if failed {
                        svc.status = ServiceStatus::Failed;
                        svc.append_log(&format!("[gateway] Process exited with code {:?}", code));
                    } else {
                        svc.status = ServiceStatus::Stopped;
                        svc.append_log("[gateway] Process exited cleanly");
                    }

                    changed.push(name.clone());

                    // Check restart policy
                    let should_restart = match svc.def.restart {
                        RestartPolicy::Always => true,
                        RestartPolicy::OnFailure => failed,
                        RestartPolicy::Never => false,
                    };

                    if should_restart {
                        to_restart.push(name.clone());
                    } else {
                        to_remove.push((
                            name.clone(),
                            StoppedEntry {
                                def: svc.def.clone(),
                                status: svc.status,
                                exit_code: svc.exit_code,
                                restart_count: svc.restart_count,
                                log_lines: svc.log_lines.clone(),
                            },
                        ));
                    }
                }
                Ok(None) => {
                    // Still running — promote from Starting to Running after
                    // a brief grace period (if no health check configured).
                    if svc.status == ServiceStatus::Starting
                        && svc.def.health_check.is_none()
                        && svc.uptime_secs() >= 2
                    {
                        svc.status = ServiceStatus::Running;
                        changed.push(name.clone());
                    }
                }
                Err(e) => {
                    warn!(service = %name, error = %e, "Error checking service status");
                }
            }
        }

        // Move exited services to stopped
        for (name, entry) in to_remove {
            self.running.remove(&name);
            self.stopped.insert(name, entry);
        }

        // Handle restarts
        for name in to_restart {
            if let Some(mut svc) = self.running.remove(&name) {
                svc.restart_count += 1;
                let count = svc.restart_count;
                let def = svc.def.clone();
                let log_lines = svc.log_lines.clone();

                match self.spawn_process(&def).await {
                    Ok(child) => {
                        svc.child = child;
                        svc.status = ServiceStatus::Starting;
                        svc.started_at = Instant::now();
                        svc.exit_code = None;
                        svc.health_ok = None;
                        svc.health_failures = 0;
                        svc.log_lines = log_lines;
                        svc.append_log(&format!("[gateway] Restarting (attempt #{})", count));
                        self.running.insert(name.clone(), svc);
                        info!(service = %name, attempt = count, "Service restarted by policy");
                    }
                    Err(e) => {
                        warn!(service = %name, error = %e, "Failed to restart service");
                        self.stopped.insert(
                            name,
                            StoppedEntry {
                                def,
                                status: ServiceStatus::Failed,
                                exit_code: None,
                                restart_count: count,
                                log_lines,
                            },
                        );
                    }
                }
            }
        }

        changed
    }

    /// Run health checks on services that have them configured.
    pub async fn run_health_checks(&mut self) -> Vec<String> {
        let mut changed = Vec::new();

        let names: Vec<String> = self
            .running
            .iter()
            .filter(|(_, svc)| {
                svc.def.health_check.is_some()
                    && matches!(
                        svc.status,
                        ServiceStatus::Starting | ServiceStatus::Running | ServiceStatus::Unhealthy
                    )
            })
            .map(|(n, _)| n.clone())
            .collect();

        for name in names {
            let healthy = {
                let svc = self.running.get(&name).unwrap();
                let hc = svc.def.health_check.as_ref().unwrap();
                Self::check_health(hc).await
            };

            if let Some(svc) = self.running.get_mut(&name) {
                let hc = svc.def.health_check.as_ref().unwrap();
                let prev_status = svc.status;

                if healthy {
                    svc.health_ok = Some(true);
                    svc.health_failures = 0;
                    if svc.status != ServiceStatus::Running {
                        svc.status = ServiceStatus::Running;
                    }
                } else {
                    svc.health_failures += 1;
                    if svc.health_failures >= hc.retries {
                        svc.health_ok = Some(false);
                        svc.status = ServiceStatus::Unhealthy;
                    }
                }

                if svc.status != prev_status {
                    changed.push(name);
                }
            }
        }

        changed
    }

    /// Set the MCP tool count for a service (called after MCP discovery).
    pub fn set_mcp_tool_count(&mut self, name: &str, count: u32) {
        if let Some(svc) = self.running.get_mut(name) {
            svc.mcp_tool_count = count;
        }
    }

    /// Stop all running services (for graceful shutdown).
    pub async fn stop_all(&mut self) {
        let names: Vec<String> = self.running.keys().cloned().collect();
        for name in names {
            if let Err(e) = self.stop(&name).await {
                warn!(service = %name, error = %e, "Error stopping service during shutdown");
            }
        }
    }

    /// Dynamically register a new service definition (not from config file).
    pub fn register(&mut self, name: String, def: ServiceDef) -> Result<(), String> {
        if self.config.services.contains_key(&name) {
            return Err(format!("Service '{}' already exists", name));
        }
        self.config.services.insert(name, def);
        Ok(())
    }

    // ── Internal helpers ────────────────────────────────────────────

    async fn spawn_process(&self, def: &ServiceDef) -> Result<Child, String> {
        let full_cmd = format!("{} {}", &def.command, &def.args.join(" "));
        if let Err(e) = crate::tools::helpers::validate_command_safe(&full_cmd) {
            return Err(format!(
                "Service command rejected by security validation: {}",
                e
            ));
        }

        let mut cmd = Command::new(&def.command);
        cmd.args(&def.args);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        for (key, value) in &def.env {
            cmd.env(key, value);
        }
        if let Some(ref cwd) = def.cwd {
            cmd.current_dir(cwd);
        }

        cmd.spawn()
            .map_err(|e| format!("Failed to spawn service: {}", e))
    }

    fn read_child_output(svc: &mut RunningService) {
        // Collect output lines first, then append them to the log.
        // This avoids holding a mutable borrow of svc.child while
        // calling svc.append_log().
        let mut lines: Vec<String> = Vec::new();

        // Read stdout
        if let Some(ref mut stdout) = svc.child.stdout {
            let mut buf = [0u8; 4096];
            loop {
                match try_read_async(stdout, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        for line in text.lines() {
                            lines.push(line.to_string());
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        // Read stderr
        if let Some(ref mut stderr) = svc.child.stderr {
            let mut buf = [0u8; 4096];
            loop {
                match try_read_async(stderr, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        for line in text.lines() {
                            lines.push(format!("[stderr] {}", line));
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        for line in &lines {
            svc.append_log(line);
        }
    }

    async fn check_health(hc: &HealthCheck) -> bool {
        let timeout = std::time::Duration::from_secs(hc.timeout_secs as u64);

        match &hc.method {
            HealthMethod::HttpGet { url } => {
                match tokio::time::timeout(timeout, async { reqwest::get(url).await }).await {
                    Ok(Ok(resp)) => resp.status().is_success(),
                    _ => false,
                }
            }
            HealthMethod::TcpConnect { address } => {
                matches!(
                    tokio::time::timeout(timeout, tokio::net::TcpStream::connect(address)).await,
                    Ok(Ok(_))
                )
            }
            HealthMethod::Command { command } => {
                match tokio::time::timeout(timeout, async {
                    Command::new("sh").arg("-c").arg(command).output().await
                })
                .await
                {
                    Ok(Ok(output)) => output.status.success(),
                    _ => false,
                }
            }
        }
    }
}

/// Non-blocking read from a tokio ChildStdout/ChildStderr.
///
/// Uses `try_read` on Unix (the underlying fd). Falls back to returning 0
/// on other platforms.
fn try_read_async<T: tokio::io::AsyncRead + std::os::unix::io::AsRawFd>(
    reader: &mut T,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    let fd = reader.as_raw_fd();

    let mut poll_fd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    let ready = unsafe { libc::poll(&mut poll_fd, 1, 0) };

    if ready > 0 && (poll_fd.revents & libc::POLLIN) != 0 {
        use std::io::Read;
        // Safety: we only read when poll says data is ready
        unsafe {
            let mut file = std::fs::File::from_raw_fd(fd);
            let n = file.read(buf);
            // Prevent drop from closing the fd — it's owned by the Child
            std::mem::forget(file);
            n
        }
    } else {
        Ok(0)
    }
}

// Bring trait into scope for from_raw_fd / as_raw_fd
use std::os::unix::io::FromRawFd;

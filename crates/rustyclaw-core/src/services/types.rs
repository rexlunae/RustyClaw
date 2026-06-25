//! Service definition and status types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for all managed services.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServicesConfig {
    /// Named service definitions.
    #[serde(default)]
    pub services: HashMap<String, ServiceDef>,
}

impl ServicesConfig {
    /// Get services marked for auto-start.
    pub fn auto_start_services(&self) -> impl Iterator<Item = (&String, &ServiceDef)> {
        self.services.iter().filter(|(_, def)| def.auto_start)
    }

    /// Check if any services are configured.
    pub fn has_services(&self) -> bool {
        !self.services.is_empty()
    }
}

/// Definition of a managed backend service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDef {
    /// Command to run (e.g., "npx", "/usr/local/bin/my-api").
    pub command: String,

    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Working directory for the process.
    #[serde(default)]
    pub cwd: Option<PathBuf>,

    /// Type of service (determines how the gateway interacts with it).
    #[serde(default)]
    pub service_type: ServiceType,

    /// Restart policy when the process exits.
    #[serde(default)]
    pub restart: RestartPolicy,

    /// Whether to start automatically when the gateway starts.
    #[serde(default)]
    pub auto_start: bool,

    /// Health check configuration.
    #[serde(default)]
    pub health_check: Option<HealthCheck>,

    /// Maximum number of log lines to retain in memory.
    #[serde(default = "default_max_log_lines")]
    pub max_log_lines: usize,
}

fn default_max_log_lines() -> usize {
    1000
}

impl Default for ServiceDef {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            service_type: ServiceType::default(),
            restart: RestartPolicy::default(),
            auto_start: false,
            health_check: None,
            max_log_lines: default_max_log_lines(),
        }
    }
}

/// Type of managed service.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType {
    /// Native MCP stdio server — tools are auto-discovered via MCP protocol.
    Mcp,
    /// HTTP/REST service with optional health endpoint.
    Http,
    /// Arbitrary process with log capture.
    #[default]
    Process,
}

impl ServiceType {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Mcp => "MCP",
            Self::Http => "HTTP",
            Self::Process => "Process",
        }
    }
}

/// Restart policy for a managed service.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    /// Never restart (default).
    #[default]
    Never,
    /// Restart only on non-zero exit.
    OnFailure,
    /// Always restart (even on clean exit).
    Always,
}

/// Health check configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    /// How to probe health.
    pub method: HealthMethod,

    /// Interval between checks in seconds.
    #[serde(default = "default_health_interval")]
    pub interval_secs: u32,

    /// Timeout for each check in seconds.
    #[serde(default = "default_health_timeout")]
    pub timeout_secs: u32,

    /// Number of consecutive failures before marking unhealthy.
    #[serde(default = "default_health_retries")]
    pub retries: u32,
}

fn default_health_interval() -> u32 {
    30
}
fn default_health_timeout() -> u32 {
    5
}
fn default_health_retries() -> u32 {
    3
}

/// Health check probe method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HealthMethod {
    /// HTTP GET to a URL; healthy if 2xx status.
    HttpGet { url: String },
    /// TCP connection to host:port; healthy if connection succeeds.
    TcpConnect { address: String },
    /// Run a command; healthy if exit code is 0.
    Command { command: String },
}

/// Current status of a running service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    /// Not yet started.
    Stopped,
    /// Starting up (process spawned, waiting for health).
    Starting,
    /// Running and healthy.
    Running,
    /// Running but health check is failing.
    Unhealthy,
    /// Shutting down.
    Stopping,
    /// Exited with a failure.
    Failed,
}

impl ServiceStatus {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Stopped => "Stopped",
            Self::Starting => "Starting",
            Self::Running => "Running",
            Self::Unhealthy => "Unhealthy",
            Self::Stopping => "Stopping",
            Self::Failed => "Failed",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Unhealthy | Self::Stopping
        )
    }
}

/// Snapshot of a service's current state (for display / protocol).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub service_type: ServiceType,
    pub status: ServiceStatus,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub restart_count: u32,
    pub exit_code: Option<i32>,
    pub health_ok: Option<bool>,
    pub mcp_tools: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_services_config() {
        let toml = r#"
            [services.my-api]
            command = "/usr/local/bin/my-api"
            args = ["--port", "8080"]
            service_type = "http"
            restart = "on-failure"
            auto_start = true

            [services.my-mcp]
            command = "npx"
            args = ["-y", "@my/mcp-server"]
            service_type = "mcp"
            auto_start = true
        "#;

        let config: ServicesConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.services.len(), 2);

        let api = &config.services["my-api"];
        assert_eq!(api.service_type, ServiceType::Http);
        assert_eq!(api.restart, RestartPolicy::OnFailure);
        assert!(api.auto_start);

        let mcp = &config.services["my-mcp"];
        assert_eq!(mcp.service_type, ServiceType::Mcp);
    }

    #[test]
    fn test_service_status_is_active() {
        assert!(!ServiceStatus::Stopped.is_active());
        assert!(ServiceStatus::Starting.is_active());
        assert!(ServiceStatus::Running.is_active());
        assert!(ServiceStatus::Unhealthy.is_active());
        assert!(ServiceStatus::Stopping.is_active());
        assert!(!ServiceStatus::Failed.is_active());
    }

    #[test]
    fn test_health_check_deserialize() {
        let toml = r#"
            [services.test]
            command = "test"
            service_type = "http"
            [services.test.health_check]
            method = { type = "http_get", url = "http://localhost:8080/health" }
            interval_secs = 15
            timeout_secs = 3
            retries = 5
        "#;

        let config: ServicesConfig = toml::from_str(toml).unwrap();
        let svc = &config.services["test"];
        let hc = svc.health_check.as_ref().unwrap();
        assert_eq!(hc.interval_secs, 15);
        assert_eq!(hc.retries, 5);
        assert!(matches!(hc.method, HealthMethod::HttpGet { .. }));
    }
}

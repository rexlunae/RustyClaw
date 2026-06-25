//! Component data for the managed-services panel.

/// Display data for a single managed service.
#[derive(Clone, Debug, PartialEq)]
pub struct ServiceInfoData {
    pub name: String,
    pub service_type: String,
    pub status: String,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub restart_count: u32,
    pub exit_code: Option<i32>,
    pub health_ok: Option<bool>,
    pub mcp_tools: u32,
}

impl ServiceInfoData {
    /// Human-friendly uptime string (e.g. "2h 15m", "34s").
    pub fn uptime_display(&self) -> String {
        match self.uptime_secs {
            Some(s) if s >= 3600 => format!("{}h {}m", s / 3600, (s % 3600) / 60),
            Some(s) if s >= 60 => format!("{}m {}s", s / 60, s % 60),
            Some(s) => format!("{}s", s),
            None => "—".into(),
        }
    }

    /// CSS-like class for the status badge.
    pub fn status_class(&self) -> &'static str {
        match self.status.as_str() {
            "Running" => "is-success",
            "Starting" => "is-info",
            "Unhealthy" => "is-warning",
            "Stopping" => "is-info",
            "Failed" => "is-danger",
            _ => "is-light",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.status.as_str(),
            "Running" | "Starting" | "Unhealthy" | "Stopping"
        )
    }
}

/// Data for the services list panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ServiceListData {
    pub services: Vec<ServiceInfoData>,
}

impl ServiceListData {
    pub fn running_count(&self) -> usize {
        self.services
            .iter()
            .filter(|s| s.status == "Running")
            .count()
    }

    pub fn total_count(&self) -> usize {
        self.services.len()
    }
}

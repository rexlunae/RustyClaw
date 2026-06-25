//! Component data for host hardware and system load panels.

/// GPU info for display.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuDisplayInfo {
    pub name: String,
    pub vendor: String,
    pub vram_gib: f64,
}

/// Everything the host-info panel needs to render.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HostInfoData {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cpu_brand: String,
    pub cpu_cores_physical: usize,
    pub cpu_cores_logical: usize,
    pub cpu_frequency_mhz: u64,
    pub total_memory_gib: f64,
    pub total_swap_gib: f64,
    pub disk_total_gib: f64,
    pub disk_available_gib: f64,
    pub gpus: Vec<GpuDisplayInfo>,
    pub summary: String,
}

impl HostInfoData {
    /// Disk usage as a percentage string (e.g. "42%").
    pub fn disk_used_percent(&self) -> String {
        if self.disk_total_gib > 0.0 {
            let used = self.disk_total_gib - self.disk_available_gib;
            format!("{:.0}%", (used / self.disk_total_gib) * 100.0)
        } else {
            "N/A".into()
        }
    }

    pub fn has_gpu(&self) -> bool {
        !self.gpus.is_empty()
    }
}

/// Everything the load-status panel needs to render.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LoadStatusData {
    pub load_score: f64,
    pub avg_load_score: f64,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub summary: String,
}

impl LoadStatusData {
    /// Semantic label for the composite load score.
    pub fn load_label(&self) -> &'static str {
        if self.load_score < 0.25 {
            "Low"
        } else if self.load_score < 0.60 {
            "Moderate"
        } else if self.load_score < 0.85 {
            "High"
        } else {
            "Critical"
        }
    }

    /// CSS-like class for the load level.
    pub fn load_class(&self) -> &'static str {
        if self.load_score < 0.25 {
            "is-success"
        } else if self.load_score < 0.60 {
            "is-info"
        } else if self.load_score < 0.85 {
            "is-warn"
        } else {
            "is-danger"
        }
    }
}

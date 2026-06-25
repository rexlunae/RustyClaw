//! Host hardware introspection.
//!
//! Detects the capabilities of the machine the gateway is running on:
//! CPU, memory, swap, disk, GPU (best-effort), and OS metadata.
//! This information feeds into load balancing decisions and determines
//! which locally-managed models the host can run.

use serde::{Deserialize, Serialize};
use std::path::Path;
use sysinfo::System;
use tracing::debug;

/// Detected GPU device (best-effort).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    /// Device name (e.g. "NVIDIA GeForce RTX 4090").
    pub name: String,
    /// Vendor string.
    pub vendor: String,
    /// VRAM in bytes, if detectable.
    pub vram_bytes: Option<u64>,
}

/// Static hardware profile of the host.
///
/// Captured once at gateway startup; does not change during the process
/// lifetime (hot-plug events are not tracked).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCapabilities {
    pub hostname: String,
    pub os_name: String,
    pub os_version: String,
    pub arch: String,

    // ── CPU ──────────────────────────────────────────────────────────
    pub cpu_brand: String,
    pub cpu_cores_physical: usize,
    pub cpu_cores_logical: usize,
    /// Max frequency reported by any core (MHz).
    pub cpu_frequency_mhz: u64,

    // ── Memory ──────────────────────────────────────────────────────
    pub total_memory_bytes: u64,
    pub total_swap_bytes: u64,

    // ── GPU ─────────────────────────────────────────────────────────
    pub gpus: Vec<GpuInfo>,

    // ── Disk ────────────────────────────────────────────────────────
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
}

impl HostCapabilities {
    /// Probe the current host and return a snapshot of its capabilities.
    pub fn detect() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();

        let hostname = System::host_name().unwrap_or_else(|| "unknown".into());
        let os_name = System::name().unwrap_or_else(|| "unknown".into());
        let os_version = System::os_version().unwrap_or_else(|| "unknown".into());
        let arch = std::env::consts::ARCH.to_string();

        // CPU
        let cpus = sys.cpus();
        let cpu_brand = cpus
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "unknown".into());
        let cpu_cores_physical = System::physical_core_count().unwrap_or(0);
        let cpu_cores_logical = cpus.len();
        let cpu_frequency_mhz = cpus.iter().map(|c| c.frequency()).max().unwrap_or(0);

        // Memory
        let total_memory_bytes = sys.total_memory();
        let total_swap_bytes = sys.total_swap();

        // Disk — aggregate all mount points
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let (disk_total_bytes, disk_available_bytes) =
            disks
                .iter()
                .fold((0u64, 0u64), |(total, avail), d| {
                    (
                        total.saturating_add(d.total_space()),
                        avail.saturating_add(d.available_space()),
                    )
                });

        // GPU — best-effort scan
        let gpus = detect_gpus();

        let caps = Self {
            hostname,
            os_name,
            os_version,
            arch,
            cpu_brand,
            cpu_cores_physical,
            cpu_cores_logical,
            cpu_frequency_mhz,
            total_memory_bytes,
            total_swap_bytes,
            gpus,
            disk_total_bytes,
            disk_available_bytes,
        };

        debug!(
            hostname = %caps.hostname,
            cpus = caps.cpu_cores_logical,
            ram_gb = caps.total_memory_bytes / (1024 * 1024 * 1024),
            gpus = caps.gpus.len(),
            "Host capabilities detected"
        );

        caps
    }

    /// Total GPU VRAM across all detected devices.
    pub fn total_vram_bytes(&self) -> u64 {
        self.gpus
            .iter()
            .filter_map(|g| g.vram_bytes)
            .sum()
    }

    /// Whether any GPU is available.
    pub fn has_gpu(&self) -> bool {
        !self.gpus.is_empty()
    }

    /// Human-readable summary (suitable for tool output / status display).
    pub fn summary(&self) -> String {
        let ram_gb = self.total_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        let disk_gb = self.disk_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        let disk_avail_gb = self.disk_available_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

        let mut s = format!(
            "Host: {} ({} {} {})\n\
             CPU:  {} ({} physical / {} logical cores, {} MHz)\n\
             RAM:  {:.1} GB\n\
             Swap: {:.1} GB\n\
             Disk: {:.1} GB total, {:.1} GB available",
            self.hostname,
            self.os_name,
            self.os_version,
            self.arch,
            self.cpu_brand,
            self.cpu_cores_physical,
            self.cpu_cores_logical,
            self.cpu_frequency_mhz,
            ram_gb,
            self.total_swap_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
            disk_gb,
            disk_avail_gb,
        );

        if self.gpus.is_empty() {
            s.push_str("\nGPU:  none detected");
        } else {
            for (i, gpu) in self.gpus.iter().enumerate() {
                let vram = gpu
                    .vram_bytes
                    .map(|v| format!("{:.1} GB", v as f64 / (1024.0 * 1024.0 * 1024.0)))
                    .unwrap_or_else(|| "unknown".into());
                s.push_str(&format!(
                    "\nGPU {}: {} ({}, VRAM: {})",
                    i, gpu.name, gpu.vendor, vram
                ));
            }
        }

        s
    }
}

// ── GPU detection helpers ──────────────────────────────────────────────────

/// Best-effort GPU detection. Tries Linux DRM sysfs, then falls back to
/// checking for NVIDIA tools.
fn detect_gpus() -> Vec<GpuInfo> {
    #[allow(unused_mut, unused_assignments)]
    let mut gpus = Vec::new();

    // Linux: scan /sys/class/drm/card*/device/
    #[cfg(target_os = "linux")]
    {
        gpus = detect_gpus_linux_drm();
    }

    if gpus.is_empty() {
        // Fallback: try nvidia-smi
        if let Some(nvidia_gpus) = detect_gpus_nvidia_smi() {
            gpus = nvidia_gpus;
        }
    }

    gpus
}

/// Scan Linux DRM sysfs for GPU devices.
#[cfg(target_os = "linux")]
fn detect_gpus_linux_drm() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    let drm_path = Path::new("/sys/class/drm");
    if !drm_path.exists() {
        return gpus;
    }

    // Track PCI addresses to deduplicate render nodes vs card nodes.
    let mut seen_pci = std::collections::HashSet::new();

    let Ok(entries) = std::fs::read_dir(drm_path) else {
        return gpus;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Only look at card0, card1, … (skip renderD*)
        if !name_str.starts_with("card") || name_str.contains('-') {
            continue;
        }

        let device_dir = entry.path().join("device");
        if !device_dir.exists() {
            continue;
        }

        // Read PCI vendor/device for dedup
        let vendor_id = read_sysfs_hex(&device_dir.join("vendor"));
        let device_id = read_sysfs_hex(&device_dir.join("device"));
        let pci_key = (vendor_id, device_id);
        if pci_key != (0, 0) && !seen_pci.insert(pci_key) {
            continue;
        }

        let vendor = match vendor_id {
            0x10de => "NVIDIA",
            0x1002 => "AMD",
            0x8086 => "Intel",
            _ => "Unknown",
        };

        // Try to get a human-readable product name from uevent
        let product_name = read_uevent_name(&device_dir).unwrap_or_else(|| {
            format!("{} GPU {:04x}:{:04x}", vendor, vendor_id, device_id)
        });

        // VRAM: AMD exposes mem_info_vram_total, NVIDIA requires nvidia-smi
        let vram_bytes = read_sysfs_u64(&device_dir.join("mem_info_vram_total"));

        gpus.push(GpuInfo {
            name: product_name,
            vendor: vendor.to_string(),
            vram_bytes,
        });
    }

    gpus
}

/// Try `nvidia-smi` to detect NVIDIA GPUs and VRAM.
fn detect_gpus_nvidia_smi() -> Option<Vec<GpuInfo>> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let gpus: Vec<GpuInfo> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(2, ',');
            let name = parts.next()?.trim().to_string();
            let vram_mib: u64 = parts.next()?.trim().parse().ok()?;
            Some(GpuInfo {
                name,
                vendor: "NVIDIA".to_string(),
                vram_bytes: Some(vram_mib * 1024 * 1024),
            })
        })
        .collect();

    if gpus.is_empty() {
        None
    } else {
        Some(gpus)
    }
}

/// Read a hex value from a sysfs file (e.g. "0x10de\n").
#[cfg(target_os = "linux")]
fn read_sysfs_hex(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| {
            let s = s.trim().trim_start_matches("0x");
            u32::from_str_radix(s, 16).ok()
        })
        .unwrap_or(0)
}

/// Read a decimal u64 from a sysfs file.
#[cfg(target_os = "linux")]
fn read_sysfs_u64(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Parse the product name from a uevent file.
#[cfg(target_os = "linux")]
fn read_uevent_name(device_dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(device_dir.join("uevent")).ok()?;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("PCI_SLOT_NAME=") {
            // Use the PCI slot as fallback name
            return Some(format!("GPU at {}", val));
        }
    }
    None
}

// ── Shared type alias ──────────────────────────────────────────────────────

/// Thread-safe shared reference to host capabilities.
pub type SharedHostCapabilities = std::sync::Arc<HostCapabilities>;

/// Create a shared host capabilities snapshot.
pub fn detect_host() -> SharedHostCapabilities {
    std::sync::Arc::new(HostCapabilities::detect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_valid_data() {
        let caps = HostCapabilities::detect();
        assert!(!caps.hostname.is_empty());
        assert!(caps.cpu_cores_logical > 0);
        assert!(caps.total_memory_bytes > 0);
    }

    #[test]
    fn summary_is_nonempty() {
        let caps = HostCapabilities::detect();
        let s = caps.summary();
        assert!(s.contains("Host:"));
        assert!(s.contains("CPU:"));
        assert!(s.contains("RAM:"));
    }

    #[test]
    fn total_vram_without_gpus() {
        let mut caps = HostCapabilities::detect();
        caps.gpus.clear();
        assert_eq!(caps.total_vram_bytes(), 0);
        assert!(!caps.has_gpu());
    }

    #[test]
    fn shared_host_is_arc() {
        let shared = detect_host();
        let clone = shared.clone();
        assert_eq!(shared.hostname, clone.hostname);
    }
}

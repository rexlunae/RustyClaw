//! Real-time resource load tracking.
//!
//! Periodically samples CPU, memory, swap, and inference activity to produce
//! a [`LoadSnapshot`] and a composite load score.  The score can be shared
//! with peer gateways for load-balancing decisions.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::System;
use tokio::sync::RwLock;
use tracing::debug;

/// Default sampling interval.
const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Maximum number of snapshots retained (≈ 5 minutes at 5 s interval).
const MAX_SNAPSHOTS: usize = 60;

/// A single point-in-time resource measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadSnapshot {
    /// Wall-clock seconds since the tracker was started.
    pub uptime_secs: u64,
    /// Per-core average CPU usage (0.0–100.0).
    pub cpu_usage_percent: f32,
    /// Bytes of RAM currently in use.
    pub memory_used_bytes: u64,
    /// Total RAM on the host.
    pub memory_total_bytes: u64,
    /// Bytes of swap currently in use.
    pub swap_used_bytes: u64,
    /// Total swap on the host.
    pub swap_total_bytes: u64,
    /// Number of locally-managed models currently loaded in memory.
    pub active_model_count: u32,
    /// Number of inference requests currently in flight.
    pub active_inference_count: u32,
}

impl LoadSnapshot {
    /// Memory utilization as a fraction (0.0–1.0).
    pub fn memory_utilization(&self) -> f32 {
        if self.memory_total_bytes == 0 {
            return 0.0;
        }
        self.memory_used_bytes as f32 / self.memory_total_bytes as f32
    }

    /// Composite load score in [0.0, 1.0].
    ///
    /// Higher values mean the host is more loaded and less desirable for
    /// new work.  The formula weights CPU and memory equally and adds a
    /// small penalty for each active inference.
    pub fn load_score(&self) -> f32 {
        let cpu = self.cpu_usage_percent / 100.0;
        let mem = self.memory_utilization();
        let inference_penalty = (self.active_inference_count as f32 * 0.05).min(0.3);
        ((cpu * 0.5 + mem * 0.5) + inference_penalty).min(1.0)
    }
}

/// Shared load-tracker state protected by an async RwLock.
pub type SharedLoadTracker = Arc<RwLock<LoadTracker>>;

/// Tracks resource load over a sliding window.
pub struct LoadTracker {
    snapshots: VecDeque<LoadSnapshot>,
    start: Instant,
    /// Externally-reported counts (set by the gateway).
    active_model_count: u32,
    active_inference_count: u32,
}

impl LoadTracker {
    pub fn new() -> Self {
        Self {
            snapshots: VecDeque::with_capacity(MAX_SNAPSHOTS),
            start: Instant::now(),
            active_model_count: 0,
            active_inference_count: 0,
        }
    }

    /// Record a new sample from the OS.
    pub fn sample(&mut self, sys: &System) {
        let cpu_usage_percent = sys.global_cpu_usage();

        let snapshot = LoadSnapshot {
            uptime_secs: self.start.elapsed().as_secs(),
            cpu_usage_percent,
            memory_used_bytes: sys.used_memory(),
            memory_total_bytes: sys.total_memory(),
            swap_used_bytes: sys.used_swap(),
            swap_total_bytes: sys.total_swap(),
            active_model_count: self.active_model_count,
            active_inference_count: self.active_inference_count,
        };

        if self.snapshots.len() >= MAX_SNAPSHOTS {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
    }

    /// The most recent snapshot, if any.
    pub fn latest(&self) -> Option<&LoadSnapshot> {
        self.snapshots.back()
    }

    /// Current composite load score (latest snapshot).
    pub fn load_score(&self) -> f32 {
        self.latest().map(|s| s.load_score()).unwrap_or(0.0)
    }

    /// Average load score over the retained window.
    pub fn avg_load_score(&self) -> f32 {
        if self.snapshots.is_empty() {
            return 0.0;
        }
        let sum: f32 = self.snapshots.iter().map(|s| s.load_score()).sum();
        sum / self.snapshots.len() as f32
    }

    /// All retained snapshots (oldest first).
    pub fn history(&self) -> &VecDeque<LoadSnapshot> {
        &self.snapshots
    }

    /// Update the active model count (called by the gateway when models
    /// are loaded or unloaded).
    pub fn set_active_model_count(&mut self, count: u32) {
        self.active_model_count = count;
    }

    /// Update the in-flight inference count.
    pub fn set_active_inference_count(&mut self, count: u32) {
        self.active_inference_count = count;
    }

    /// Increment the in-flight inference count.
    pub fn inference_started(&mut self) {
        self.active_inference_count = self.active_inference_count.saturating_add(1);
    }

    /// Decrement the in-flight inference count.
    pub fn inference_finished(&mut self) {
        self.active_inference_count = self.active_inference_count.saturating_sub(1);
    }

    /// Human-readable summary of current load.
    pub fn summary(&self) -> String {
        match self.latest() {
            Some(snap) => {
                format!(
                    "Load score: {:.2} (CPU: {:.1}%, RAM: {:.1}%, models: {}, inferences: {})",
                    snap.load_score(),
                    snap.cpu_usage_percent,
                    snap.memory_utilization() * 100.0,
                    snap.active_model_count,
                    snap.active_inference_count,
                )
            }
            None => "No load data yet".to_string(),
        }
    }
}

impl Default for LoadTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a shared load tracker.
pub fn create_load_tracker() -> SharedLoadTracker {
    Arc::new(RwLock::new(LoadTracker::new()))
}

/// Spawn a background task that samples system resources at a fixed interval.
///
/// Returns a [`tokio::task::JoinHandle`] that can be used to abort the
/// sampling loop on shutdown.
pub fn spawn_load_sampler(
    tracker: SharedLoadTracker,
    interval: Option<Duration>,
) -> tokio::task::JoinHandle<()> {
    let interval = interval.unwrap_or(DEFAULT_SAMPLE_INTERVAL);

    tokio::spawn(async move {
        // sysinfo needs two refreshes to compute meaningful CPU %
        let mut sys = System::new_all();
        sys.refresh_all();
        tokio::time::sleep(Duration::from_millis(500)).await;

        loop {
            sys.refresh_cpu_all();
            sys.refresh_memory();

            {
                let mut tracker = tracker.write().await;
                tracker.sample(&sys);
                debug!(
                    score = tracker.load_score(),
                    cpu = %format!("{:.1}%", sys.global_cpu_usage()),
                    "Load sample"
                );
            }

            tokio::time::sleep(interval).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_load_score_range() {
        let snap = LoadSnapshot {
            uptime_secs: 0,
            cpu_usage_percent: 50.0,
            memory_used_bytes: 4_000_000_000,
            memory_total_bytes: 8_000_000_000,
            swap_used_bytes: 0,
            swap_total_bytes: 2_000_000_000,
            active_model_count: 1,
            active_inference_count: 2,
        };
        let score = snap.load_score();
        assert!((0.0..=1.0).contains(&score), "score was {score}");
    }

    #[test]
    fn idle_system_scores_low() {
        let snap = LoadSnapshot {
            uptime_secs: 10,
            cpu_usage_percent: 2.0,
            memory_used_bytes: 1_000_000_000,
            memory_total_bytes: 16_000_000_000,
            swap_used_bytes: 0,
            swap_total_bytes: 0,
            active_model_count: 0,
            active_inference_count: 0,
        };
        assert!(snap.load_score() < 0.2);
    }

    #[test]
    fn tracker_sample_and_summary() {
        let mut tracker = LoadTracker::new();
        let sys = System::new_all();
        tracker.sample(&sys);
        assert!(tracker.latest().is_some());
        assert!(!tracker.summary().is_empty());
    }

    #[test]
    fn inference_counting() {
        let mut tracker = LoadTracker::new();
        tracker.inference_started();
        tracker.inference_started();
        assert_eq!(tracker.active_inference_count, 2);
        tracker.inference_finished();
        assert_eq!(tracker.active_inference_count, 1);
    }

    #[test]
    fn sliding_window_cap() {
        let mut tracker = LoadTracker::new();
        let sys = System::new_all();
        for _ in 0..MAX_SNAPSHOTS + 10 {
            tracker.sample(&sys);
        }
        assert_eq!(tracker.snapshots.len(), MAX_SNAPSHOTS);
    }
}

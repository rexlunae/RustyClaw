//! Model failover and auth profile rotation.
//!
//! Provides automatic failover when a model provider returns errors, and
//! rotation through multiple auth profiles (API keys) for load distribution
//! and rate-limit avoidance.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// An auth profile — a named set of credentials for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    /// Profile name (e.g., "primary", "backup", "org-key").
    pub name: String,
    /// Provider this profile is for (e.g., "openai", "anthropic").
    pub provider: String,
    /// API key or token.
    pub api_key: String,
    /// Optional base URL override.
    pub base_url: Option<String>,
    /// Whether this profile is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Max requests per minute (0 = unlimited).
    #[serde(default)]
    pub rate_limit_rpm: u32,
}

fn default_true() -> bool {
    true
}

/// Tracks the health of a model/profile combination.
#[derive(Debug)]
pub struct HealthTracker {
    /// Consecutive failures.
    consecutive_failures: AtomicU64,
    /// Total requests.
    total_requests: AtomicU64,
    /// Total failures.
    total_failures: AtomicU64,
    /// Unix timestamp when the circuit was opened (0 = closed).
    circuit_open_since: AtomicU64,
    /// Duration in seconds before retrying after circuit opens.
    circuit_break_secs: u64,
}

impl HealthTracker {
    pub fn new(circuit_break_secs: u64) -> Self {
        Self {
            consecutive_failures: AtomicU64::new(0),
            total_requests: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            circuit_open_since: AtomicU64::new(0),
            circuit_break_secs,
        }
    }

    /// Record a successful request.
    pub fn record_success(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures.store(0, Ordering::Relaxed);
        // Close circuit on success
        self.circuit_open_since.store(0, Ordering::Relaxed);
    }

    /// Record a failed request.
    pub fn record_failure(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_failures.fetch_add(1, Ordering::Relaxed);
        let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;

        // Open circuit after 3 consecutive failures
        if failures >= 3 {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            self.circuit_open_since.store(now, Ordering::Relaxed);
            warn!(
                consecutive_failures = failures,
                "Circuit breaker opened after {} consecutive failures", failures
            );
        }
    }

    /// Check if the circuit is open (should not send requests).
    pub fn is_circuit_open(&self) -> bool {
        let opened_at = self.circuit_open_since.load(Ordering::Relaxed);
        if opened_at == 0 {
            return false;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let elapsed = now.saturating_sub(opened_at);

        // Allow retry after circuit_break_secs (half-open state)
        if elapsed >= self.circuit_break_secs {
            debug!(
                elapsed_secs = elapsed,
                "Circuit breaker entering half-open state"
            );
            return false;
        }

        true
    }

    /// Get health statistics.
    pub fn stats(&self) -> HealthStats {
        HealthStats {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            total_failures: self.total_failures.load(Ordering::Relaxed),
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
            circuit_open: self.is_circuit_open(),
        }
    }
}

/// Health statistics snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct HealthStats {
    pub total_requests: u64,
    pub total_failures: u64,
    pub consecutive_failures: u64,
    pub circuit_open: bool,
}

/// Failover strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailoverStrategy {
    /// Try profiles in order, skip to next on failure.
    #[default]
    Sequential,
    /// Round-robin across healthy profiles.
    RoundRobin,
    /// Random selection from healthy profiles.
    Random,
}

/// Failover configuration for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverConfig {
    /// Profiles for this provider.
    #[serde(default)]
    pub profiles: Vec<AuthProfile>,

    /// Failover strategy.
    #[serde(default)]
    pub strategy: FailoverStrategy,

    /// Fallback model IDs to try if all profiles for the primary model fail.
    #[serde(default)]
    pub fallback_models: Vec<String>,

    /// Seconds before retrying a circuit-broken profile.
    #[serde(default = "default_circuit_break")]
    pub circuit_break_secs: u64,

    /// Maximum retries before giving up entirely.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_circuit_break() -> u64 {
    60
}

fn default_max_retries() -> u32 {
    3
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self {
            profiles: Vec::new(),
            strategy: FailoverStrategy::default(),
            fallback_models: Vec::new(),
            circuit_break_secs: default_circuit_break(),
            max_retries: default_max_retries(),
        }
    }
}

/// Model failover manager.
///
/// Manages auth profile rotation and automatic failover for model providers.
pub struct FailoverManager {
    /// Per-provider failover configs.
    configs: HashMap<String, FailoverConfig>,
    /// Health trackers keyed by "provider/profile_name".
    health: HashMap<String, HealthTracker>,
    /// Round-robin index per provider.
    rr_index: HashMap<String, AtomicUsize>,
}

impl FailoverManager {
    /// Create a new failover manager.
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            health: HashMap::new(),
            rr_index: HashMap::new(),
        }
    }

    /// Register a failover configuration for a provider.
    pub fn register(&mut self, provider: String, config: FailoverConfig) {
        let circuit_secs = config.circuit_break_secs;

        // Create health trackers for each profile
        for profile in &config.profiles {
            let key = format!("{}/{}", provider, profile.name);
            self.health.insert(key, HealthTracker::new(circuit_secs));
        }

        self.rr_index.insert(provider.clone(), AtomicUsize::new(0));
        self.configs.insert(provider, config);

        info!("Failover config registered");
    }

    /// Select the next auth profile to use for a provider.
    ///
    /// Returns `None` if no healthy profiles are available.
    pub fn select_profile(&self, provider: &str) -> Option<&AuthProfile> {
        let config = self.configs.get(provider)?;
        let healthy: Vec<_> = config
            .profiles
            .iter()
            .filter(|p| {
                if !p.enabled {
                    return false;
                }
                let key = format!("{}/{}", provider, p.name);
                if let Some(tracker) = self.health.get(&key) {
                    !tracker.is_circuit_open()
                } else {
                    true
                }
            })
            .collect();

        if healthy.is_empty() {
            warn!(provider = %provider, "No healthy profiles available");
            return None;
        }

        match config.strategy {
            FailoverStrategy::Sequential => Some(healthy[0]),
            FailoverStrategy::RoundRobin => {
                if let Some(idx) = self.rr_index.get(provider) {
                    let i = idx.fetch_add(1, Ordering::Relaxed) % healthy.len();
                    Some(healthy[i])
                } else {
                    Some(healthy[0])
                }
            }
            FailoverStrategy::Random => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_nanos();
                let mut hasher = DefaultHasher::new();
                now.hash(&mut hasher);
                let i = hasher.finish() as usize % healthy.len();
                Some(healthy[i])
            }
        }
    }

    /// Record a successful request for a profile.
    pub fn record_success(&self, provider: &str, profile_name: &str) {
        let key = format!("{}/{}", provider, profile_name);
        if let Some(tracker) = self.health.get(&key) {
            tracker.record_success();
        }
    }

    /// Record a failed request for a profile.
    pub fn record_failure(&self, provider: &str, profile_name: &str) {
        let key = format!("{}/{}", provider, profile_name);
        if let Some(tracker) = self.health.get(&key) {
            tracker.record_failure();
        }
    }

    /// Get fallback model IDs for a provider.
    pub fn fallback_models(&self, provider: &str) -> &[String] {
        self.configs
            .get(provider)
            .map(|c| c.fallback_models.as_slice())
            .unwrap_or(&[])
    }

    /// Get health stats for all profiles.
    pub fn health_report(&self) -> HashMap<String, HealthStats> {
        self.health
            .iter()
            .map(|(key, tracker)| (key.clone(), tracker.stats()))
            .collect()
    }

    /// Check if a provider has failover configured.
    pub fn has_failover(&self, provider: &str) -> bool {
        self.configs
            .get(provider)
            .map(|c| c.profiles.len() > 1 || !c.fallback_models.is_empty())
            .unwrap_or(false)
    }
}

impl Default for FailoverManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile(name: &str, provider: &str) -> AuthProfile {
        AuthProfile {
            name: name.to_string(),
            provider: provider.to_string(),
            api_key: format!("key-{}", name),
            base_url: None,
            enabled: true,
            rate_limit_rpm: 0,
        }
    }

    #[test]
    fn test_health_tracker_success_resets_failures() {
        let tracker = HealthTracker::new(60);
        tracker.record_failure();
        tracker.record_failure();
        assert_eq!(tracker.stats().consecutive_failures, 2);

        tracker.record_success();
        assert_eq!(tracker.stats().consecutive_failures, 0);
    }

    #[test]
    fn test_circuit_opens_after_3_failures() {
        let tracker = HealthTracker::new(60);
        tracker.record_failure();
        tracker.record_failure();
        assert!(!tracker.is_circuit_open());

        tracker.record_failure();
        assert!(tracker.is_circuit_open());
    }

    #[test]
    fn test_failover_sequential() {
        let mut mgr = FailoverManager::new();
        mgr.register(
            "openai".to_string(),
            FailoverConfig {
                profiles: vec![
                    make_profile("primary", "openai"),
                    make_profile("backup", "openai"),
                ],
                strategy: FailoverStrategy::Sequential,
                ..Default::default()
            },
        );

        let profile = mgr.select_profile("openai").unwrap();
        assert_eq!(profile.name, "primary");
    }

    #[test]
    fn test_failover_round_robin() {
        let mut mgr = FailoverManager::new();
        mgr.register(
            "openai".to_string(),
            FailoverConfig {
                profiles: vec![make_profile("a", "openai"), make_profile("b", "openai")],
                strategy: FailoverStrategy::RoundRobin,
                ..Default::default()
            },
        );

        let first = mgr.select_profile("openai").unwrap().name.clone();
        let second = mgr.select_profile("openai").unwrap().name.clone();
        assert_ne!(first, second);
    }

    #[test]
    fn test_failover_skips_broken_circuit() {
        let mut mgr = FailoverManager::new();
        mgr.register(
            "openai".to_string(),
            FailoverConfig {
                profiles: vec![
                    make_profile("primary", "openai"),
                    make_profile("backup", "openai"),
                ],
                strategy: FailoverStrategy::Sequential,
                ..Default::default()
            },
        );

        // Break primary's circuit
        for _ in 0..3 {
            mgr.record_failure("openai", "primary");
        }

        let profile = mgr.select_profile("openai").unwrap();
        assert_eq!(profile.name, "backup");
    }

    #[test]
    fn test_fallback_models() {
        let mut mgr = FailoverManager::new();
        mgr.register(
            "anthropic".to_string(),
            FailoverConfig {
                profiles: vec![make_profile("main", "anthropic")],
                fallback_models: vec!["openai/gpt-4.1".to_string()],
                ..Default::default()
            },
        );

        assert_eq!(mgr.fallback_models("anthropic"), &["openai/gpt-4.1"]);
        assert!(mgr.has_failover("anthropic"));
    }

    #[test]
    fn test_no_failover() {
        let mgr = FailoverManager::new();
        assert!(!mgr.has_failover("missing"));
        assert!(mgr.select_profile("missing").is_none());
    }

    #[test]
    fn test_health_report() {
        let mut mgr = FailoverManager::new();
        mgr.register(
            "openai".to_string(),
            FailoverConfig {
                profiles: vec![make_profile("main", "openai")],
                ..Default::default()
            },
        );

        mgr.record_success("openai", "main");
        mgr.record_success("openai", "main");
        mgr.record_failure("openai", "main");

        let report = mgr.health_report();
        let stats = report.get("openai/main").unwrap();
        assert_eq!(stats.total_requests, 3);
        assert_eq!(stats.total_failures, 1);
        assert_eq!(stats.consecutive_failures, 1);
    }
}

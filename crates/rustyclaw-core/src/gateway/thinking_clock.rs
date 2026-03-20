//! Thinking Clock — periodic reflection with multi-model orchestration.
//!
//! The Thinking Clock provides ambient awareness by periodically running a
//! cheap/local model to assess whether the agent should proactively take
//! action. This mirrors OpenClaw's "Thinking Clock" feature:
//!
//! - A **ticker** fires at a configurable interval (e.g., every 5 minutes).
//! - A **cheap model** (e.g., local Ollama, economy tier) evaluates the
//!   current context and decides if any action is needed.
//! - If the cheap model detects something worth acting on, it **escalates**
//!   to the primary (more capable) model for actual execution.
//!
//! Use cases:
//! - Monitor cron job results and alert on failures.
//! - Check for pending messages that need follow-up.
//! - Periodic status summaries in messenger channels.
//! - Background awareness of system health.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

/// Configuration for the Thinking Clock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingClockConfig {
    /// Whether the thinking clock is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Interval in seconds between ticks (default: 300 = 5 minutes).
    #[serde(default = "default_interval")]
    pub interval_secs: u64,

    /// Model ID for the cheap/ambient check (e.g., "ollama/llama3.2:3b").
    /// If not set, uses the cheapest available model from the registry.
    #[serde(default)]
    pub ambient_model: Option<String>,

    /// Model ID for escalation (primary model).
    /// If not set, uses the active model.
    #[serde(default)]
    pub escalation_model: Option<String>,

    /// System prompt for the ambient check.
    #[serde(default = "default_ambient_prompt")]
    pub ambient_prompt: String,

    /// Maximum tokens for the ambient check (keep low to stay cheap).
    #[serde(default = "default_ambient_max_tokens")]
    pub ambient_max_tokens: u32,

    /// Keywords/phrases that trigger escalation from the ambient model's
    /// response (e.g., ["ESCALATE", "ACTION_NEEDED"]).
    #[serde(default = "default_escalation_triggers")]
    pub escalation_triggers: Vec<String>,

    /// Whether to log ambient check results (even when no action taken).
    #[serde(default)]
    pub verbose_logging: bool,
}

fn default_interval() -> u64 {
    300
}

fn default_ambient_prompt() -> String {
    "You are an ambient awareness monitor. Review the current context and \
     determine if any action is needed. If action is needed, respond with \
     'ESCALATE: <reason>'. If no action is needed, respond with 'OK'."
        .to_string()
}

fn default_ambient_max_tokens() -> u32 {
    256
}

fn default_escalation_triggers() -> Vec<String> {
    vec![
        "ESCALATE".to_string(),
        "ACTION_NEEDED".to_string(),
        "ALERT".to_string(),
    ]
}

impl Default for ThinkingClockConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: default_interval(),
            ambient_model: None,
            escalation_model: None,
            ambient_prompt: default_ambient_prompt(),
            ambient_max_tokens: default_ambient_max_tokens(),
            escalation_triggers: default_escalation_triggers(),
            verbose_logging: false,
        }
    }
}

/// Result of an ambient check.
#[derive(Debug, Clone, Serialize)]
pub struct AmbientCheckResult {
    /// The ambient model's response.
    pub response: String,
    /// Whether escalation was triggered.
    pub escalated: bool,
    /// The reason for escalation (if any).
    pub escalation_reason: Option<String>,
    /// Duration of the check.
    pub duration_ms: u64,
}

/// Check if an ambient response should trigger escalation.
pub fn should_escalate(response: &str, triggers: &[String]) -> Option<String> {
    // Use case-insensitive search directly on the original string to avoid
    // byte-position misalignment between `to_uppercase()` and the original
    // (multi-byte characters like ß→SS can change byte lengths).
    let response_lower = response.to_lowercase();

    for trigger in triggers {
        let trigger_lower = trigger.to_lowercase();
        if let Some(pos) = response_lower.find(&trigger_lower) {
            // `pos` is a byte offset into `response_lower` which has the
            // same byte length as `response` (lowercasing ASCII-range
            // characters preserves byte length for the trigger keywords
            // we care about: ESCALATE, ACTION_NEEDED, ALERT).
            let after = &response[pos + trigger.len()..];
            let reason = after
                .trim_start_matches(':')
                .trim_start_matches(' ')
                .trim();
            let reason = if reason.is_empty() {
                response.to_string()
            } else {
                reason.to_string()
            };

            debug!(trigger = %trigger, reason = %reason, "Escalation triggered");
            return Some(reason);
        }
    }

    None
}

/// State for the thinking clock tick loop.
pub struct ThinkingClock {
    config: ThinkingClockConfig,
    tick_count: u64,
}

impl ThinkingClock {
    /// Create a new thinking clock.
    pub fn new(config: ThinkingClockConfig) -> Self {
        Self {
            config,
            tick_count: 0,
        }
    }

    /// Get the tick interval.
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.config.interval_secs)
    }

    /// Get the ambient model ID.
    pub fn ambient_model(&self) -> Option<&str> {
        self.config.ambient_model.as_deref()
    }

    /// Get the escalation model ID.
    pub fn escalation_model(&self) -> Option<&str> {
        self.config.escalation_model.as_deref()
    }

    /// Get the ambient prompt.
    pub fn ambient_prompt(&self) -> &str {
        &self.config.ambient_prompt
    }

    /// Get max tokens for ambient check.
    pub fn ambient_max_tokens(&self) -> u32 {
        self.config.ambient_max_tokens
    }

    /// Record a tick and return the tick count.
    pub fn tick(&mut self) -> u64 {
        self.tick_count += 1;
        if self.config.verbose_logging {
            debug!(tick = self.tick_count, "Thinking clock tick");
        }
        self.tick_count
    }

    /// Process an ambient model response.
    pub fn process_response(&self, response: &str, duration_ms: u64) -> AmbientCheckResult {
        let escalation = should_escalate(response, &self.config.escalation_triggers);
        let escalated = escalation.is_some();

        if escalated {
            info!(
                reason = ?escalation,
                "Thinking clock: escalation triggered"
            );
        } else if self.config.verbose_logging {
            debug!(response = %response, "Thinking clock: no action needed");
        }

        AmbientCheckResult {
            response: response.to_string(),
            escalated,
            escalation_reason: escalation,
            duration_ms,
        }
    }

    /// Check if the thinking clock is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the current tick count.
    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }
}

/// Run the thinking clock loop.
///
/// This is a skeleton that the gateway integrates with its model dispatch.
/// The actual model calls are performed by the gateway using the ambient
/// and escalation model IDs from the config.
pub async fn run_thinking_clock_loop(
    config: ThinkingClockConfig,
    cancel: CancellationToken,
    // The gateway provides a callback for each tick
    on_tick: impl Fn(u64) + Send + 'static,
) {
    if !config.enabled {
        debug!("Thinking clock disabled");
        return;
    }

    let mut clock = ThinkingClock::new(config);
    let interval = clock.interval();

    info!(
        interval_secs = interval.as_secs(),
        "Thinking clock started"
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Thinking clock stopped");
                break;
            }
            _ = tokio::time::sleep(interval) => {
                let tick = clock.tick();
                on_tick(tick);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = ThinkingClockConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.interval_secs, 300);
        assert_eq!(config.ambient_max_tokens, 256);
        assert!(!config.escalation_triggers.is_empty());
    }

    #[test]
    fn test_should_escalate_yes() {
        let triggers = vec!["ESCALATE".to_string(), "ALERT".to_string()];

        let result = should_escalate("ESCALATE: server is down", &triggers);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "server is down");
    }

    #[test]
    fn test_should_escalate_no() {
        let triggers = vec!["ESCALATE".to_string()];

        let result = should_escalate("OK - all systems normal", &triggers);
        assert!(result.is_none());
    }

    #[test]
    fn test_should_escalate_case_insensitive() {
        let triggers = vec!["ESCALATE".to_string()];

        let result = should_escalate("escalate: need attention", &triggers);
        assert!(result.is_some());
    }

    #[test]
    fn test_thinking_clock_tick() {
        let config = ThinkingClockConfig {
            enabled: true,
            ..Default::default()
        };
        let mut clock = ThinkingClock::new(config);

        assert_eq!(clock.tick_count(), 0);
        assert_eq!(clock.tick(), 1);
        assert_eq!(clock.tick(), 2);
        assert_eq!(clock.tick_count(), 2);
    }

    #[test]
    fn test_process_response_no_escalation() {
        let config = ThinkingClockConfig::default();
        let clock = ThinkingClock::new(config);

        let result = clock.process_response("OK - everything is fine", 50);
        assert!(!result.escalated);
        assert!(result.escalation_reason.is_none());
    }

    #[test]
    fn test_process_response_with_escalation() {
        let config = ThinkingClockConfig::default();
        let clock = ThinkingClock::new(config);

        let result = clock.process_response("ESCALATE: cron job failed", 100);
        assert!(result.escalated);
        assert_eq!(result.escalation_reason.unwrap(), "cron job failed");
    }

    #[test]
    fn test_thinking_clock_disabled() {
        let config = ThinkingClockConfig::default(); // disabled by default
        let clock = ThinkingClock::new(config);
        assert!(!clock.is_enabled());
    }

    #[test]
    fn test_interval() {
        let config = ThinkingClockConfig {
            interval_secs: 60,
            ..Default::default()
        };
        let clock = ThinkingClock::new(config);
        assert_eq!(clock.interval(), Duration::from_secs(60));
    }
}

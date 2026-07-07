//! In-memory stats and log-ring observer.
//!
//! [`StatsObserver`] aggregates LLM/tool telemetry so the gateway's
//! analytics and logs panels have something to query. It keeps two
//! bounded buffers:
//!
//! * per-call records of every LLM request (provider, model, tokens,
//!   latency, success), aggregated on demand by [`StatsObserver::usage`];
//! * a ring of human-readable event lines served by
//!   [`StatsObserver::recent_logs`].
//!
//! Registered in [`crate::runtime_ctx`] by the gateway at startup so the
//! panel handler can reach it without extra plumbing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::traits::{Observer, ObserverEvent, ObserverMetric};

/// Maximum retained LLM call records.
const MAX_CALLS: usize = 10_000;
/// Maximum retained log lines.
const MAX_LOG_LINES: usize = 2_000;

/// One completed LLM provider call.
#[derive(Debug, Clone)]
pub struct LlmCallRecord {
    /// Wall-clock time of completion (ms since epoch).
    pub ts_ms: u64,
    pub provider: String,
    pub model: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub latency_ms: u64,
    pub success: bool,
}

/// Aggregated usage for one (provider, model) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelUsage {
    pub provider: String,
    pub model: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub avg_latency_ms: u64,
}

/// Aggregated usage totals over a queried window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageSnapshot {
    pub total_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_latency_ms: u64,
    pub per_model: Vec<ModelUsage>,
}

/// Observer that aggregates usage stats and keeps a log ring.
#[derive(Default)]
pub struct StatsObserver {
    calls: Mutex<VecDeque<LlmCallRecord>>,
    logs: Mutex<VecDeque<String>>,
}

/// Shared handle to the stats observer.
pub type SharedStatsObserver = Arc<StatsObserver>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl StatsObserver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Aggregate usage for calls at or after `since_ms` (`None` = all).
    pub fn usage(&self, since_ms: Option<u64>) -> UsageSnapshot {
        let calls = match self.calls.lock() {
            Ok(calls) => calls,
            Err(_) => return UsageSnapshot::default(),
        };
        let mut snapshot = UsageSnapshot::default();
        let mut latency_totals: Vec<u64> = Vec::new();
        for call in calls.iter() {
            if since_ms.is_some_and(|since| call.ts_ms < since) {
                continue;
            }
            snapshot.total_requests += 1;
            snapshot.total_input_tokens += call.input_tokens.unwrap_or(0);
            snapshot.total_output_tokens += call.output_tokens.unwrap_or(0);
            snapshot.total_latency_ms += call.latency_ms;

            match snapshot
                .per_model
                .iter_mut()
                .zip(latency_totals.iter_mut())
                .find(|(m, _)| m.provider == call.provider && m.model == call.model)
            {
                Some((entry, latency_total)) => {
                    entry.requests += 1;
                    entry.input_tokens += call.input_tokens.unwrap_or(0);
                    entry.output_tokens += call.output_tokens.unwrap_or(0);
                    *latency_total += call.latency_ms;
                }
                None => {
                    snapshot.per_model.push(ModelUsage {
                        provider: call.provider.clone(),
                        model: call.model.clone(),
                        requests: 1,
                        input_tokens: call.input_tokens.unwrap_or(0),
                        output_tokens: call.output_tokens.unwrap_or(0),
                        avg_latency_ms: 0,
                    });
                    latency_totals.push(call.latency_ms);
                }
            }
        }
        for (entry, latency_total) in snapshot.per_model.iter_mut().zip(latency_totals) {
            entry.avg_latency_ms = latency_total / entry.requests.max(1);
        }
        snapshot
            .per_model
            .sort_by_key(|m| std::cmp::Reverse(m.requests));
        snapshot
    }

    /// The most recent `tail` log lines, oldest first.
    pub fn recent_logs(&self, tail: usize) -> Vec<String> {
        match self.logs.lock() {
            Ok(logs) => {
                let skip = logs.len().saturating_sub(tail);
                logs.iter().skip(skip).cloned().collect()
            }
            Err(_) => Vec::new(),
        }
    }

    fn push_call(&self, record: LlmCallRecord) {
        if let Ok(mut calls) = self.calls.lock() {
            if calls.len() >= MAX_CALLS {
                calls.pop_front();
            }
            calls.push_back(record);
        }
    }

    fn push_log(&self, line: String) {
        let ts = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S");
        if let Ok(mut logs) = self.logs.lock() {
            if logs.len() >= MAX_LOG_LINES {
                logs.pop_front();
            }
            logs.push_back(format!("[{}] {}", ts, line));
        }
    }
}

impl Observer for StatsObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                self.push_log(format!("agent start: {}/{}", provider, model));
            }
            ObserverEvent::LlmRequest {
                provider,
                model,
                messages_count,
            } => {
                self.push_log(format!(
                    "llm request: {}/{} ({} messages)",
                    provider, model, messages_count
                ));
            }
            ObserverEvent::LlmResponse {
                provider,
                model,
                duration,
                success,
                error_message,
                input_tokens,
                output_tokens,
            } => {
                self.push_call(LlmCallRecord {
                    ts_ms: now_ms(),
                    provider: provider.clone(),
                    model: model.clone(),
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    latency_ms: duration.as_millis() as u64,
                    success: *success,
                });
                let tokens = match (input_tokens, output_tokens) {
                    (Some(i), Some(o)) => format!(", {}→{} tokens", i, o),
                    _ => String::new(),
                };
                match error_message {
                    Some(err) => self.push_log(format!(
                        "llm response: {}/{} FAILED after {:?}: {}",
                        provider, model, duration, err
                    )),
                    None => self.push_log(format!(
                        "llm response: {}/{} in {:?}{}",
                        provider, model, duration, tokens
                    )),
                }
            }
            ObserverEvent::AgentEnd {
                provider,
                model,
                duration,
                tokens_used,
                cost_usd,
            } => {
                let extra = match (tokens_used, cost_usd) {
                    (Some(t), Some(c)) => format!(" ({} tokens, ${:.4})", t, c),
                    (Some(t), None) => format!(" ({} tokens)", t),
                    _ => String::new(),
                };
                self.push_log(format!(
                    "agent end: {}/{} after {:?}{}",
                    provider, model, duration, extra
                ));
            }
            ObserverEvent::ToolCallStart { tool } => {
                self.push_log(format!("tool start: {}", tool));
            }
            ObserverEvent::ToolCall {
                tool,
                duration,
                success,
            } => {
                let outcome = if *success { "ok" } else { "FAILED" };
                self.push_log(format!("tool {}: {} in {:?}", outcome, tool, duration));
            }
            ObserverEvent::TurnComplete => {
                self.push_log("turn complete".to_string());
            }
            ObserverEvent::ChannelMessage { channel, direction } => {
                self.push_log(format!("channel {}: {}", direction, channel));
            }
            ObserverEvent::HeartbeatTick => {}
            ObserverEvent::Error { component, message } => {
                self.push_log(format!("ERROR [{}]: {}", component, message));
            }
        }
    }

    fn record_metric(&self, _metric: &ObserverMetric) {}

    fn name(&self) -> &str {
        "stats"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn record_response(obs: &StatsObserver, model: &str, tokens: u64, ms: u64) {
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "test".into(),
            model: model.into(),
            duration: Duration::from_millis(ms),
            success: true,
            error_message: None,
            input_tokens: Some(tokens),
            output_tokens: Some(tokens / 2),
        });
    }

    #[test]
    fn usage_aggregates_per_model() {
        let obs = StatsObserver::new();
        record_response(&obs, "a", 100, 200);
        record_response(&obs, "a", 300, 400);
        record_response(&obs, "b", 10, 50);

        let usage = obs.usage(None);
        assert_eq!(usage.total_requests, 3);
        assert_eq!(usage.total_input_tokens, 410);
        assert_eq!(usage.total_output_tokens, 205);
        assert_eq!(usage.per_model.len(), 2);
        // Sorted by request count, descending.
        assert_eq!(usage.per_model[0].model, "a");
        assert_eq!(usage.per_model[0].requests, 2);
        assert_eq!(usage.per_model[0].avg_latency_ms, 300);
    }

    #[test]
    fn usage_since_filters_by_time() {
        let obs = StatsObserver::new();
        record_response(&obs, "a", 100, 200);
        // Everything so far is "before the future".
        let usage = obs.usage(Some(now_ms() + 10_000));
        assert_eq!(usage.total_requests, 0);
        let usage = obs.usage(Some(0));
        assert_eq!(usage.total_requests, 1);
    }

    #[test]
    fn log_ring_keeps_recent_lines() {
        let obs = StatsObserver::new();
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "read_file".into(),
        });
        obs.record_event(&ObserverEvent::Error {
            component: "provider".into(),
            message: "boom".into(),
        });
        let logs = obs.recent_logs(10);
        assert_eq!(logs.len(), 2);
        assert!(logs[0].contains("tool start: read_file"));
        assert!(logs[1].contains("ERROR [provider]: boom"));
        // Tail smaller than the buffer returns the newest lines.
        let logs = obs.recent_logs(1);
        assert_eq!(logs.len(), 1);
        assert!(logs[0].contains("boom"));
    }
}

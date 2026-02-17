use std::time::Duration;

/// Strategy for retrying transient failures with exponential backoff.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum attempts including the first request.
    pub max_attempts: u32,
    /// Base delay for the first retry.
    pub base_delay: Duration,
    /// Maximum delay cap for later retries.
    pub max_delay: Duration,
    /// Jitter ratio (0.0..=1.0) applied to delay.
    pub jitter_ratio: f64,
}

impl RetryPolicy {
    /// Default policy for outbound HTTP provider requests.
    pub fn http_default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(8),
            jitter_ratio: 0.20,
        }
    }

    /// Exponential backoff delay for the given retry index (1-based).
    pub fn backoff_delay(&self, retry_index: u32) -> Duration {
        let shift = retry_index.saturating_sub(1).min(31);
        let multiplier = 1u32 << shift;
        let base = self
            .base_delay
            .checked_mul(multiplier)
            .unwrap_or(self.max_delay);
        base.min(self.max_delay)
    }

    /// Apply jitter to a delay using a symmetric random range.
    pub fn with_jitter(&self, delay: Duration) -> Duration {
        if self.jitter_ratio <= 0.0 {
            return delay;
        }
        let ratio = self.jitter_ratio.clamp(0.0, 1.0);
        let millis = delay.as_millis() as f64;
        let spread = millis * ratio;
        let low = (millis - spread).max(0.0);
        let high = millis + spread;
        let sampled = if high <= low {
            low
        } else {
            rand::random::<f64>() * (high - low) + low
        };
        Duration::from_millis(sampled.round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
            jitter_ratio: 0.0,
        };
        assert_eq!(policy.backoff_delay(1), Duration::from_millis(100));
        assert_eq!(policy.backoff_delay(2), Duration::from_millis(200));
        assert_eq!(policy.backoff_delay(3), Duration::from_millis(400));
        assert_eq!(policy.backoff_delay(4), Duration::from_millis(500));
    }
}

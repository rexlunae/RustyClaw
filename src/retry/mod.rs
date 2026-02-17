mod policy;

pub use policy::RetryPolicy;

use std::future::Future;
use std::time::Duration;

/// Classification of transient retry causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryReason {
    Connect,
    Timeout,
    RateLimited,
    ServerError,
    RequestTimeout,
}

impl RetryReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Timeout => "timeout",
            Self::RateLimited => "rate_limited",
            Self::ServerError => "server_error",
            Self::RequestTimeout => "request_timeout",
        }
    }
}

/// Retry decision for one attempt result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry {
        reason: RetryReason,
        retry_after: Option<Duration>,
    },
    DoNotRetry,
}

/// Metadata for one scheduled retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryAttempt {
    pub attempt: u32,
    pub delay: Duration,
    pub reason: RetryReason,
}

/// Parse `Retry-After` header value as a delay.
///
/// Supports:
/// - Delta-seconds (`Retry-After: 5`)
/// - HTTP-date (`Retry-After: Wed, 21 Oct 2015 07:28:00 GMT`)
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim();

    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }

    if let Ok(when) = httpdate::parse_http_date(raw) {
        let now = std::time::SystemTime::now();
        if let Ok(delay) = when.duration_since(now) {
            return Some(delay);
        }
        return Some(Duration::from_secs(0));
    }

    None
}

/// Classify reqwest result into retry/no-retry.
pub fn classify_reqwest_result(
    result: &std::result::Result<reqwest::Response, reqwest::Error>,
) -> RetryDecision {
    match result {
        Ok(resp) => {
            let status = resp.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return RetryDecision::Retry {
                    reason: RetryReason::RateLimited,
                    retry_after: parse_retry_after(resp.headers()),
                };
            }
            if status == reqwest::StatusCode::REQUEST_TIMEOUT {
                return RetryDecision::Retry {
                    reason: RetryReason::RequestTimeout,
                    retry_after: parse_retry_after(resp.headers()),
                };
            }
            if status.is_server_error() {
                return RetryDecision::Retry {
                    reason: RetryReason::ServerError,
                    retry_after: parse_retry_after(resp.headers()),
                };
            }
            RetryDecision::DoNotRetry
        }
        Err(err) => {
            if err.is_timeout() {
                return RetryDecision::Retry {
                    reason: RetryReason::Timeout,
                    retry_after: None,
                };
            }
            if err.is_connect() || err.is_request() {
                return RetryDecision::Retry {
                    reason: RetryReason::Connect,
                    retry_after: None,
                };
            }
            RetryDecision::DoNotRetry
        }
    }
}

/// Retry an async operation with backoff according to `policy`.
///
/// - `operation(attempt)` is called with a 1-based attempt number.
/// - `classify(result)` decides whether to retry.
/// - `on_retry(info)` is called right before sleeping.
pub async fn retry_with_backoff<T, E, Op, Fut, Classify, OnRetry>(
    policy: &RetryPolicy,
    mut operation: Op,
    mut classify: Classify,
    mut on_retry: OnRetry,
) -> std::result::Result<T, E>
where
    Op: FnMut(u32) -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    Classify: FnMut(&std::result::Result<T, E>) -> RetryDecision,
    OnRetry: FnMut(RetryAttempt),
{
    let max_attempts = policy.max_attempts.max(1);

    for attempt in 1..=max_attempts {
        let result = operation(attempt).await;
        let decision = if attempt < max_attempts {
            classify(&result)
        } else {
            RetryDecision::DoNotRetry
        };

        match (decision, result) {
            (RetryDecision::Retry { reason, retry_after }, Err(err)) => {
                let backoff = policy.backoff_delay(attempt);
                let base_delay = retry_after.unwrap_or(backoff);
                let delay = policy.with_jitter(base_delay);
                on_retry(RetryAttempt {
                    attempt,
                    delay,
                    reason,
                });
                tokio::time::sleep(delay).await;
                let _ = err;
            }
            (RetryDecision::Retry { reason, retry_after }, Ok(_)) => {
                let backoff = policy.backoff_delay(attempt);
                let base_delay = retry_after.unwrap_or(backoff);
                let delay = policy.with_jitter(base_delay);
                on_retry(RetryAttempt {
                    attempt,
                    delay,
                    reason,
                });
                tokio::time::sleep(delay).await;
            }
            (_, final_result) => return final_result,
        }
    }

    unreachable!("retry loop always returns");
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn parse_retry_after_delta_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("7"));
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(7)));
    }

    #[tokio::test]
    async fn retry_helper_retries_until_success() {
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            jitter_ratio: 0.0,
        };
        let attempts = Arc::new(AtomicU32::new(0));
        let seen = attempts.clone();

        let result = retry_with_backoff(
            &policy,
            move |_attempt| {
                let seen = seen.clone();
                async move {
                    let n = seen.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < 3 {
                        Err("transient")
                    } else {
                        Ok("ok")
                    }
                }
            },
            |r: &std::result::Result<&str, &str>| match r {
                Err(_) => RetryDecision::Retry {
                    reason: RetryReason::Connect,
                    retry_after: None,
                },
                Ok(_) => RetryDecision::DoNotRetry,
            },
            |_info| {},
        )
        .await;

        assert_eq!(result, Ok("ok"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}

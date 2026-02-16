//! Multi-provider LLM failover support.
//!
//! Provides automatic failover between multiple LLM providers based on:
//! - Priority ordering (lower number = higher priority)
//! - Error classification (retryable vs fatal)
//! - Per-provider retry limits
//! - Cost tracking across providers
//!
//! ## Architecture
//!
//! The failover system wraps the existing retry engine (src/retry/mod.rs)
//! and provides provider-level failover on top of request-level retries.
//!
//! For each request:
//! 1. Select provider based on strategy (priority/round-robin/cost-optimized)
//! 2. Attempt request with retry_with_backoff (existing retry engine)
//! 3. If all retries fail with retryable errors, failover to next provider
//! 4. If error is fatal (auth, invalid request), fail immediately
//! 5. Track costs and failures per provider for future decisions

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::{Config, FailoverConfig, FailoverProvider};
use crate::secret::SecretString;
use crate::secrets::SecretsManager;
use super::types::ModelContext;

/// Provider statistics for failover decisions.
#[derive(Debug, Clone, Default)]
pub struct ProviderStats {
    /// Total number of requests attempted on this provider.
    requests: u64,
    /// Total number of failures (after retries).
    failures: u64,
    /// Total estimated cost (in USD) for this provider.
    cost_usd: f64,
    /// Timestamp of last failure (for backoff).
    last_failure: Option<std::time::Instant>,
}

/// Failover manager that selects providers and tracks their health.
pub struct FailoverManager {
    /// Failover configuration from config.toml
    config: FailoverConfig,
    /// Provider statistics indexed by provider ID.
    stats: Arc<RwLock<HashMap<String, ProviderStats>>>,
    /// Round-robin index for round-robin strategy.
    round_robin_index: Arc<RwLock<usize>>,
}

impl FailoverManager {
    /// Create a new failover manager from configuration.
    pub fn new(config: FailoverConfig) -> Self {
        Self {
            config,
            stats: Arc::new(RwLock::new(HashMap::new())),
            round_robin_index: Arc::new(RwLock::new(0)),
        }
    }

    /// Create failover manager from app config, or None if failover is disabled.
    pub fn from_config(config: &Config) -> Option<Self> {
        if !config.failover.enabled || config.failover.providers.is_empty() {
            return None;
        }
        Some(Self::new(config.failover.clone()))
    }

    /// Get the list of providers in priority order.
    pub fn providers(&self) -> Vec<FailoverProvider> {
        let mut providers = self.config.providers.clone();
        providers.sort_by_key(|p| p.priority);
        providers
    }

    /// Select the next provider to try based on the configured strategy.
    ///
    /// Returns the index into the providers array (after sorting by priority).
    pub async fn select_provider(&self) -> Result<usize> {
        match self.config.strategy.as_str() {
            "priority" => Ok(0), // Always use highest priority (lowest number)
            "round-robin" => {
                let mut index = self.round_robin_index.write().await;
                let providers = self.providers();
                if providers.is_empty() {
                    anyhow::bail!("No failover providers configured");
                }
                let selected = *index;
                *index = (*index + 1) % providers.len();
                Ok(selected)
            }
            "cost-optimized" => {
                // Select provider with lowest cost per request
                let stats = self.stats.read().await;
                let providers = self.providers();

                let selected = providers
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let a_stats = stats.get(&a.provider).cloned().unwrap_or_default();
                        let b_stats = stats.get(&b.provider).cloned().unwrap_or_default();

                        // Calculate cost per successful request
                        let a_cost_per_req = if a_stats.requests > a_stats.failures {
                            a_stats.cost_usd / (a_stats.requests - a_stats.failures) as f64
                        } else {
                            f64::MAX // Penalize providers with 100% failure rate
                        };

                        let b_cost_per_req = if b_stats.requests > b_stats.failures {
                            b_stats.cost_usd / (b_stats.requests - b_stats.failures) as f64
                        } else {
                            f64::MAX
                        };

                        a_cost_per_req.partial_cmp(&b_cost_per_req).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);

                Ok(selected)
            }
            other => {
                eprintln!("[failover] unknown strategy '{}', falling back to priority", other);
                Ok(0)
            }
        }
    }

    /// Record a successful request for cost tracking.
    pub async fn record_success(&self, provider: &str, cost_usd: f64) {
        let mut stats = self.stats.write().await;
        let entry = stats.entry(provider.to_string()).or_default();
        entry.requests += 1;
        entry.cost_usd += cost_usd;
    }

    /// Record a failed request (after all retries exhausted).
    pub async fn record_failure(&self, provider: &str) {
        let mut stats = self.stats.write().await;
        let entry = stats.entry(provider.to_string()).or_default();
        entry.requests += 1;
        entry.failures += 1;
        entry.last_failure = Some(std::time::Instant::now());
    }

    /// Get statistics for a provider.
    pub async fn get_stats(&self, provider: &str) -> ProviderStats {
        self.stats.read().await.get(provider).cloned().unwrap_or_default()
    }

    /// Convert a FailoverProvider config into a ModelContext.
    ///
    /// Resolves the API key from secrets manager and applies provider defaults.
    pub fn resolve_provider_context(
        provider_config: &FailoverProvider,
        _config: &Config,
        secrets: &mut SecretsManager,
    ) -> Result<ModelContext> {
        // Find provider definition from catalogue
        let provider_def = crate::providers::PROVIDERS
            .iter()
            .find(|p| p.id == provider_config.provider)
            .with_context(|| format!("Unknown provider: {}", provider_config.provider))?;

        // Get API key from secrets if the provider requires it
        let api_key = if let Some(secret_name) = provider_def.secret_key {
            secrets.get_secret(secret_name, true).ok().flatten().map(SecretString::new)
        } else {
            None
        };

        // Use provider-specific base URL or default
        let base_url = provider_config
            .base_url
            .clone()
            .or_else(|| provider_def.base_url.map(String::from))
            .unwrap_or_else(|| {
                // Fallback to generic OpenAI-compatible localhost
                "http://localhost:11434/v1".to_string()
            });

        // Use provider-specific model or first available model
        let model = provider_config
            .model
            .clone()
            .or_else(|| provider_def.models.first().map(|s| s.to_string()))
            .unwrap_or_else(|| "default".to_string());

        Ok(ModelContext {
            provider: provider_config.provider.clone(),
            model,
            base_url,
            api_key,
        })
    }

    /// Create a list of ModelContext instances for all failover providers.
    pub fn resolve_all_contexts(
        &self,
        config: &Config,
        secrets: &mut SecretsManager,
    ) -> Result<Vec<ModelContext>> {
        let providers = self.providers();
        let mut contexts = Vec::new();

        for provider_config in providers {
            match Self::resolve_provider_context(&provider_config, config, secrets) {
                Ok(ctx) => contexts.push(ctx),
                Err(e) => {
                    eprintln!(
                        "[failover] Failed to resolve provider '{}': {}",
                        provider_config.provider, e
                    );
                    // Continue with other providers
                }
            }
        }

        if contexts.is_empty() {
            anyhow::bail!("No valid failover providers could be resolved");
        }

        Ok(contexts)
    }
}

/// Classify an error to determine if we should failover to another provider.
///
/// Returns:
/// - `true` if the error is retryable/transient and we should try next provider
/// - `false` if the error is fatal (auth, bad request) and we should fail immediately
pub fn should_failover(error: &anyhow::Error) -> bool {
    let error_msg = error.to_string().to_lowercase();

    // Fatal errors - don't failover
    if error_msg.contains("unauthorized")
        || error_msg.contains("401")
        || error_msg.contains("403")
        || error_msg.contains("forbidden")
        || error_msg.contains("invalid api key")
        || error_msg.contains("authentication failed")
        || error_msg.contains("auth failed")
        || error_msg.contains("token exchange failed")
    {
        return false;
    }

    // Bad request errors - don't failover
    if error_msg.contains("400")
        || error_msg.contains("bad request")
        || error_msg.contains("invalid request")
        || error_msg.contains("validation error")
    {
        return false;
    }

    // Rate limit, timeout, connection errors - should failover
    if error_msg.contains("429")
        || error_msg.contains("rate limit")
        || error_msg.contains("timeout")
        || error_msg.contains("connection")
        || error_msg.contains("503")
        || error_msg.contains("502")
        || error_msg.contains("504")
        || error_msg.contains("500")
        || error_msg.contains("server error")
    {
        return true;
    }

    // Default: treat unknown errors as transient and allow failover
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_failover() {
        // Fatal errors - should NOT failover
        assert!(!should_failover(&anyhow::anyhow!("401 Unauthorized")));
        assert!(!should_failover(&anyhow::anyhow!("403 Forbidden")));
        assert!(!should_failover(&anyhow::anyhow!("Invalid API key")));
        assert!(!should_failover(&anyhow::anyhow!("400 Bad Request")));

        // Transient errors - should failover
        assert!(should_failover(&anyhow::anyhow!("429 Rate Limit Exceeded")));
        assert!(should_failover(&anyhow::anyhow!("Connection timeout")));
        assert!(should_failover(&anyhow::anyhow!("503 Service Unavailable")));
        assert!(should_failover(&anyhow::anyhow!("500 Internal Server Error")));

        // Unknown errors - should failover (conservative approach)
        assert!(should_failover(&anyhow::anyhow!("Something went wrong")));
    }

    #[tokio::test]
    async fn test_provider_selection_priority() {
        let config = FailoverConfig {
            enabled: true,
            strategy: "priority".to_string(),
            max_retries: 2,
            providers: vec![
                FailoverProvider {
                    provider: "anthropic".to_string(),
                    model: None,
                    base_url: None,
                    priority: 100,
                },
                FailoverProvider {
                    provider: "openai".to_string(),
                    model: None,
                    base_url: None,
                    priority: 50, // Higher priority (lower number)
                },
            ],
        };

        let manager = FailoverManager::new(config);
        let selected = manager.select_provider().await.unwrap();

        // Should select openai (priority 50) over anthropic (priority 100)
        let providers = manager.providers();
        assert_eq!(providers[selected].provider, "openai");
    }

    #[tokio::test]
    async fn test_provider_selection_round_robin() {
        let config = FailoverConfig {
            enabled: true,
            strategy: "round-robin".to_string(),
            max_retries: 2,
            providers: vec![
                FailoverProvider {
                    provider: "provider1".to_string(),
                    model: None,
                    base_url: None,
                    priority: 1,
                },
                FailoverProvider {
                    provider: "provider2".to_string(),
                    model: None,
                    base_url: None,
                    priority: 2,
                },
            ],
        };

        let manager = FailoverManager::new(config);
        let providers = manager.providers();

        let idx1 = manager.select_provider().await.unwrap();
        assert_eq!(providers[idx1].provider, "provider1");

        let idx2 = manager.select_provider().await.unwrap();
        assert_eq!(providers[idx2].provider, "provider2");

        let idx3 = manager.select_provider().await.unwrap();
        assert_eq!(providers[idx3].provider, "provider1"); // Wraps around
    }
}

//! Config-driven provider registry using genai as the backend.
//!
//! This module provides a configuration-based approach to provider management,
//! allowing new providers to be added via TOML config without code changes.

use anyhow::{Context, Result};
use genai::chat::{ChatMessage, ChatRequest, ChatResponse, ChatStreamResponse};
use genai::resolver::AuthData;
use genai::{Client, ClientBuilder, ModelIden, ServiceTarget};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::debug;

// ── Configuration Types ─────────────────────────────────────────────────────

/// Authentication method for a provider.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// API key from environment variable.
    #[default]
    ApiKeyEnv,
    /// API key from file.
    ApiKeyFile,
    /// API key inline (not recommended for production).
    ApiKeyInline,
    /// No authentication required.
    None,
}

/// Provider configuration from TOML.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// API type: anthropic, openai, gemini, ollama, groq, xai, deepseek, cohere, openai-compatible
    pub api: String,

    /// Base URL override (required for openai-compatible, optional for others).
    #[serde(default)]
    pub base_url: Option<String>,

    /// Environment variable name for API key.
    #[serde(default)]
    pub api_key_env: Option<String>,

    /// File path containing API key.
    #[serde(default)]
    pub api_key_file: Option<String>,

    /// Inline API key (not recommended).
    #[serde(default)]
    pub api_key: Option<String>,

    /// Default model for this provider.
    #[serde(default)]
    pub default_model: Option<String>,

    /// Display name for UI.
    #[serde(default)]
    pub display_name: Option<String>,

    /// Help URL for getting API keys.
    #[serde(default)]
    pub help_url: Option<String>,
}

impl ProviderConfig {
    /// Resolve the API key from configured source.
    pub fn resolve_api_key(&self) -> Option<String> {
        // Priority: inline > file > env
        if let Some(ref key) = self.api_key {
            return Some(key.clone());
        }

        if let Some(ref path) = self.api_key_file {
            if let Ok(key) = std::fs::read_to_string(path) {
                return Some(key.trim().to_string());
            }
        }

        if let Some(ref env_var) = self.api_key_env {
            if let Ok(key) = std::env::var(env_var) {
                return Some(key);
            }
        }

        None
    }

    /// Get the authentication method being used.
    pub fn auth_method(&self) -> AuthMethod {
        if self.api_key.is_some() {
            AuthMethod::ApiKeyInline
        } else if self.api_key_file.is_some() {
            AuthMethod::ApiKeyFile
        } else if self.api_key_env.is_some() {
            AuthMethod::ApiKeyEnv
        } else {
            AuthMethod::None
        }
    }
}

/// Top-level provider registry configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ProvidersConfig {
    /// Provider configurations keyed by provider ID.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,

    /// Model aliases (e.g., "fast" -> "groq/llama-3.1-70b-versatile").
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
}

// ── Provider Registry ───────────────────────────────────────────────────────

/// A resolved model reference.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub provider_id: String,
    pub model_name: String,
    pub config: ProviderConfig,
}

/// Config-driven provider registry.
pub struct ProviderRegistry {
    config: Arc<ProvidersConfig>,
    client: Client,
}

impl ProviderRegistry {
    /// Create a new registry from configuration.
    pub fn new(config: ProvidersConfig) -> Result<Self> {
        let config = Arc::new(config);

        // Create resolver closures that capture the config
        let auth_config = Arc::clone(&config);
        let target_config = Arc::clone(&config);

        let client = ClientBuilder::default()
            .with_auth_resolver_fn(move |model: ModelIden| -> genai::resolver::Result<Option<AuthData>> {
                let provider_id = model.adapter_kind.as_str();
                if let Some(provider) = auth_config.providers.get(provider_id) {
                    if let Some(api_key) = provider.resolve_api_key() {
                        debug!(provider = %provider_id, "Resolved API key from config");
                        return Ok(Some(AuthData::from_single(api_key)));
                    }
                }
                // Return None to fall back to genai defaults
                Ok(None)
            })
            .with_service_target_resolver_fn(move |mut service_target: ServiceTarget| -> genai::resolver::Result<ServiceTarget> {
                let provider_id = service_target.model.adapter_kind.as_str();
                if let Some(provider) = target_config.providers.get(provider_id) {
                    if let Some(ref base_url) = provider.base_url {
                        debug!(provider = %provider_id, url = %base_url, "Using custom base URL");
                        // Override the endpoint URL
                        service_target.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
                    }
                }
                Ok(service_target)
            })
            .build();

        Ok(Self { config, client })
    }

    /// Resolve a user-facing model reference into the fully-qualified genai
    /// model identifier (`provider/model`) expected by the client.
    fn resolve_full_model(&self, model_ref: &str) -> Result<String> {
        let resolved = self.resolve(model_ref)?;
        Ok(format!("{}/{}", resolved.provider_id, resolved.model_name))
    }

    /// Resolve a model reference to provider + model.
    ///
    /// Formats:
    /// - "provider/model" -> provider, model
    /// - "alias" -> resolved from model_aliases
    /// - "bare-model" -> auto-detect via genai
    ///
    /// Alias chains are followed iteratively; a cycle is detected via a visited
    /// set and reported as an error naming the exact aliases that form the cycle.
    pub fn resolve(&self, model_ref: &str) -> Result<ResolvedModel> {
        let mut current = model_ref;
        // `path` preserves insertion order so we can slice out the exact cycle.
        let mut path: Vec<&str> = Vec::new();
        let mut visited: HashSet<&str> = HashSet::new();

        loop {
            if !visited.insert(current) {
                // Find where in `path` the repeated alias first appeared and
                // extract only that segment as the cycle description.
                let cycle_start = path.iter().position(|&s| s == current).unwrap_or(0);
                let cycle: Vec<&str> = path[cycle_start..].to_vec();
                anyhow::bail!(
                    "Circular alias detected while resolving '{}': {} -> {} (cycle)",
                    model_ref,
                    cycle.join(" -> "),
                    current
                );
            }
            path.push(current);

            // Follow alias if one exists.
            if let Some(next) = self.config.model_aliases.get(current) {
                current = next.as_str();
                continue;
            }

            // Check for provider/model format.
            if let Some((provider_id, model_name)) = current.split_once('/') {
                if let Some(config) = self.config.providers.get(provider_id) {
                    return Ok(ResolvedModel {
                        provider_id: provider_id.to_string(),
                        model_name: model_name.to_string(),
                        config: config.clone(),
                    });
                }
            }

            // No alias and not a provider/model reference.
            anyhow::bail!(
                "Cannot resolve model '{}'. Use 'provider/model' format or define an alias.",
                current
            );
        }
    }

    /// Get the genai client for direct use.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// List all configured providers.
    pub fn providers(&self) -> Vec<&str> {
        self.config.providers.keys().map(|s| s.as_str()).collect()
    }

    /// Get a provider's config by ID.
    pub fn provider_config(&self, id: &str) -> Option<&ProviderConfig> {
        self.config.providers.get(id)
    }

    /// Get the base URL for a provider.
    pub fn base_url(&self, id: &str) -> Option<&str> {
        self.config
            .providers
            .get(id)
            .and_then(|p| p.base_url.as_deref())
    }

    /// Get the environment variable name for a provider's API key.
    pub fn api_key_env(&self, id: &str) -> Option<&str> {
        self.config
            .providers
            .get(id)
            .and_then(|p| p.api_key_env.as_deref())
    }

    /// Get the display name for a provider.
    pub fn display_name(&self, id: &str) -> Option<&str> {
        self.config
            .providers
            .get(id)
            .and_then(|p| p.display_name.as_deref())
    }

    /// Get the default model for a provider.
    pub fn default_model(&self, id: &str) -> Option<&str> {
        self.config
            .providers
            .get(id)
            .and_then(|p| p.default_model.as_deref())
    }

    /// Execute a chat request.
    pub async fn chat(&self, model_ref: &str, messages: Vec<ChatMessage>) -> Result<ChatResponse> {
        self.chat_request(model_ref, ChatRequest::new(messages))
            .await
    }

    /// Execute a native genai chat request.
    ///
    /// Use this when the caller needs genai features such as top-level system
    /// prompts or native tool definitions.
    pub async fn chat_request(
        &self,
        model_ref: &str,
        request: ChatRequest,
    ) -> Result<ChatResponse> {
        let full_model = self.resolve_full_model(model_ref)?;

        self.client
            .exec_chat(&full_model, request, None)
            .await
            .context("Chat request failed")
    }

    /// Execute a streaming chat request.
    pub async fn chat_stream(
        &self,
        model_ref: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<ChatStreamResponse> {
        self.chat_stream_request(model_ref, ChatRequest::new(messages))
            .await
    }

    /// Execute a native genai streaming chat request.
    ///
    /// Use this when the caller needs genai features such as top-level system
    /// prompts or native tool definitions in streaming mode.
    pub async fn chat_stream_request(
        &self,
        model_ref: &str,
        request: ChatRequest,
    ) -> Result<ChatStreamResponse> {
        let full_model = self.resolve_full_model(model_ref)?;

        self.client
            .exec_chat_stream(&full_model, request, None)
            .await
            .context("Chat stream request failed")
    }
}

// ── Built-in Provider Defaults ──────────────────────────────────────────────

/// Get the default configuration for built-in providers.
pub fn builtin_providers() -> HashMap<String, ProviderConfig> {
    let mut providers = HashMap::new();

    providers.insert(
        "anthropic".to_string(),
        ProviderConfig {
            api: "anthropic".to_string(),
            base_url: Some("https://api.anthropic.com".to_string()),
            api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("claude-sonnet-4-20250514".to_string()),
            display_name: Some("Anthropic (Claude)".to_string()),
            help_url: Some("https://console.anthropic.com/settings/keys".to_string()),
        },
    );

    providers.insert(
        "openai".to_string(),
        ProviderConfig {
            api: "openai".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("gpt-4o".to_string()),
            display_name: Some("OpenAI (GPT)".to_string()),
            help_url: Some("https://platform.openai.com/api-keys".to_string()),
        },
    );

    providers.insert(
        "google".to_string(),
        ProviderConfig {
            api: "gemini".to_string(),
            base_url: Some("https://generativelanguage.googleapis.com/v1beta".to_string()),
            api_key_env: Some("GEMINI_API_KEY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("gemini-2.5-pro".to_string()),
            display_name: Some("Google (Gemini)".to_string()),
            help_url: Some("https://aistudio.google.com/apikey".to_string()),
        },
    );

    providers.insert(
        "ollama".to_string(),
        ProviderConfig {
            api: "ollama".to_string(),
            base_url: Some("http://localhost:11434".to_string()),
            api_key_env: None,
            api_key_file: None,
            api_key: None,
            default_model: Some("llama3.1".to_string()),
            display_name: Some("Ollama (local)".to_string()),
            help_url: None,
        },
    );

    providers.insert(
        "groq".to_string(),
        ProviderConfig {
            api: "groq".to_string(),
            base_url: Some("https://api.groq.com/openai/v1".to_string()),
            api_key_env: Some("GROQ_API_KEY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("llama-3.1-70b-versatile".to_string()),
            display_name: Some("Groq".to_string()),
            help_url: Some("https://console.groq.com/keys".to_string()),
        },
    );

    providers.insert(
        "xai".to_string(),
        ProviderConfig {
            api: "xai".to_string(),
            base_url: Some("https://api.x.ai/v1".to_string()),
            api_key_env: Some("XAI_API_KEY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("grok-3".to_string()),
            display_name: Some("xAI (Grok)".to_string()),
            help_url: Some("https://console.x.ai/".to_string()),
        },
    );

    providers.insert(
        "deepseek".to_string(),
        ProviderConfig {
            api: "deepseek".to_string(),
            base_url: Some("https://api.deepseek.com".to_string()),
            api_key_env: Some("DEEPSEEK_API_KEY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("deepseek-chat".to_string()),
            display_name: Some("DeepSeek".to_string()),
            help_url: Some("https://platform.deepseek.com/api_keys".to_string()),
        },
    );

    providers.insert(
        "openrouter".to_string(),
        ProviderConfig {
            api: "openai-compatible".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_key_env: Some("OPENROUTER_API_KEY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("anthropic/claude-sonnet-4-20250514".to_string()),
            display_name: Some("OpenRouter".to_string()),
            help_url: Some("https://openrouter.ai/keys".to_string()),
        },
    );

    // Local inference servers
    providers.insert(
        "lmstudio".to_string(),
        ProviderConfig {
            api: "openai-compatible".to_string(),
            base_url: Some("http://localhost:1234/v1".to_string()),
            api_key_env: None,
            api_key_file: None,
            api_key: None,
            default_model: None,
            display_name: Some("LM Studio (local)".to_string()),
            help_url: Some("https://lmstudio.ai/".to_string()),
        },
    );

    providers.insert(
        "exo".to_string(),
        ProviderConfig {
            api: "openai-compatible".to_string(),
            base_url: Some("http://localhost:52415/v1".to_string()),
            api_key_env: None,
            api_key_file: None,
            api_key: None,
            default_model: None,
            display_name: Some("exo (distributed)".to_string()),
            help_url: Some("https://github.com/exo-explore/exo".to_string()),
        },
    );

    // GitHub Copilot (device flow auth - uses OpenAI-compatible API)
    providers.insert(
        "github-copilot".to_string(),
        ProviderConfig {
            api: "openai-compatible".to_string(),
            base_url: Some("https://api.githubcopilot.com".to_string()),
            api_key_env: Some("GITHUB_COPILOT_TOKEN".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("gpt-4o".to_string()),
            display_name: Some("GitHub Copilot".to_string()),
            help_url: Some("https://github.com/features/copilot".to_string()),
        },
    );

    // Copilot proxy (for Copilot extensions)
    providers.insert(
        "copilot-proxy".to_string(),
        ProviderConfig {
            api: "openai-compatible".to_string(),
            base_url: Some("https://api.githubcopilot.com".to_string()),
            api_key_env: Some("GITHUB_COPILOT_TOKEN".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: Some("gpt-4o".to_string()),
            display_name: Some("Copilot Proxy".to_string()),
            help_url: None,
        },
    );

    // Custom provider (user-defined)
    providers.insert(
        "custom".to_string(),
        ProviderConfig {
            api: "openai-compatible".to_string(),
            base_url: None, // Must be configured by user
            api_key_env: Some("CUSTOM_API_KEY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: None,
            display_name: Some("Custom Provider".to_string()),
            help_url: None,
        },
    );

    providers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_config_resolve_api_key_from_env() {
        // SAFETY: This test runs single-threaded and we clean up after
        unsafe {
            std::env::set_var("TEST_API_KEY_PROVIDER_REGISTRY", "test-key-123");
        }

        let config = ProviderConfig {
            api: "openai".to_string(),
            base_url: None,
            api_key_env: Some("TEST_API_KEY_PROVIDER_REGISTRY".to_string()),
            api_key_file: None,
            api_key: None,
            default_model: None,
            display_name: None,
            help_url: None,
        };

        assert_eq!(config.resolve_api_key(), Some("test-key-123".to_string()));

        // SAFETY: Cleanup
        unsafe {
            std::env::remove_var("TEST_API_KEY_PROVIDER_REGISTRY");
        }
    }

    #[test]
    fn test_resolve_model_with_provider_prefix() {
        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                api: "anthropic".to_string(),
                base_url: None,
                api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
                api_key_file: None,
                api_key: None,
                default_model: None,
                display_name: None,
                help_url: None,
            },
        );

        let config = ProvidersConfig {
            providers,
            model_aliases: HashMap::new(),
        };

        let registry = ProviderRegistry::new(config).unwrap();
        let resolved = registry.resolve("anthropic/claude-sonnet-4").unwrap();

        assert_eq!(resolved.provider_id, "anthropic");
        assert_eq!(resolved.model_name, "claude-sonnet-4");
    }

    #[test]
    fn test_resolve_model_alias() {
        let mut providers = HashMap::new();
        providers.insert(
            "groq".to_string(),
            ProviderConfig {
                api: "groq".to_string(),
                base_url: None,
                api_key_env: Some("GROQ_API_KEY".to_string()),
                api_key_file: None,
                api_key: None,
                default_model: None,
                display_name: None,
                help_url: None,
            },
        );

        let mut aliases = HashMap::new();
        aliases.insert(
            "fast".to_string(),
            "groq/llama-3.1-70b-versatile".to_string(),
        );

        let config = ProvidersConfig {
            providers,
            model_aliases: aliases,
        };

        let registry = ProviderRegistry::new(config).unwrap();
        let resolved = registry.resolve("fast").unwrap();

        assert_eq!(resolved.provider_id, "groq");
        assert_eq!(resolved.model_name, "llama-3.1-70b-versatile");
    }

    #[test]
    fn test_builtin_providers() {
        let providers = builtin_providers();

        assert!(providers.contains_key("anthropic"));
        assert!(providers.contains_key("openai"));
        assert!(providers.contains_key("google"));
        assert!(providers.contains_key("ollama"));
        assert!(providers.contains_key("groq"));

        let anthropic = &providers["anthropic"];
        assert_eq!(anthropic.api, "anthropic");
        assert_eq!(anthropic.api_key_env, Some("ANTHROPIC_API_KEY".to_string()));
    }
}

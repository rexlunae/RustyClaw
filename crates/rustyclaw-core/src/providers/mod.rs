//! Shared provider catalogue.
//!
//! Single source of truth for supported providers, their secret key names,
//! base URLs, and available models.  Used by both the onboarding wizard and
//! the TUI `/provider` + `/model` commands.

/// Wrap any `std::error::Error + Send + Sync + 'static` into an
/// `anyhow_tracing::Error`.  Spelled out as a free function rather
/// than relying on a blanket impl because anyhow_tracing only provides
/// `From<anyhow::Error>`, not the wider blanket `From<E: StdError>`,
/// so each call site would otherwise need
/// `anyhow_tracing::Error::from(anyhow::Error::from(e))`.
pub(crate) fn wrap_err<E>(e: E) -> anyhow_tracing::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    anyhow_tracing::Error::from(anyhow::Error::from(e))
}

use anyhow_tracing::{Context, Result, anyhow, bail};

use crate::error_details::RequestDetails;

/// Authentication method for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// API key-based authentication (Bearer token).
    ApiKey,
    /// OAuth 2.0 device flow authentication.
    DeviceFlow,
    /// No authentication required.
    None,
    /// API key is optional (e.g. Ollama: local needs no key, cloud does).
    OptionalApiKey,
}

/// Device flow configuration for OAuth providers.
pub struct DeviceFlowConfig {
    /// OAuth client ID for the application.
    pub client_id: &'static str,
    /// Device authorization endpoint URL.
    pub device_auth_url: &'static str,
    /// Token endpoint URL.
    pub token_url: &'static str,
    /// Optional scope to request.
    pub scope: Option<&'static str>,
}

/// A provider definition with its secret key name and available models.
pub struct ProviderDef {
    pub id: &'static str,
    pub display: &'static str,
    /// Authentication method for this provider.
    pub auth_method: AuthMethod,
    /// Name of the secret that holds the API key or access token.
    /// For API key auth: e.g. `"ANTHROPIC_API_KEY"`.
    /// For device flow: e.g. `"GITHUB_COPILOT_TOKEN"`.
    /// `None` means the provider does not require authentication (e.g. Ollama).
    pub secret_key: Option<&'static str>,
    /// Device flow configuration (only used when auth_method is DeviceFlow).
    pub device_flow: Option<&'static DeviceFlowConfig>,
    pub base_url: Option<&'static str>,
    pub models: &'static [&'static str],
    /// URL where the user can sign up or get an API key.
    pub help_url: Option<&'static str>,
    /// Short hint shown in the API key dialog (e.g. "Get one at …").
    pub help_text: Option<&'static str>,
}

// GitHub Copilot device flow configuration.
// This uses the official GitHub Copilot CLI client ID which is publicly documented
// at https://docs.github.com/en/copilot/using-github-copilot/using-github-copilot-in-the-cli
pub const GITHUB_COPILOT_DEVICE_FLOW: DeviceFlowConfig = DeviceFlowConfig {
    client_id: "Iv1.b507a08c87ecfe98", // GitHub Copilot CLI client ID
    device_auth_url: "https://github.com/login/device/code",
    token_url: "https://github.com/login/oauth/access_token",
    scope: None, // OAuth App's default scopes include Copilot access
};

pub const PROVIDERS: &[ProviderDef] = &[
    ProviderDef {
        id: "anthropic",
        display: "Anthropic (Claude)",
        auth_method: AuthMethod::ApiKey,
        secret_key: Some("ANTHROPIC_API_KEY"),
        device_flow: None,
        base_url: Some("https://api.anthropic.com"),
        models: &[
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
            "claude-haiku-4-20250514",
        ],
        help_url: Some("https://console.anthropic.com/settings/keys"),
        help_text: Some("Get a key at console.anthropic.com → API Keys"),
    },
    ProviderDef {
        id: "openai",
        display: "OpenAI (GPT / o-series)",
        auth_method: AuthMethod::ApiKey,
        secret_key: Some("OPENAI_API_KEY"),
        device_flow: None,
        base_url: Some("https://api.openai.com/v1"),
        models: &["gpt-4.1", "gpt-4.1-mini", "gpt-4.1-nano", "o3", "o4-mini"],
        help_url: Some("https://platform.openai.com/api-keys"),
        help_text: Some("Get a key at platform.openai.com → API Keys"),
    },
    ProviderDef {
        id: "google",
        display: "Google (Gemini)",
        auth_method: AuthMethod::ApiKey,
        secret_key: Some("GEMINI_API_KEY"),
        device_flow: None,
        base_url: Some("https://generativelanguage.googleapis.com/v1beta"),
        models: &["gemini-2.5-pro", "gemini-2.5-flash", "gemini-2.0-flash"],
        help_url: Some("https://aistudio.google.com/apikey"),
        help_text: Some("Get a key at aistudio.google.com → API Key"),
    },
    ProviderDef {
        id: "xai",
        display: "xAI (Grok)",
        auth_method: AuthMethod::ApiKey,
        secret_key: Some("XAI_API_KEY"),
        device_flow: None,
        base_url: Some("https://api.x.ai/v1"),
        models: &["grok-3", "grok-3-mini"],
        help_url: Some("https://console.x.ai/"),
        help_text: Some("Get a key at console.x.ai"),
    },
    ProviderDef {
        id: "openrouter",
        display: "OpenRouter",
        auth_method: AuthMethod::ApiKey,
        secret_key: Some("OPENROUTER_API_KEY"),
        device_flow: None,
        base_url: Some("https://openrouter.ai/api/v1"),
        // Popular models — OpenRouter has 300+ models; use /model fetch or
        // the dynamic fetch_models() API for a complete list.
        models: &[
            // Anthropic
            "anthropic/claude-opus-4-20250514",
            "anthropic/claude-sonnet-4-20250514",
            "anthropic/claude-haiku-4-20250514",
            "anthropic/claude-3.5-sonnet",
            "anthropic/claude-3.5-haiku",
            // OpenAI
            "openai/gpt-4.1",
            "openai/gpt-4.1-mini",
            "openai/gpt-4.1-nano",
            "openai/o3",
            "openai/o4-mini",
            "openai/gpt-4o",
            "openai/gpt-4o-mini",
            // Google
            "google/gemini-2.5-pro",
            "google/gemini-2.5-flash",
            "google/gemini-2.0-flash",
            // Meta
            "meta-llama/llama-4-maverick",
            "meta-llama/llama-4-scout",
            "meta-llama/llama-3.3-70b-instruct",
            // Mistral
            "mistralai/mistral-large",
            "mistralai/mistral-small",
            "mistralai/codestral",
            // DeepSeek
            "deepseek/deepseek-chat-v3",
            "deepseek/deepseek-r1",
            // xAI
            "x-ai/grok-3",
            "x-ai/grok-3-mini",
            // Qwen
            "qwen/qwen3-coder",
            "qwen/qwen-2.5-72b-instruct",
        ],
        help_url: Some("https://openrouter.ai/keys"),
        help_text: Some("Get a key at openrouter.ai/keys (free tier available)"),
    },
    ProviderDef {
        id: "github-copilot",
        display: "GitHub Copilot",
        auth_method: AuthMethod::DeviceFlow,
        secret_key: Some("GITHUB_COPILOT_TOKEN"),
        device_flow: Some(&GITHUB_COPILOT_DEVICE_FLOW),
        base_url: Some("https://api.githubcopilot.com"),
        // Keep in sync with COPILOT_STATIC_CATALOG in providers/models.rs.
        models: &[
            "claude-fable-5",
            "claude-haiku-4.5",
            "claude-opus-4.5",
            "claude-opus-4.7",
            "claude-opus-4.8",
            "claude-sonnet-4.5",
            "claude-sonnet-4.6",
            "gemini-2.5-pro",
            "gemini-3-flash-preview",
            "gemini-3.1-pro-preview",
            "gemini-3.5-flash",
            "gpt-4.1",
            "gpt-4o",
            "gpt-5-mini",
            "gpt-5.2",
            "gpt-5.3-codex",
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.5",
        ],
        help_url: None,
        help_text: Some("Uses GitHub device flow — no manual key needed"),
    },
    ProviderDef {
        id: "copilot-proxy",
        display: "Copilot Proxy",
        auth_method: AuthMethod::DeviceFlow,
        secret_key: Some("COPILOT_PROXY_TOKEN"),
        device_flow: Some(&GITHUB_COPILOT_DEVICE_FLOW),
        base_url: None, // will prompt for proxy URL
        models: &[],
        help_url: None,
        help_text: None,
    },
    ProviderDef {
        id: "ollama",
        display: "Ollama (local or cloud)",
        auth_method: AuthMethod::OptionalApiKey,
        secret_key: Some("OLLAMA_API_KEY"),
        device_flow: None,
        base_url: Some("http://localhost:11434/v1"),
        models: &["llama3.1", "mistral", "codellama", "deepseek-coder"],
        help_url: None,
        help_text: Some("No key needed for local Ollama. For Ollama Cloud set OLLAMA_API_KEY."),
    },
    ProviderDef {
        id: "lmstudio",
        display: "LM Studio (local)",
        auth_method: AuthMethod::None,
        secret_key: None,
        device_flow: None,
        base_url: Some("http://localhost:1234/v1"),
        models: &[],
        help_url: None,
        help_text: Some("No key needed — runs locally. Default port 1234. Install: lmstudio.ai"),
    },
    ProviderDef {
        id: "exo",
        display: "exo cluster (local)",
        auth_method: AuthMethod::None,
        secret_key: None,
        device_flow: None,
        base_url: Some("http://localhost:52415/v1"),
        models: &[],
        help_url: None,
        help_text: Some(
            "No key needed — exo cluster. Default port 52415. Install: github.com/exo-explore/exo",
        ),
    },
    ProviderDef {
        id: "opencode",
        display: "OpenCode Zen",
        auth_method: AuthMethod::ApiKey,
        secret_key: Some("OPENCODE_API_KEY"),
        device_flow: None,
        // OpenAI-compatible chat/completions endpoint for most models.
        // Claude models also work here via OpenCode's OpenAI-compatible layer.
        base_url: Some("https://opencode.ai/zen/v1"),
        models: &[
            // Free models
            "big-pickle",
            "minimax-m2.5-free",
            "kimi-k2.5-free",
            // Claude models (via OpenAI-compatible API)
            "claude-opus-4-6",
            "claude-opus-4-5",
            "claude-sonnet-4-5",
            "claude-sonnet-4",
            "claude-haiku-4-5",
            "claude-3-5-haiku",
            // GPT models
            "gpt-5.2",
            "gpt-5.2-codex",
            "gpt-5.1",
            "gpt-5.1-codex",
            "gpt-5.1-codex-max",
            "gpt-5.1-codex-mini",
            "gpt-5",
            "gpt-5-codex",
            "gpt-5-nano",
            // Gemini models
            "gemini-3-pro",
            "gemini-3-flash",
            // Other models
            "minimax-m2.5",
            "minimax-m2.1",
            "glm-5",
            "glm-4.7",
            "glm-4.6",
            "kimi-k2.5",
            "kimi-k2-thinking",
            "kimi-k2",
            "qwen3-coder",
        ],
        help_url: Some("https://opencode.ai/auth"),
        help_text: Some(
            "Get a key at opencode.ai/auth — includes free models (Big Pickle, MiniMax, Kimi)",
        ),
    },
    ProviderDef {
        id: "custom",
        display: "Custom / OpenAI-compatible endpoint",
        auth_method: AuthMethod::ApiKey,
        secret_key: Some("CUSTOM_API_KEY"),
        device_flow: None,
        base_url: None, // will prompt
        models: &[],
        help_url: None,
        help_text: Some("Enter the API key for your custom endpoint"),
    },
];

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Look up a provider by ID.
pub fn provider_by_id(id: &str) -> Option<&'static ProviderDef> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Return the secret-key name for the given provider ID, or `None` if the
/// provider doesn't require one (e.g. Ollama).
pub fn secret_key_for_provider(id: &str) -> Option<&'static str> {
    provider_by_id(id).and_then(|p| p.secret_key)
}

/// Return the display name for the given provider ID.
pub fn display_name_for_provider(id: &str) -> &str {
    provider_by_id(id).map(|p| p.display).unwrap_or(id)
}

/// Return all provider IDs.
pub fn provider_ids() -> Vec<&'static str> {
    PROVIDERS.iter().map(|p| p.id).collect()
}

/// Return all model names across all providers (for tab-completion).
pub fn all_model_names() -> Vec<&'static str> {
    PROVIDERS
        .iter()
        .flat_map(|p| p.models.iter().copied())
        .collect()
}

/// Return the models for the given provider ID.
pub fn models_for_provider(id: &str) -> &'static [&'static str] {
    provider_by_id(id).map(|p| p.models).unwrap_or(&[])
}

/// Return the base URL for the given provider ID.
pub fn base_url_for_provider(id: &str) -> Option<&'static str> {
    provider_by_id(id).and_then(|p| p.base_url)
}

// ── Dynamic model fetching ──────────────────────────────────────────────────

/// Rich model metadata returned by [`fetch_models_detailed`].
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Provider-specific model ID (e.g. `anthropic/claude-opus-4-20250514`).
    pub id: String,
    /// Human-readable name (if available from the API).
    pub name: Option<String>,
    /// Context window size in tokens (if available).
    pub context_length: Option<u64>,
    /// Price per prompt/input token in USD (if available).
    pub pricing_prompt: Option<f64>,
    /// Price per completion/output token in USD (if available).
    pub pricing_completion: Option<f64>,
}

impl ModelInfo {
    /// Format a one-line summary suitable for display in the TUI.
    pub fn display_line(&self) -> String {
        let mut parts = vec![self.id.clone()];
        if let Some(ref name) = self.name {
            if name != &self.id {
                parts.push(format!("({})", name));
            }
        }
        if let Some(ctx) = self.context_length {
            parts.push(format!("{}k ctx", ctx / 1000));
        }
        if let (Some(p), Some(c)) = (self.pricing_prompt, self.pricing_completion) {
            // Show price per million tokens for readability
            let p_m = p * 1_000_000.0;
            let c_m = c * 1_000_000.0;
            parts.push(format!("${:.2}/${:.2} per 1M tok", p_m, c_m));
        }
        parts.join(" · ")
    }
}

/// Fetch the list of available models from a provider's API.
///
/// Returns `Err` with a human-readable message on any failure — no silent
/// fallbacks.  Callers should display the error to the user.
mod device_flow;
mod models;
pub use device_flow::*;
pub use models::*;

#[cfg(test)]
mod tests;

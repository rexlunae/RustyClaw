//! Shared provider catalogue.
//!
//! Single source of truth for supported providers, their secret key names,
//! base URLs, and available models.  Used by both the onboarding wizard and
//! the TUI `/provider` + `/model` commands.

/// Authentication method for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// API key-based authentication (Bearer token).
    ApiKey,
    /// OAuth 2.0 device flow authentication.
    DeviceFlow,
    /// No authentication required.
    None,
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
    scope: Some("read:user"),
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
        models: &[
            "gpt-4.1",
            "gpt-4.1-mini",
            "o3",
            "o4-mini",
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
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
        display: "Ollama (local)",
        auth_method: AuthMethod::None,
        secret_key: None,
        device_flow: None,
        base_url: Some("http://localhost:11434/v1"),
        models: &["llama3.1", "mistral", "codellama", "deepseek-coder"],
        help_url: None,
        help_text: Some("No key needed — runs locally. Install: ollama.com"),
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
pub async fn fetch_models(
    provider_id: &str,
    api_key: Option<&str>,
    base_url_override: Option<&str>,
) -> Result<Vec<String>, String> {
    // Delegate to the detailed version and strip down to IDs.
    fetch_models_detailed(provider_id, api_key, base_url_override)
        .await
        .map(|v| v.into_iter().map(|m| m.id).collect())
}

/// Fetch models with full metadata (pricing, context length, name).
///
/// Providers that don't expose rich metadata will still return [`ModelInfo`]
/// entries — just with `None` for the optional fields.
pub async fn fetch_models_detailed(
    provider_id: &str,
    api_key: Option<&str>,
    base_url_override: Option<&str>,
) -> Result<Vec<ModelInfo>, String> {
    let def = match provider_by_id(provider_id) {
        Some(d) => d,
        None => return Err(format!("Unknown provider: {}", provider_id)),
    };

    let base = base_url_override.or(def.base_url).unwrap_or("");

    if base.is_empty() {
        return Err(format!(
            "No base URL configured for {}. Set one in config.toml or use /provider.",
            def.display,
        ));
    }

    // Anthropic has no public models endpoint — return the static list.
    if provider_id == "anthropic" {
        let static_models: Vec<ModelInfo> = def
            .models
            .iter()
            .map(|id| ModelInfo {
                id: id.to_string(),
                name: None,
                context_length: None,
                pricing_prompt: None,
                pricing_completion: None,
            })
            .collect();
        return Ok(static_models);
    }

    let result = match provider_id {
        // Google Gemini uses a different response shape
        "google" => fetch_google_models_detailed(base, api_key).await,
        // Local providers — no auth needed, OpenAI-compatible /v1/models
        "ollama" | "lmstudio" | "exo" => fetch_openai_compatible_models_detailed(base, None).await,
        // Everything else is OpenAI-compatible
        _ => fetch_openai_compatible_models_detailed(base, api_key).await,
    };

    match result {
        Ok(models) if models.is_empty() => Err(format!(
            "The {} API returned an empty model list.",
            def.display,
        )),
        Ok(models) => Ok(models),
        Err(e) => Err(format!(
            "Failed to fetch models from {}: {}",
            def.display, e
        )),
    }
}

/// Non-chat model ID patterns.  Any model whose ID contains one of these
/// substrings (case-insensitive) is filtered out of the selector.
const NON_CHAT_PATTERNS: &[&str] = &[
    "embed",
    "tts",
    "whisper",
    "dall-e",
    "davinci",
    "babbage",
    "moderation",
    "search",
    "similarity",
    "code-search",
    "text-search",
    "audio",
    "realtime",
    "transcri",
    "computer-use",
    "canary", // internal/experimental
];

/// Check whether a model entry looks like it supports chat completions.
///
/// 1. If the entry has `capabilities.chat` (GitHub Copilot style),
///    use that.
/// 2. Otherwise fall back to filtering out known non-chat ID patterns.
fn is_chat_model(entry: &serde_json::Value) -> bool {
    // GitHub Copilot and some providers expose capabilities metadata.
    if let Some(caps) = entry.get("capabilities") {
        return caps
            .get("chat")
            .or_else(|| caps.get("type").filter(|v| v.as_str() == Some("chat")))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    }

    // Some endpoints use object type "model" vs "embedding" etc.
    if let Some(obj) = entry.get("object").and_then(|v| v.as_str()) {
        if obj != "model" {
            return false;
        }
    }

    // Fall back to ID pattern matching.
    let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let lower = id.to_lowercase();
    !NON_CHAT_PATTERNS.iter().any(|pat| lower.contains(pat))
}

/// Fetch from an OpenAI-compatible `/models` endpoint with full metadata.
///
/// Works for OpenAI, xAI, OpenRouter, Ollama, GitHub Copilot, and
/// custom providers.  Only models that appear to support chat
/// completions are returned (see [`is_chat_model`]).
async fn fetch_openai_compatible_models_detailed(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>, reqwest::Error> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut req = client.get(&url);
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }

    let resp = req.send().await?.error_for_status()?;
    let body: serde_json::Value = resp.json().await?;

    let mut models: Vec<ModelInfo> = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|m| is_chat_model(m))
                .filter_map(|m| {
                    let id = m.get("id").and_then(|v| v.as_str())?.to_string();
                    let name = m.get("name").and_then(|v| v.as_str()).map(String::from);
                    let context_length = m.get("context_length").and_then(|v| v.as_u64());
                    // OpenRouter-style pricing: { "prompt": "0.000015", "completion": "0.000075" }
                    let pricing_prompt =
                        m.get("pricing")
                            .and_then(|p| p.get("prompt"))
                            .and_then(|v| {
                                v.as_str()
                                    .and_then(|s| s.parse::<f64>().ok())
                                    .or_else(|| v.as_f64())
                            });
                    let pricing_completion = m
                        .get("pricing")
                        .and_then(|p| p.get("completion"))
                        .and_then(|v| {
                            v.as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .or_else(|| v.as_f64())
                        });
                    Some(ModelInfo {
                        id,
                        name,
                        context_length,
                        pricing_prompt,
                        pricing_completion,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

/// Fetch from the Google Gemini `/models` endpoint with metadata.
async fn fetch_google_models_detailed(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>, reqwest::Error> {
    let key = match api_key {
        Some(k) => k,
        // No key — return empty so the outer match produces a clear error
        None => return Ok(Vec::new()),
    };

    let url = format!("{}/models?key={}", base_url.trim_end_matches('/'), key);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let resp = client.get(&url).send().await?.error_for_status()?;
    let body: serde_json::Value = resp.json().await?;

    let models = body
        .get("models")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let raw_name = m.get("name").and_then(|v| v.as_str())?;
                    let id = raw_name
                        .strip_prefix("models/")
                        .unwrap_or(raw_name)
                        .to_string();
                    let display_name = m
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    // Google returns inputTokenLimit / outputTokenLimit
                    let context_length = m.get("inputTokenLimit").and_then(|v| v.as_u64());
                    Some(ModelInfo {
                        id,
                        name: display_name,
                        context_length,
                        pricing_prompt: None,
                        pricing_completion: None,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(models)
}

// ── OAuth Device Flow ───────────────────────────────────────────────────────

use serde::Deserialize;

/// Response from the device authorization endpoint.
#[derive(Debug, Deserialize)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Response from the token endpoint.
///
/// Uses a flat struct with all-optional fields for robust deserialization.
/// GitHub's token endpoint returns either a success object (with
/// `access_token`) or an error object (with `error`), but
/// `#[serde(untagged)]` enums are fragile and silently fail when the
/// response shape differs even slightly from what's expected.
#[derive(Debug, Deserialize)]
pub struct RawTokenResponse {
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    pub token_type: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// Interpreted token response for pattern-matching callers.
#[derive(Debug)]
pub enum TokenResponse {
    Success {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        token_type: String,
    },
    Pending {
        error: String,
        error_description: Option<String>,
    },
}

impl From<RawTokenResponse> for TokenResponse {
    fn from(raw: RawTokenResponse) -> Self {
        if let Some(access_token) = raw.access_token {
            TokenResponse::Success {
                access_token,
                token_type: raw.token_type.unwrap_or_else(|| "bearer".to_string()),
                refresh_token: raw.refresh_token,
                expires_in: raw.expires_in,
            }
        } else {
            TokenResponse::Pending {
                error: raw.error.unwrap_or_else(|| "unknown".to_string()),
                error_description: raw.error_description,
            }
        }
    }
}

/// Initiate OAuth device flow and return device code and verification URL.
pub async fn start_device_flow(config: &DeviceFlowConfig) -> Result<DeviceAuthResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let params = [
        ("client_id", config.client_id),
        ("scope", config.scope.unwrap_or("")),
    ];

    let resp = client
        .post(config.device_auth_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to request device code: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Device authorization failed: {}", e))?;

    let auth_response: DeviceAuthResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse device authorization response: {}", e))?;

    Ok(auth_response)
}

/// Poll the token endpoint to complete device flow authentication.
///
/// Returns Ok(Some(token)) when authentication succeeds,
/// Ok(None) when still pending, and Err when authentication fails.
pub async fn poll_device_token(
    config: &DeviceFlowConfig,
    device_code: &str,
) -> Result<Option<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let params = [
        ("client_id", config.client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ];

    let resp = client
        .post(config.token_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to poll token endpoint: {}", e))?;

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    tracing::debug!("Device flow token poll response: {}", body);

    // Parse as a flat struct first, then interpret.  This avoids the
    // fragility of serde(untagged) which silently fails when the
    // response shape is slightly unexpected.
    let raw: RawTokenResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse token response ({}): body={}", e, body))?;
    let token_response: TokenResponse = raw.into();

    match token_response {
        TokenResponse::Success { access_token, .. } => {
            tracing::info!("Device flow authentication succeeded");
            Ok(Some(access_token))
        }
        TokenResponse::Pending { error, .. } => {
            if error == "authorization_pending" || error == "slow_down" {
                tracing::trace!("Device flow still pending: {}", error);
                Ok(None) // Still waiting for user authorization
            } else {
                Err(format!("Authentication failed: {}", error))
            }
        }
    }
}

// ── Copilot session token exchange ──────────────────────────────────────────

/// Response from the Copilot internal token endpoint.
///
/// The `token` field is a short-lived session token (valid ~30 min).
/// `expires_at` is a Unix timestamp indicating when it expires.
#[derive(Debug, Deserialize)]
pub struct CopilotSessionResponse {
    pub token: String,
    pub expires_at: i64,
}

/// Exchange a GitHub OAuth token for a short-lived Copilot API session token.
///
/// The Copilot chat API (`api.githubcopilot.com`) requires a session token
/// obtained by presenting the long-lived OAuth device-flow token to
/// GitHub's internal token endpoint.  Session tokens expire after ~30
/// minutes; the caller should cache and refresh before `expires_at`.
pub async fn exchange_copilot_session(
    http: &reqwest::Client,
    oauth_token: &str,
) -> Result<CopilotSessionResponse, String> {
    let resp = http
        .get("https://api.github.com/copilot_internal/v2/token")
        .header("Authorization", format!("token {}", oauth_token))
        .header("User-Agent", "RustyClaw")
        .send()
        .await
        .map_err(|e| format!("Failed to exchange Copilot token: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Copilot token exchange returned {} — {}",
            status, body,
        ));
    }

    resp.json::<CopilotSessionResponse>()
        .await
        .map_err(|e| format!("Failed to parse Copilot session response: {}", e))
}

/// Whether the given provider requires Copilot session-token exchange.
pub fn needs_copilot_session(provider_id: &str) -> bool {
    matches!(provider_id, "github-copilot" | "copilot-proxy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_by_id() {
        let provider = provider_by_id("anthropic");
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().display, "Anthropic (Claude)");

        let provider = provider_by_id("github-copilot");
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().display, "GitHub Copilot");
        assert_eq!(provider.unwrap().auth_method, AuthMethod::DeviceFlow);

        let provider = provider_by_id("nonexistent");
        assert!(provider.is_none());
    }

    #[test]
    fn test_provider_auth_methods() {
        // API key providers
        let anthropic = provider_by_id("anthropic").unwrap();
        assert_eq!(anthropic.auth_method, AuthMethod::ApiKey);
        assert!(anthropic.device_flow.is_none());

        // Device flow providers
        let copilot = provider_by_id("github-copilot").unwrap();
        assert_eq!(copilot.auth_method, AuthMethod::DeviceFlow);
        assert!(copilot.device_flow.is_some());

        let copilot_proxy = provider_by_id("copilot-proxy").unwrap();
        assert_eq!(copilot_proxy.auth_method, AuthMethod::DeviceFlow);
        assert!(copilot_proxy.device_flow.is_some());

        // No auth providers
        let ollama = provider_by_id("ollama").unwrap();
        assert_eq!(ollama.auth_method, AuthMethod::None);
        assert!(ollama.secret_key.is_none());
    }

    #[test]
    fn test_github_copilot_provider_config() {
        let provider = provider_by_id("github-copilot").unwrap();
        assert_eq!(provider.id, "github-copilot");
        assert_eq!(provider.secret_key, Some("GITHUB_COPILOT_TOKEN"));

        let device_config = provider.device_flow.unwrap();
        assert_eq!(
            device_config.device_auth_url,
            "https://github.com/login/device/code"
        );
        assert_eq!(
            device_config.token_url,
            "https://github.com/login/oauth/access_token"
        );
        assert!(!device_config.client_id.is_empty());
    }

    #[test]
    fn test_copilot_proxy_provider_config() {
        let provider = provider_by_id("copilot-proxy").unwrap();
        assert_eq!(provider.id, "copilot-proxy");
        assert_eq!(provider.secret_key, Some("COPILOT_PROXY_TOKEN"));
        assert_eq!(provider.base_url, None); // Should prompt for URL

        let device_config = provider.device_flow.unwrap();
        // Should use same device flow as github-copilot
        assert_eq!(
            device_config.device_auth_url,
            "https://github.com/login/device/code"
        );
    }

    #[test]
    fn test_token_response_parsing() {
        // Test successful token response
        let json = r#"{"access_token":"test_token","token_type":"bearer"}"#;
        let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
        let response: TokenResponse = raw.into();
        match response {
            TokenResponse::Success { access_token, .. } => {
                assert_eq!(access_token, "test_token");
            }
            _ => panic!("Expected Success variant"),
        }

        // Test pending response
        let json = r#"{"error":"authorization_pending"}"#;
        let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
        let response: TokenResponse = raw.into();
        match response {
            TokenResponse::Pending { error, .. } => {
                assert_eq!(error, "authorization_pending");
            }
            _ => panic!("Expected Pending variant"),
        }

        // Test success response with extra fields (e.g. scope)
        let json = r#"{"access_token":"gho_xxx","token_type":"bearer","scope":"read:user"}"#;
        let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
        let response: TokenResponse = raw.into();
        match response {
            TokenResponse::Success { access_token, .. } => {
                assert_eq!(access_token, "gho_xxx");
            }
            _ => panic!("Expected Success variant"),
        }

        // Test success response even if token_type is missing
        let json = r#"{"access_token":"gho_xxx"}"#;
        let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
        let response: TokenResponse = raw.into();
        match response {
            TokenResponse::Success {
                access_token,
                token_type,
                ..
            } => {
                assert_eq!(access_token, "gho_xxx");
                assert_eq!(token_type, "bearer"); // defaults to "bearer"
            }
            _ => panic!("Expected Success variant"),
        }

        // Test error response with description
        let json = r#"{"error":"access_denied","error_description":"user denied"}"#;
        let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
        let response: TokenResponse = raw.into();
        match response {
            TokenResponse::Pending {
                error,
                error_description,
            } => {
                assert_eq!(error, "access_denied");
                assert_eq!(error_description, Some("user denied".to_string()));
            }
            _ => panic!("Expected Pending variant"),
        }
    }

    #[test]
    fn test_all_providers_have_valid_config() {
        for provider in PROVIDERS {
            // Verify basic fields are set
            assert!(!provider.id.is_empty());
            assert!(!provider.display.is_empty());

            // Verify auth consistency
            match provider.auth_method {
                AuthMethod::ApiKey => {
                    assert!(
                        provider.secret_key.is_some(),
                        "Provider {} with ApiKey auth must have secret_key",
                        provider.id
                    );
                    assert!(
                        provider.device_flow.is_none(),
                        "Provider {} with ApiKey auth should not have device_flow",
                        provider.id
                    );
                }
                AuthMethod::DeviceFlow => {
                    assert!(
                        provider.secret_key.is_some(),
                        "Provider {} with DeviceFlow auth must have secret_key",
                        provider.id
                    );
                    assert!(
                        provider.device_flow.is_some(),
                        "Provider {} with DeviceFlow auth must have device_flow config",
                        provider.id
                    );
                }
                AuthMethod::None => {
                    assert!(
                        provider.secret_key.is_none(),
                        "Provider {} with None auth should not have secret_key",
                        provider.id
                    );
                    assert!(
                        provider.device_flow.is_none(),
                        "Provider {} with None auth should not have device_flow",
                        provider.id
                    );
                }
            }
        }
    }

    #[test]
    fn test_needs_copilot_session() {
        assert!(needs_copilot_session("github-copilot"));
        assert!(needs_copilot_session("copilot-proxy"));
        assert!(!needs_copilot_session("openai"));
        assert!(!needs_copilot_session("anthropic"));
        assert!(!needs_copilot_session("google"));
        assert!(!needs_copilot_session("ollama"));
        assert!(!needs_copilot_session("custom"));
    }

    #[test]
    fn test_copilot_session_response_parsing() {
        let json = r#"{"token":"tid=abc123;exp=9999999999","expires_at":1750000000}"#;
        let resp: CopilotSessionResponse = serde_json::from_str(json).unwrap();
        assert!(resp.token.starts_with("tid="));
        assert_eq!(resp.expires_at, 1750000000);
    }
}

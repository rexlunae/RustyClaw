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
        id: "deepseek",
        display: "DeepSeek",
        auth_method: AuthMethod::ApiKey,
        secret_key: Some("DEEPSEEK_API_KEY"),
        device_flow: None,
        base_url: Some("https://api.deepseek.com/v1"),
        // Fallback only — the live list comes from GET /models. DeepSeek
        // retired the old `deepseek-chat` / `deepseek-reasoner` aliases.
        models: &["deepseek-v4-pro", "deepseek-v4-flash"],
        help_url: Some("https://platform.deepseek.com/api_keys"),
        help_text: Some("Get a key at platform.deepseek.com → API Keys"),
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
        id: "llamacpp",
        display: "llama.cpp (local)",
        auth_method: AuthMethod::None,
        secret_key: None,
        device_flow: None,
        base_url: Some("http://localhost:8080/v1"),
        models: &[],
        help_url: Some("https://github.com/ggml-org/llama.cpp"),
        help_text: Some(
            "No key needed — local llama-server. Default port 8080. Serves GGUF models.",
        ),
    },
    ProviderDef {
        id: "joshua",
        display: "Joshua (local)",
        auth_method: AuthMethod::None,
        secret_key: None,
        device_flow: None,
        base_url: Some("http://localhost:8331/v1"),
        models: &[],
        help_url: Some("https://github.com/rexlunae/joshua"),
        help_text: Some(
            "No key needed — pure-Rust local inference. Default port 8331. Serves GGUF models; \
             manage it via /engines.",
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
//
// All lookups cover both the built-in catalogue and any user-registered
// custom providers (see [`custom::set_custom_providers`]).

/// Return every known provider: built-ins followed by custom providers.
pub fn all_providers() -> Vec<&'static ProviderDef> {
    let mut all: Vec<&'static ProviderDef> = PROVIDERS.iter().collect();
    all.extend(custom::custom_provider_defs());
    all
}

/// Look up a provider by ID.
pub fn provider_by_id(id: &str) -> Option<&'static ProviderDef> {
    PROVIDERS
        .iter()
        .find(|p| p.id == id)
        .or_else(|| custom::custom_provider_by_id(id))
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
    all_providers().into_iter().map(|p| p.id).collect()
}

/// Return all model names across all providers (for tab-completion).
pub fn all_model_names() -> Vec<&'static str> {
    all_providers()
        .into_iter()
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

/// Decide which config `base_url` override to keep when switching providers.
///
/// A stored override only makes sense for the provider it was entered for
/// (e.g. the `custom` or `copilot-proxy` prompt).  When switching to a
/// different provider that has a catalogue base URL, drop the override so
/// the catalogue URL wins instead of a stale value from the previous
/// provider.
pub fn base_url_override_for_switch(
    new_provider: &str,
    prev_provider: Option<&str>,
    prev_base_url: Option<String>,
) -> Option<String> {
    if prev_provider == Some(new_provider) {
        return prev_base_url;
    }
    if base_url_for_provider(new_provider).is_none() {
        // Provider needs a manually-entered URL — keep whatever we had.
        return prev_base_url;
    }
    None
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
mod custom;
mod device_flow;
mod genai_backend;
mod models;
pub use custom::*;
pub use device_flow::*;
pub use genai_backend::{
    call_anthropic_with_tools, call_google_with_tools, call_openai_with_tools,
    encode_assistant_message, encode_tool_result,
};
pub use models::*;

/// Dispatch a tool-capable model request to the backend for `req.provider`.
///
/// Streams via `writer` for providers that support it; Google is forced
/// non-streaming (matching prior behaviour). This is the single dispatch
/// point — prefer it over picking a `call_*_with_tools` variant by hand.
pub async fn call_with_tools(
    http: &reqwest::Client,
    req: &crate::gateway::ProviderRequest,
    writer: Option<&mut dyn crate::gateway::TransportWriter>,
) -> anyhow::Result<crate::gateway::ModelResponse> {
    match req.provider.as_str() {
        "anthropic" => call_anthropic_with_tools(http, req, writer).await,
        "google" => call_google_with_tools(http, req).await,
        _ => call_openai_with_tools(http, req, writer).await,
    }
}

#[cfg(test)]
mod tests;

// ── Provider HTTP client ─────────────────────────────────────────────────────

/// How long to wait for the TCP/TLS handshake with a provider.
///
/// Reaching a host either works quickly or is not going to.
pub const PROVIDER_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a provider may go **silent** mid-response before the request is
/// abandoned.
///
/// Deliberately not a total-duration timeout. A completion is legitimately
/// long — minutes of tokens is a normal answer, and a cap on the whole request
/// would cut off the good case along with the bad. What is not normal is a
/// connection that has stopped producing bytes, and that is what this bounds.
///
/// The default is generous because time-to-first-token is the widest part of
/// the distribution: a local model on cold weights, or a hosted one under
/// load, can take a while before the first byte. Override with
/// `RUSTYCLAW_PROVIDER_READ_TIMEOUT_SECS` when running something slower.
pub const PROVIDER_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// How long an unused pooled connection is kept before being retired.
///
/// Short enough that a connection idle across a network change is dropped
/// rather than handed to the next request, long enough that back-to-back
/// turns still reuse one and skip a TLS handshake.
pub const PROVIDER_POOL_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// TCP keepalive interval on provider connections, so a peer that has gone
/// away is noticed rather than discovered by a request that hangs on it.
pub const PROVIDER_TCP_KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(30);

/// Read timeout for provider requests, honouring the environment override.
///
/// A value of `0` disables the read timeout, which restores the old
/// behaviour for anyone who needs it — at the cost of what it was fixing.
pub fn provider_read_timeout() -> Option<std::time::Duration> {
    match std::env::var("RUSTYCLAW_PROVIDER_READ_TIMEOUT_SECS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(std::time::Duration::from_secs(secs)),
            Err(_) => {
                tracing::warn!(
                    value = %raw,
                    "ignoring unparseable RUSTYCLAW_PROVIDER_READ_TIMEOUT_SECS; \
                     using the default"
                );
                Some(PROVIDER_READ_TIMEOUT)
            }
        },
        Err(_) => Some(PROVIDER_READ_TIMEOUT),
    }
}

/// The pinned trust anchor for provider TLS connections, set from the
/// `model.tls_ca_cert` config (see [`set_provider_tls_pin`]). `None` means
/// "use the system trust store".
static PROVIDER_TLS_PIN: std::sync::OnceLock<Option<reqwest::tls::Certificate>> =
    std::sync::OnceLock::new();

/// Pin the TLS trust anchor for provider connections to the certificate in
/// the given PEM file (issue #234).
///
/// Called once at config load when `model.tls_ca_cert` is set. Only the
/// first call takes effect — the pin is a boot-time decision, and letting a
/// later config reload silently swap it would defeat the purpose. A file
/// that cannot be read or parsed is a hard error: a silently-ignored pin is
/// worse than no pin, because the operator believes the connection is
/// protected when it is not.
pub fn set_provider_tls_pin(path: &std::path::Path) -> Result<()> {
    let pem = std::fs::read(path).map_err(wrap_err)?;
    let cert = reqwest::tls::Certificate::from_pem(&pem).map_err(wrap_err)?;
    match PROVIDER_TLS_PIN.set(Some(cert)) {
        Ok(()) => {
            tracing::info!(
                path = %path.display(),
                "Pinned provider TLS trust anchor"
            );
            Ok(())
        }
        Err(_) => Err(anyhow::anyhow!(
            "provider TLS pin was already set; refusing to overwrite it (set {} only at boot)",
            path.display()
        )
        .into()),
    }
}

/// Apply the configured TLS pin (if any) to a client builder.
fn apply_tls_pin(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    match PROVIDER_TLS_PIN.get().and_then(|p| p.as_ref()) {
        Some(cert) => builder.add_root_certificate(cert.clone()),
        None => builder,
    }
}

/// The configured builder behind [`http_client`], for the rare caller that
/// needs to add to it — the IPv4-only retry binds a local address before
/// building. Going through here is what keeps that retry bounded by the same
/// deadlines as the request it is retrying.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    let builder = apply_tls_pin(
        reqwest::Client::builder()
            .connect_timeout(PROVIDER_CONNECT_TIMEOUT)
            // Pool hygiene, which matters as much as the deadlines. A pooled
            // keep-alive connection whose peer went away silently — a network
            // change, a NAT rebind, a suspend — is not detectably dead until
            // something writes to it. Without these, every later request is handed
            // one of those corpses and hangs on it in turn, so a single network
            // blip reads as "the model is down" for the life of the process while
            // the gateway keeps answering connections and completing auth. Both of
            // these end that: idle connections are retired rather than reused, and
            // keepalive probes surface a dead one before a request picks it up.
            .pool_idle_timeout(PROVIDER_POOL_IDLE_TIMEOUT)
            .tcp_keepalive(PROVIDER_TCP_KEEPALIVE),
    );
    if let Some(read) = provider_read_timeout() {
        builder.read_timeout(read)
    } else {
        builder
    }
}

/// A `reqwest::Client` for talking to model providers.
///
/// Every provider request should come from here. The alternative — building a
/// client at each call site — is how the gateway ended up with the requests
/// that matter least (model listing, device flow) bounded at 10s while the one
/// carrying a user's turn had no deadline at all. One of those hung for
/// 1h56m before TCP gave up on it, and for that whole time the turn was live:
/// no completion, no error, and a client sitting on "processing" with nothing
/// to show and nothing to report.
pub fn http_client() -> reqwest::Client {
    let builder = http_client_builder();
    // A client that fails to build would leave the caller with no way to talk
    // to a provider at all; the default one at least works, minus the
    // deadlines this function exists to set.
    builder.build().unwrap_or_else(|e| {
        tracing::warn!(
            error = %e,
            "falling back to an untimed HTTP client — provider requests will not \
             be bounded"
        );
        reqwest::Client::new()
    })
}

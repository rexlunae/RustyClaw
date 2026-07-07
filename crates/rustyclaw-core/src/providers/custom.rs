//! User-configured custom model providers.
//!
//! Built-in providers live in the static [`super::PROVIDERS`] catalogue.
//! Custom providers are defined by the user in `config.toml` under
//! `[[custom_providers]]` (see [`CustomProviderConfig`]) and registered at
//! runtime via [`set_custom_providers`].  Registered providers are exposed
//! through the same lookup helpers (`provider_by_id`, `provider_ids`, …) as
//! the built-ins, so every menu, selector, and backend path picks them up
//! without special-casing.
//!
//! Because the rest of the codebase passes providers around as
//! `&'static ProviderDef`, registration converts each config entry into a
//! leaked `ProviderDef`.  The leak is bounded: entries are only re-leaked
//! when the user edits their provider list, which is a rare, interactive
//! action.

use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::{AuthMethod, ProviderDef};

/// Wire format a custom provider's API speaks.
///
/// Determines which client adapter is used for chat calls and which
/// `/models` discovery endpoint shape is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiFormat {
    /// OpenAI-compatible `/chat/completions` (vLLM, llama.cpp, LM Studio,
    /// Ollama, Joshua, text-generation-webui, most local servers).
    #[default]
    OpenAi,
    /// Anthropic Messages API.
    Anthropic,
    /// Google Gemini `generateContent` API.
    Gemini,
    /// xAI API.
    Xai,
}

impl ApiFormat {
    /// All formats, for building selection menus.
    pub const ALL: &'static [ApiFormat] = &[
        ApiFormat::OpenAi,
        ApiFormat::Anthropic,
        ApiFormat::Gemini,
        ApiFormat::Xai,
    ];

    /// Stable identifier used in config files and protocol messages.
    pub fn id(&self) -> &'static str {
        match self {
            ApiFormat::OpenAi => "openai",
            ApiFormat::Anthropic => "anthropic",
            ApiFormat::Gemini => "gemini",
            ApiFormat::Xai => "xai",
        }
    }

    /// Human-readable label for menus.
    pub fn display(&self) -> &'static str {
        match self {
            ApiFormat::OpenAi => "OpenAI-compatible (most local servers)",
            ApiFormat::Anthropic => "Anthropic Messages API",
            ApiFormat::Gemini => "Google Gemini API",
            ApiFormat::Xai => "xAI API",
        }
    }

    /// Parse from a config/protocol string. Accepts a few aliases.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "openai" | "openai-compatible" | "oai" => Some(Self::OpenAi),
            "anthropic" | "claude" => Some(Self::Anthropic),
            "gemini" | "google" => Some(Self::Gemini),
            "xai" | "grok" => Some(Self::Xai),
            _ => None,
        }
    }
}

/// A user-defined model provider, persisted in `config.toml` as
/// `[[custom_providers]]`.
///
/// ```toml
/// [[custom_providers]]
/// id = "my-vllm"
/// display_name = "vLLM on the GPU box"
/// base_url = "http://192.168.1.50:8000/v1"
/// api_format = "openai"
/// api_key_secret = "MY_VLLM_API_KEY"   # omit for keyless local servers
/// models = ["qwen3-coder-30b", "llama-3.3-70b"]
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomProviderConfig {
    /// Unique provider id (lowercase letters, digits, `-`, `_`).
    /// Must not collide with a built-in provider id.
    pub id: String,
    /// Human-readable name shown in menus. Defaults to the id.
    #[serde(default)]
    pub display_name: Option<String>,
    /// API base URL (e.g. `http://localhost:8000/v1`).
    pub base_url: String,
    /// Wire format of the endpoint. Defaults to OpenAI-compatible.
    #[serde(default)]
    pub api_format: ApiFormat,
    /// Name of the secret (vault entry or environment variable) holding the
    /// API key.  `None` means the endpoint needs no key (typical for local
    /// servers).  When set, the key is optional at call time — local servers
    /// with auth disabled still work.
    #[serde(default)]
    pub api_key_secret: Option<String>,
    /// Static model list shown in pickers.  May be empty: OpenAI-compatible
    /// endpoints are also queried live via `GET <base_url>/models`.
    #[serde(default)]
    pub models: Vec<String>,
}

/// Why a [`CustomProviderConfig`] entry was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CustomProviderError {
    #[error("Provider id must not be empty")]
    EmptyId,
    #[error("Provider id '{0}' may only contain lowercase letters, digits, '-' and '_'")]
    InvalidId(String),
    #[error("Provider id '{0}' is a built-in provider")]
    BuiltInCollision(String),
    #[error("Base URL '{0}' must start with http:// or https://")]
    InvalidBaseUrl(String),
    #[error("api_key_secret must not be empty when set")]
    EmptyApiKeySecret,
}

impl CustomProviderConfig {
    /// Display name, falling back to the id.
    pub fn display(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.id)
    }

    /// Validate the entry.
    pub fn validate(&self) -> Result<(), CustomProviderError> {
        let id = self.id.trim();
        if id.is_empty() {
            return Err(CustomProviderError::EmptyId);
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(CustomProviderError::InvalidId(id.to_string()));
        }
        if super::PROVIDERS.iter().any(|p| p.id == id) {
            return Err(CustomProviderError::BuiltInCollision(id.to_string()));
        }
        let url = self.base_url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(CustomProviderError::InvalidBaseUrl(url.to_string()));
        }
        if let Some(secret) = &self.api_key_secret {
            if secret.trim().is_empty() {
                return Err(CustomProviderError::EmptyApiKeySecret);
            }
        }
        Ok(())
    }

    /// Whether the base URL points at the local host.
    pub fn is_local(&self) -> bool {
        let url = self.base_url.to_ascii_lowercase();
        ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"]
            .iter()
            .any(|h| {
                url.starts_with(&format!("http://{h}")) || url.starts_with(&format!("https://{h}"))
            })
    }

    /// Convert into a leaked `&'static ProviderDef` so custom providers can
    /// flow through the same catalogue APIs as built-ins.
    fn to_leaked_def(&self) -> &'static ProviderDef {
        fn leak(s: &str) -> &'static str {
            Box::leak(s.to_string().into_boxed_str())
        }
        let models: Vec<&'static str> = self.models.iter().map(|m| leak(m)).collect();
        let auth_method = if self.api_key_secret.is_some() {
            AuthMethod::OptionalApiKey
        } else {
            AuthMethod::None
        };
        Box::leak(Box::new(ProviderDef {
            id: leak(&self.id),
            display: leak(self.display()),
            auth_method,
            secret_key: self.api_key_secret.as_deref().map(leak),
            device_flow: None,
            base_url: Some(leak(&self.base_url)),
            models: Box::leak(models.into_boxed_slice()),
            help_url: None,
            help_text: Some(leak(&format!(
                "Custom provider ({})",
                self.api_format.display()
            ))),
        }))
    }
}

struct CustomEntry {
    config: CustomProviderConfig,
    def: &'static ProviderDef,
}

static CUSTOM_PROVIDERS: RwLock<Vec<CustomEntry>> = RwLock::new(Vec::new());

/// Replace the registered custom providers with the given set.
///
/// Call whenever the config is loaded or the user edits their custom
/// provider list.  Invalid entries are skipped with a warning so one bad
/// config line doesn't take down the whole catalogue.
pub fn set_custom_providers(configs: &[CustomProviderConfig]) {
    let mut entries = Vec::with_capacity(configs.len());
    for cfg in configs {
        if let Err(e) = cfg.validate() {
            tracing::warn!(provider = %cfg.id, error = %e, "Skipping invalid custom provider");
            continue;
        }
        if entries
            .iter()
            .any(|existing: &CustomEntry| existing.config.id == cfg.id)
        {
            tracing::warn!(provider = %cfg.id, "Skipping duplicate custom provider id");
            continue;
        }
        entries.push(CustomEntry {
            config: cfg.clone(),
            def: cfg.to_leaked_def(),
        });
    }
    *CUSTOM_PROVIDERS.write().unwrap() = entries;
}

/// All registered custom provider defs (in config order).
pub fn custom_provider_defs() -> Vec<&'static ProviderDef> {
    CUSTOM_PROVIDERS
        .read()
        .unwrap()
        .iter()
        .map(|e| e.def)
        .collect()
}

/// Look up a custom provider def by id.
pub fn custom_provider_by_id(id: &str) -> Option<&'static ProviderDef> {
    CUSTOM_PROVIDERS
        .read()
        .unwrap()
        .iter()
        .find(|e| e.config.id == id)
        .map(|e| e.def)
}

/// Full config for a custom provider (includes the API format).
pub fn custom_provider_config(id: &str) -> Option<CustomProviderConfig> {
    CUSTOM_PROVIDERS
        .read()
        .unwrap()
        .iter()
        .find(|e| e.config.id == id)
        .map(|e| e.config.clone())
}

/// Whether the given provider id refers to a registered custom provider.
pub fn is_custom_provider(id: &str) -> bool {
    CUSTOM_PROVIDERS
        .read()
        .unwrap()
        .iter()
        .any(|e| e.config.id == id)
}

/// The API format for a provider id: custom providers report their
/// configured format; unknown/built-in ids return `None` (built-ins have
/// their own hardcoded adapter mapping).
pub fn api_format_for_provider(id: &str) -> Option<ApiFormat> {
    CUSTOM_PROVIDERS
        .read()
        .unwrap()
        .iter()
        .find(|e| e.config.id == id)
        .map(|e| e.config.api_format)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(id: &str) -> CustomProviderConfig {
        CustomProviderConfig {
            id: id.to_string(),
            display_name: Some("Test Provider".into()),
            base_url: "http://localhost:8000/v1".into(),
            api_format: ApiFormat::OpenAi,
            api_key_secret: None,
            models: vec!["test-model".into()],
        }
    }

    #[test]
    fn validate_rejects_bad_entries() {
        assert!(cfg("my-provider").validate().is_ok());
        assert_eq!(cfg("").validate(), Err(CustomProviderError::EmptyId));
        assert!(matches!(
            cfg("Has Spaces").validate(),
            Err(CustomProviderError::InvalidId(_))
        ));
        assert!(matches!(
            cfg("anthropic").validate(),
            Err(CustomProviderError::BuiltInCollision(_))
        ));
        let mut bad_url = cfg("ok-id");
        bad_url.base_url = "localhost:8000".into();
        assert!(matches!(
            bad_url.validate(),
            Err(CustomProviderError::InvalidBaseUrl(_))
        ));
    }

    #[test]
    fn registration_round_trip() {
        // Use ids unique to this test — the registry is process-global.
        let a = cfg("test-rt-a");
        let mut b = cfg("test-rt-b");
        b.api_format = ApiFormat::Anthropic;
        b.api_key_secret = Some("TEST_RT_B_KEY".into());

        set_custom_providers(&[a.clone(), b.clone()]);

        let def = custom_provider_by_id("test-rt-a").expect("registered");
        assert_eq!(def.display, "Test Provider");
        assert_eq!(def.base_url, Some("http://localhost:8000/v1"));
        assert_eq!(def.models, ["test-model"]);
        assert_eq!(def.auth_method, AuthMethod::None);

        let def_b = custom_provider_by_id("test-rt-b").expect("registered");
        assert_eq!(def_b.auth_method, AuthMethod::OptionalApiKey);
        assert_eq!(def_b.secret_key, Some("TEST_RT_B_KEY"));
        assert_eq!(
            api_format_for_provider("test-rt-b"),
            Some(ApiFormat::Anthropic)
        );
        assert!(is_custom_provider("test-rt-a"));
        assert!(!is_custom_provider("anthropic"));

        // Invalid entries are skipped, valid ones kept.
        set_custom_providers(&[cfg(""), a.clone()]);
        assert!(custom_provider_by_id("test-rt-a").is_some());
        assert!(custom_provider_by_id("test-rt-b").is_none());

        set_custom_providers(&[]);
        assert!(!is_custom_provider("test-rt-a"));
    }

    #[test]
    fn toml_round_trip() {
        let toml_src = r#"
id = "my-vllm"
base_url = "http://localhost:8000/v1"
api_format = "anthropic"
api_key_secret = "MY_VLLM_KEY"
models = ["a", "b"]
"#;
        let cfg: CustomProviderConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.api_format, ApiFormat::Anthropic);
        assert_eq!(cfg.models, ["a", "b"]);
        let out = toml::to_string(&cfg).unwrap();
        assert!(out.contains("api_format = \"anthropic\""));

        // Minimal entry: only id + base_url, defaults for the rest.
        let minimal: CustomProviderConfig =
            toml::from_str("id = \"local\"\nbase_url = \"http://localhost:1/v1\"\n").unwrap();
        assert_eq!(minimal.api_format, ApiFormat::OpenAi);
        assert!(minimal.api_key_secret.is_none());
        assert!(minimal.models.is_empty());
    }

    #[test]
    fn api_format_parsing() {
        assert_eq!(ApiFormat::parse("openai"), Some(ApiFormat::OpenAi));
        assert_eq!(ApiFormat::parse("Anthropic"), Some(ApiFormat::Anthropic));
        assert_eq!(ApiFormat::parse("google"), Some(ApiFormat::Gemini));
        assert_eq!(ApiFormat::parse("grok"), Some(ApiFormat::Xai));
        assert_eq!(ApiFormat::parse("nope"), None);
    }

    #[test]
    fn local_detection() {
        assert!(cfg("test-local").is_local());
        let mut remote = cfg("test-remote");
        remote.base_url = "https://api.example.com/v1".into();
        assert!(!remote.is_local());
    }
}

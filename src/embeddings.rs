//! Embedding generation with local and cloud providers.
//!
//! Provides vector embeddings for semantic search and similarity matching:
//! - **Local**: Privacy-preserving offline embeddings via fastembed-rs
//! - **OpenAI**: Cloud-based embeddings with higher quality
//! - **Fallback**: Automatic fallback from local to OpenAI on errors
//!
//! ## Architecture
//!
//! ```text
//! EmbeddingProvider (trait)
//!     ├── LocalEmbeddingProvider (fastembed, offline, 384-dim)
//!     ├── OpenAIEmbeddingProvider (API-based, 1536-dim)
//!     └── FallbackEmbeddingProvider (local → OpenAI on failure)
//! ```
//!
//! ## Configuration Example
//!
//! ```toml
//! [embeddings]
//! provider = "fallback"  # "local", "openai", or "fallback"
//! model = "all-MiniLM-L6-v2"  # for local provider
//! cache_dir = "~/.cache/rustyclaw/embeddings"
//! ```
//!
//! ## Performance
//!
//! - **Local**: ~100ms per embedding, ~90MB model download, offline
//! - **OpenAI**: ~200ms per embedding, API cost, requires internet
//! - **Quality**: OpenAI > Local (1536-dim vs 384-dim)

use anyhow::{Context as AnyhowContext, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Embedding provider trait for generating vector embeddings.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embedding for a single text input.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Generate embeddings for multiple text inputs (batched).
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Get the dimensionality of embeddings produced by this provider.
    fn dimensions(&self) -> usize;

    /// Get the name of the provider for logging/debugging.
    fn name(&self) -> &str;
}

/// Embedding provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    /// Provider type: "local", "openai", or "fallback"
    #[serde(default = "EmbeddingsConfig::default_provider")]
    pub provider: String,

    /// Model name for local embeddings (e.g., "all-MiniLM-L6-v2")
    #[serde(default = "EmbeddingsConfig::default_model")]
    pub model: String,

    /// Cache directory for downloaded models
    #[serde(default = "EmbeddingsConfig::default_cache_dir")]
    pub cache_dir: String,

    /// OpenAI API key (if using openai or fallback provider)
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// OpenAI model (e.g., "text-embedding-3-small")
    #[serde(default = "EmbeddingsConfig::default_openai_model")]
    pub openai_model: String,
}

impl EmbeddingsConfig {
    fn default_provider() -> String {
        "fallback".to_string()
    }

    fn default_model() -> String {
        "all-MiniLM-L6-v2".to_string()
    }

    fn default_cache_dir() -> String {
        "~/.cache/rustyclaw/embeddings".to_string()
    }

    fn default_openai_model() -> String {
        "text-embedding-3-small".to_string()
    }
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            provider: Self::default_provider(),
            model: Self::default_model(),
            cache_dir: Self::default_cache_dir(),
            openai_api_key: None,
            openai_model: Self::default_openai_model(),
        }
    }
}

/// Create an embedding provider from configuration.
pub fn create_provider(config: &EmbeddingsConfig) -> Result<Box<dyn EmbeddingProvider>> {
    match config.provider.as_str() {
        "local" => {
            #[cfg(feature = "local-embeddings")]
            {
                let provider = LocalEmbeddingProvider::new(config)?;
                Ok(Box::new(provider))
            }
            #[cfg(not(feature = "local-embeddings"))]
            {
                anyhow::bail!(
                    "Local embeddings provider requested but 'local-embeddings' feature not enabled. \
                    Rebuild with --features local-embeddings"
                );
            }
        }
        "openai" => {
            let provider = OpenAIEmbeddingProvider::new(config)?;
            Ok(Box::new(provider))
        }
        "fallback" => {
            let provider = FallbackEmbeddingProvider::new(config)?;
            Ok(Box::new(provider))
        }
        other => anyhow::bail!("Unknown embedding provider: {}", other),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Local Embedding Provider (fastembed-rs)
// ────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "local-embeddings")]
pub struct LocalEmbeddingProvider {
    model: std::sync::Arc<tokio::sync::Mutex<fastembed::TextEmbedding>>,
    dimensions: usize,
}

#[cfg(feature = "local-embeddings")]
impl LocalEmbeddingProvider {
    /// Create a new local embedding provider.
    pub fn new(config: &EmbeddingsConfig) -> Result<Self> {
        use fastembed::{EmbeddingModel, TextEmbedding, InitOptions};

        // For now, only support all-MiniLM-L6-v2
        if config.model != "all-MiniLM-L6-v2" && config.model != "AllMiniLML6V2" {
            anyhow::bail!(
                "Unknown local embedding model: '{}'. Supported: all-MiniLM-L6-v2",
                config.model
            );
        }

        // Expand cache directory path
        let cache_dir = shellexpand::tilde(&config.cache_dir).to_string();
        let cache_path = PathBuf::from(cache_dir);

        println!("[Embeddings] Initializing local model: {}", config.model);
        println!("[Embeddings] Cache directory: {}", config.cache_dir);
        println!("[Embeddings] Model will be downloaded on first use (~90MB)");

        // Initialize fastembed with cache directory
        let mut init_options = InitOptions::new(EmbeddingModel::AllMiniLML6V2);
        init_options = init_options.with_cache_dir(cache_path);

        let model = TextEmbedding::try_new(init_options)
            .context("Failed to initialize local embedding model")?;

        Ok(Self {
            model: std::sync::Arc::new(tokio::sync::Mutex::new(model)),
            dimensions: 384, // all-MiniLM-L6-v2 produces 384-dimensional embeddings
        })
    }
}

#[cfg(feature = "local-embeddings")]
#[async_trait]
impl EmbeddingProvider for LocalEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut model = self.model.lock().await;
        let embeddings = model.embed(vec![text], None)?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No embedding returned"))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut model = self.model.lock().await;
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = model.embed(text_refs, None)?;
        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn name(&self) -> &str {
        "local-fastembed"
    }
}

// ────────────────────────────────────────────────────────────────────────────
// OpenAI Embedding Provider
// ────────────────────────────────────────────────────────────────────────────

pub struct OpenAIEmbeddingProvider {
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl OpenAIEmbeddingProvider {
    /// Create a new OpenAI embedding provider.
    pub fn new(config: &EmbeddingsConfig) -> Result<Self> {
        let api_key = config
            .openai_api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "OpenAI API key not found in config or OPENAI_API_KEY environment variable"
                )
            })?;

        let dimensions = Self::get_model_dimensions(&config.openai_model);

        Ok(Self {
            api_key,
            model: config.openai_model.clone(),
            dimensions,
            client: reqwest::Client::new(),
        })
    }

    /// Get dimensions for OpenAI models.
    fn get_model_dimensions(model: &str) -> usize {
        match model {
            "text-embedding-3-small" => 1536,
            "text-embedding-3-large" => 3072,
            "text-embedding-ada-002" => 1536,
            _ => 1536, // Default fallback
        }
    }

    /// Call OpenAI embeddings API.
    async fn call_api(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        #[derive(Serialize)]
        struct EmbeddingRequest {
            model: String,
            input: Vec<String>,
        }

        #[derive(Deserialize)]
        struct EmbeddingResponse {
            data: Vec<EmbeddingData>,
        }

        #[derive(Deserialize)]
        struct EmbeddingData {
            embedding: Vec<f32>,
        }

        let request = EmbeddingRequest {
            model: self.model.clone(),
            input: texts,
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to OpenAI API")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error {}: {}", status, body);
        }

        let response_data: EmbeddingResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI API response")?;

        Ok(response_data.data.into_iter().map(|d| d.embedding).collect())
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self.call_api(vec![text.to_string()]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No embedding returned"))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.call_api(texts.to_vec()).await
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn name(&self) -> &str {
        "openai"
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Fallback Embedding Provider
// ────────────────────────────────────────────────────────────────────────────

pub struct FallbackEmbeddingProvider {
    #[cfg(feature = "local-embeddings")]
    local: Option<LocalEmbeddingProvider>,
    openai: Option<OpenAIEmbeddingProvider>,
}

impl FallbackEmbeddingProvider {
    /// Create a new fallback embedding provider.
    pub fn new(config: &EmbeddingsConfig) -> Result<Self> {
        #[cfg(feature = "local-embeddings")]
        let local = LocalEmbeddingProvider::new(config).ok();

        let openai = OpenAIEmbeddingProvider::new(config).ok();

        #[cfg(feature = "local-embeddings")]
        if local.is_none() && openai.is_none() {
            anyhow::bail!("No embedding providers available (both local and OpenAI failed to initialize)");
        }

        #[cfg(not(feature = "local-embeddings"))]
        if openai.is_none() {
            anyhow::bail!("No embedding providers available (OpenAI failed to initialize and local-embeddings feature not enabled)");
        }

        Ok(Self {
            #[cfg(feature = "local-embeddings")]
            local,
            openai,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for FallbackEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // Try local first
        #[cfg(feature = "local-embeddings")]
        if let Some(ref local) = self.local {
            if let Ok(embedding) = local.embed(text).await {
                return Ok(embedding);
            } else {
                eprintln!("[Embeddings] Local provider failed, falling back to OpenAI");
            }
        }

        // Fallback to OpenAI
        if let Some(ref openai) = self.openai {
            return openai.embed(text).await;
        }

        anyhow::bail!("All embedding providers failed")
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // Try local first
        #[cfg(feature = "local-embeddings")]
        if let Some(ref local) = self.local {
            if let Ok(embeddings) = local.embed_batch(texts).await {
                return Ok(embeddings);
            } else {
                eprintln!("[Embeddings] Local provider failed, falling back to OpenAI");
            }
        }

        // Fallback to OpenAI
        if let Some(ref openai) = self.openai {
            return openai.embed_batch(texts).await;
        }

        anyhow::bail!("All embedding providers failed")
    }

    fn dimensions(&self) -> usize {
        #[cfg(feature = "local-embeddings")]
        if let Some(ref local) = self.local {
            return local.dimensions();
        }

        if let Some(ref openai) = self.openai {
            return openai.dimensions();
        }

        1536 // Default fallback
    }

    fn name(&self) -> &str {
        "fallback"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_openai_provider_creation() {
        let config = EmbeddingsConfig {
            provider: "openai".to_string(),
            openai_api_key: Some("sk-test-key".to_string()),
            openai_model: "text-embedding-3-small".to_string(),
            ..Default::default()
        };

        let provider = OpenAIEmbeddingProvider::new(&config);
        assert!(provider.is_ok());

        let provider = provider.unwrap();
        assert_eq!(provider.dimensions(), 1536);
        assert_eq!(provider.name(), "openai");
    }

    #[cfg(feature = "local-embeddings")]
    #[tokio::test]
    async fn test_local_provider_creation() {
        let config = EmbeddingsConfig {
            provider: "local".to_string(),
            model: "all-MiniLM-L6-v2".to_string(),
            ..Default::default()
        };

        // Note: This test will download the model on first run
        let provider = LocalEmbeddingProvider::new(&config);
        if provider.is_ok() {
            let provider = provider.unwrap();
            assert_eq!(provider.dimensions(), 384);
            assert_eq!(provider.name(), "local-fastembed");
        }
    }

    #[test]
    fn test_config_defaults() {
        let config = EmbeddingsConfig::default();
        assert_eq!(config.provider, "fallback");
        assert_eq!(config.model, "all-MiniLM-L6-v2");
        assert_eq!(config.openai_model, "text-embedding-3-small");
    }
}

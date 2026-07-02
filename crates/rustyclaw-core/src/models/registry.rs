//! Model registry — manages available models with cost tiers and enable/disable.
//!
//! This module provides:
//! - A registry of all configured models
//! - Cost tier classification (premium, standard, economy, free)
//! - Enable/disable per model (independent of active selection)
//! - Model selection recommendations for sub-agents

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Cost tier for a model — used to guide sub-agent model selection.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum CostTier {
    /// Free models (local Ollama, free API tiers)
    Free,
    /// Economy models (fast, cheap — good for simple tasks)
    Economy,
    /// Standard models (balanced cost/capability)
    #[default]
    Standard,
    /// Premium models (highest capability, highest cost)
    Premium,
}

impl std::str::FromStr for CostTier {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "free" => Ok(Self::Free),
            "economy" | "eco" | "cheap" => Ok(Self::Economy),
            "standard" | "std" | "balanced" => Ok(Self::Standard),
            "premium" | "pro" | "expensive" => Ok(Self::Premium),
            _ => Err(()),
        }
    }
}

impl CostTier {
    pub fn display(&self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Economy => "Economy",
            Self::Standard => "Standard",
            Self::Premium => "Premium",
        }
    }

    /// Emoji indicator.
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Free => "🆓",
            Self::Economy => "💰",
            Self::Standard => "⚖️",
            Self::Premium => "💎",
        }
    }
}

/// Task complexity hint for model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskComplexity {
    /// Simple tasks: grep, list, format, summarize
    Simple,
    /// Medium tasks: code edits, analysis, research
    Medium,
    /// Complex tasks: architecture, debugging, multi-step reasoning
    Complex,
    /// Critical tasks: security, production changes, important decisions
    Critical,
}

impl TaskComplexity {
    /// Recommended minimum cost tier for this complexity.
    pub fn recommended_tier(&self) -> CostTier {
        match self {
            Self::Simple => CostTier::Free,
            Self::Medium => CostTier::Economy,
            Self::Complex => CostTier::Standard,
            Self::Critical => CostTier::Premium,
        }
    }
}

/// Whether a model provider runs locally or calls an external API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Locally executed and managed by the gateway (e.g. Ollama, llama.cpp).
    Internal,
    /// Remote API provider (e.g. Anthropic, OpenAI, Google).
    #[default]
    External,
    /// Subscription / proxy provider where usage is covered by a flat fee
    /// (e.g. GitHub Copilot).
    Subscription,
}

impl ProviderKind {
    pub fn display(&self) -> &'static str {
        match self {
            Self::Internal => "Internal",
            Self::External => "External",
            Self::Subscription => "Subscription",
        }
    }

    /// Whether this provider kind executes on the local host.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Internal)
    }
}

/// Resource requirements for running a model locally.
///
/// Only meaningful for [`ProviderKind::Internal`] models.  External
/// models have `None` for all fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRequirements {
    /// Minimum system RAM in bytes.
    pub min_memory_bytes: Option<u64>,
    /// Minimum GPU VRAM in bytes.
    pub min_vram_bytes: Option<u64>,
    /// Recommended number of CPU cores.
    pub recommended_cpu_cores: Option<u32>,
    /// Approximate model weight size on disk (bytes).
    pub disk_size_bytes: Option<u64>,
}

impl ResourceRequirements {
    /// Check whether the host has enough resources to run this model.
    pub fn satisfies(&self, host: &crate::host::HostCapabilities) -> bool {
        if let Some(min_mem) = self.min_memory_bytes {
            if host.total_memory_bytes < min_mem {
                return false;
            }
        }
        if let Some(min_vram) = self.min_vram_bytes {
            if host.total_vram_bytes() < min_vram {
                return false;
            }
        }
        if let Some(cores) = self.recommended_cpu_cores {
            if (host.cpu_cores_logical as u32) < cores {
                return false;
            }
        }
        true
    }
}

/// A registered model with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Full model ID (e.g., "anthropic/claude-sonnet-4")
    pub id: String,

    /// Provider ID (e.g., "anthropic", "openai", "ollama")
    pub provider: String,

    /// Short model name (e.g., "claude-sonnet-4")
    pub name: String,

    /// Display name for UI
    pub display_name: String,

    /// Cost tier
    pub tier: CostTier,

    /// Whether the model is enabled for use
    pub enabled: bool,

    /// Whether credentials are available for this model
    pub available: bool,

    /// Context window size (tokens)
    pub context_window: Option<u32>,

    /// Supports vision/images
    pub supports_vision: bool,

    /// Supports tool use
    pub supports_tools: bool,

    /// Supports extended thinking
    pub supports_thinking: bool,

    /// Optional notes
    pub notes: Option<String>,

    /// Whether this model runs locally or via an external API.
    #[serde(default)]
    pub provider_kind: ProviderKind,

    /// Resource requirements for local execution.
    #[serde(default)]
    pub resource_requirements: ResourceRequirements,
}

impl ModelEntry {
    /// Create a new model entry.
    pub fn new(id: impl Into<String>, provider: impl Into<String>, tier: CostTier) -> Self {
        let id = id.into();
        let provider = provider.into();
        let name = id.split('/').next_back().unwrap_or(&id).to_string();
        let display_name = format_display_name(&name);

        Self {
            id,
            provider,
            name,
            display_name,
            tier,
            enabled: true,
            available: false,
            context_window: None,
            supports_vision: false,
            supports_tools: true,
            supports_thinking: false,
            notes: None,
            provider_kind: ProviderKind::default(),
            resource_requirements: ResourceRequirements::default(),
        }
    }

    /// Builder: set context window.
    pub fn with_context(mut self, tokens: u32) -> Self {
        self.context_window = Some(tokens);
        self
    }

    /// Builder: set vision support.
    pub fn with_vision(mut self) -> Self {
        self.supports_vision = true;
        self
    }

    /// Builder: set thinking support.
    pub fn with_thinking(mut self) -> Self {
        self.supports_thinking = true;
        self
    }

    /// Builder: set notes.
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// Builder: set provider kind.
    pub fn with_provider_kind(mut self, kind: ProviderKind) -> Self {
        self.provider_kind = kind;
        self
    }

    /// Builder: set resource requirements.
    pub fn with_resources(mut self, reqs: ResourceRequirements) -> Self {
        self.resource_requirements = reqs;
        self
    }

    /// Check if model can be used (enabled + available).
    pub fn is_usable(&self) -> bool {
        self.enabled && self.available
    }

    /// Whether this model runs on the local host.
    pub fn is_local(&self) -> bool {
        self.provider_kind.is_local()
    }

    /// Check whether the host can run this model (always true for external).
    pub fn can_run_on(&self, host: &crate::host::HostCapabilities) -> bool {
        if !self.is_local() {
            return true;
        }
        self.resource_requirements.satisfies(host)
    }

    /// Format for display with tier indicator.
    pub fn format_display(&self) -> String {
        let status = if !self.available {
            "⚪"
        } else if !self.enabled {
            "🔴"
        } else {
            "🟢"
        };
        let kind_tag = match self.provider_kind {
            ProviderKind::Internal => " [local]",
            ProviderKind::Subscription => " [sub]",
            ProviderKind::External => "",
        };
        format!(
            "{} {} {} ({}{})",
            status,
            self.tier.emoji(),
            self.display_name,
            self.provider,
            kind_tag,
        )
    }
}

/// Errors produced by [`ModelRegistry`] operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// No model with this ID is registered.
    #[error("Model not found: {0}")]
    ModelNotFound(String),
}

/// Model registry — manages all available models.
pub struct ModelRegistry {
    /// All registered models by ID
    models: HashMap<String, ModelEntry>,

    /// Currently active model ID
    active_model: Option<String>,

    /// Default model for sub-agents by complexity
    subagent_defaults: HashMap<TaskComplexity, String>,
}

impl ModelRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            active_model: None,
            subagent_defaults: HashMap::new(),
        }
    }

    /// Create with default model entries from providers.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.populate_defaults();
        registry
    }

    /// Populate with default models from known providers.
    fn populate_defaults(&mut self) {
        // Model catalog is populated dynamically from providers::fetch_models.
        // This method only seeds the subagent recommendation defaults.

        // Set default subagent models (policy-only — populated dynamically).
        self.subagent_defaults
            .insert(TaskComplexity::Simple, "ollama/llama3.2:3b".to_string());
        self.subagent_defaults.insert(
            TaskComplexity::Medium,
            "anthropic/claude-haiku-4".to_string(),
        );
        self.subagent_defaults.insert(
            TaskComplexity::Complex,
            "anthropic/claude-sonnet-4".to_string(),
        );
        self.subagent_defaults.insert(
            TaskComplexity::Critical,
            "anthropic/claude-opus-4".to_string(),
        );
    }

    /// Register a model.
    pub fn register(&mut self, model: ModelEntry) {
        debug!(model_id = %model.id, tier = ?model.tier, "Registering model");
        self.models.insert(model.id.clone(), model);
    }

    /// Get a model by ID.
    pub fn get(&self, id: &str) -> Option<&ModelEntry> {
        self.models.get(id)
    }

    /// Populate the registry from a provider's live model list.
    ///
    /// This is the single source of truth for the model catalog: it
    /// calls [`crate::providers::fetch_models_detailed`] (which already
    /// merges live API results with any curated static fallback) and
    /// registers each returned model with an inferred [`CostTier`] and
    /// capability flags.
    ///
    /// Existing entries for the same provider are replaced.  Errors
    /// from the provider API are returned to the caller; on success
    /// the number of registered models is returned.
    pub async fn populate_from_provider(
        &mut self,
        provider_id: &str,
        api_key: Option<&str>,
        base_url: Option<&str>,
    ) -> Result<usize, anyhow::Error> {
        let models =
            crate::providers::fetch_models_detailed(provider_id, api_key, base_url).await?;

        // Drop existing entries for this provider so the registry
        // exactly mirrors what the provider currently offers.
        self.models.retain(|_, entry| entry.provider != provider_id);

        let count = models.len();
        for info in models {
            // Normalize to a fully-qualified id ("provider/model") so
            // the registry stays consistent regardless of how the
            // provider names its entries.
            let qualified_id = if info.id.starts_with(&format!("{}/", provider_id)) {
                info.id.clone()
            } else {
                format!("{}/{}", provider_id, info.id)
            };

            let tier = infer_cost_tier(provider_id, &info.id);
            let kind = infer_provider_kind(provider_id);
            let mut entry =
                ModelEntry::new(qualified_id.clone(), provider_id, tier).with_provider_kind(kind);
            if let Some(name) = info.name {
                entry.display_name = name;
            }
            if let Some(ctx) = info.context_length {
                entry.context_window = Some(ctx.min(u32::MAX as u64) as u32);
            }
            // Mark available — we found it in the provider list.
            entry.available = true;
            // Capability inference from id patterns.
            let lower = info.id.to_lowercase();
            if lower.contains("vision")
                || lower.contains("claude")
                || lower.contains("gpt-4")
                || lower.contains("gpt-5")
                || lower.contains("gemini")
                || lower.contains("o3")
                || lower.contains("o4")
            {
                entry.supports_vision = true;
            }
            if lower.contains("thinking")
                || lower.contains("opus")
                || lower.contains("sonnet")
                || lower.contains("o3")
                || lower.contains("o4")
                || lower.contains("reasoning")
            {
                entry.supports_thinking = true;
            }
            self.register(entry);
        }

        Ok(count)
    }

    /// Get a mutable model by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut ModelEntry> {
        self.models.get_mut(id)
    }

    /// List all models.
    pub fn all(&self) -> Vec<&ModelEntry> {
        let mut models: Vec<_> = self.models.values().collect();
        models.sort_by(|a, b| {
            a.tier
                .cmp(&b.tier)
                .then_with(|| a.provider.cmp(&b.provider))
                .then_with(|| a.name.cmp(&b.name))
        });
        models
    }

    /// List enabled models.
    pub fn enabled(&self) -> Vec<&ModelEntry> {
        self.all().into_iter().filter(|m| m.enabled).collect()
    }

    /// List usable models (enabled + available).
    pub fn usable(&self) -> Vec<&ModelEntry> {
        self.all().into_iter().filter(|m| m.is_usable()).collect()
    }

    /// List models by tier.
    pub fn by_tier(&self, tier: CostTier) -> Vec<&ModelEntry> {
        self.all().into_iter().filter(|m| m.tier == tier).collect()
    }

    /// List models by provider kind.
    pub fn by_kind(&self, kind: ProviderKind) -> Vec<&ModelEntry> {
        self.all()
            .into_iter()
            .filter(|m| m.provider_kind == kind)
            .collect()
    }

    /// List internal (locally-managed) models.
    pub fn internal_models(&self) -> Vec<&ModelEntry> {
        self.by_kind(ProviderKind::Internal)
    }

    /// List external (API-based) models.
    pub fn external_models(&self) -> Vec<&ModelEntry> {
        self.by_kind(ProviderKind::External)
    }

    /// List models that can run on the given host.
    pub fn runnable_on(&self, host: &crate::host::HostCapabilities) -> Vec<&ModelEntry> {
        self.usable()
            .into_iter()
            .filter(|m| m.can_run_on(host))
            .collect()
    }

    /// Enable a model.
    pub fn enable(&mut self, id: &str) -> Result<(), RegistryError> {
        let model = self
            .models
            .get_mut(id)
            .ok_or_else(|| RegistryError::ModelNotFound(id.to_string()))?;
        model.enabled = true;
        info!(model_id = %id, "Model enabled");
        Ok(())
    }

    /// Disable a model.
    pub fn disable(&mut self, id: &str) -> Result<(), RegistryError> {
        let model = self
            .models
            .get_mut(id)
            .ok_or_else(|| RegistryError::ModelNotFound(id.to_string()))?;
        model.enabled = false;
        info!(model_id = %id, "Model disabled");
        Ok(())
    }

    /// Set model availability (based on credentials).
    pub fn set_available(&mut self, id: &str, available: bool) {
        if let Some(model) = self.models.get_mut(id) {
            model.available = available;
        }
    }

    /// Set the active model.
    pub fn set_active(&mut self, id: &str) -> Result<(), RegistryError> {
        if !self.models.contains_key(id) {
            return Err(RegistryError::ModelNotFound(id.to_string()));
        }
        self.active_model = Some(id.to_string());
        info!(model_id = %id, "Active model set");
        Ok(())
    }

    /// Get the active model.
    pub fn active(&self) -> Option<&ModelEntry> {
        self.active_model
            .as_ref()
            .and_then(|id| self.models.get(id))
    }

    /// Get recommended model for a sub-agent task.
    pub fn recommend_for_subagent(&self, complexity: TaskComplexity) -> Option<&ModelEntry> {
        // Try the configured default for this complexity
        if let Some(default_id) = self.subagent_defaults.get(&complexity) {
            if let Some(model) = self.models.get(default_id) {
                if model.is_usable() {
                    return Some(model);
                }
            }
        }

        // Fall back: find any usable model at or below the recommended tier
        let recommended_tier = complexity.recommended_tier();
        self.usable()
            .into_iter()
            .filter(|m| m.tier <= recommended_tier)
            .max_by_key(|m| m.tier) // Prefer highest tier within budget
    }

    /// Set the default model for a complexity level.
    pub fn set_subagent_default(&mut self, complexity: TaskComplexity, model_id: String) {
        self.subagent_defaults.insert(complexity, model_id);
    }

    /// Get subagent defaults.
    pub fn subagent_defaults(&self) -> &HashMap<TaskComplexity, String> {
        &self.subagent_defaults
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Shared model registry.
pub type SharedModelRegistry = Arc<RwLock<ModelRegistry>>;

/// Create a shared model registry.
pub fn create_model_registry() -> SharedModelRegistry {
    Arc::new(RwLock::new(ModelRegistry::with_defaults()))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Infer the [`ProviderKind`] from a provider id.
pub fn infer_provider_kind(provider_id: &str) -> ProviderKind {
    match provider_id {
        "ollama" | "lmstudio" | "exo" | "llamacpp" | "vllm" => ProviderKind::Internal,
        "github-copilot" => ProviderKind::Subscription,
        _ => ProviderKind::External,
    }
}

/// Infer the [`CostTier`] for a model based on provider and id heuristics.
///
/// Used when populating the registry from a provider's live model list,
/// since the live API doesn't carry tier metadata.
pub fn infer_cost_tier(provider_id: &str, model_id: &str) -> CostTier {
    let lower = model_id.to_lowercase();

    // Local / subscription providers are always free at point-of-use.
    if matches!(
        provider_id,
        "ollama" | "lmstudio" | "exo" | "github-copilot"
    ) {
        return CostTier::Free;
    }

    // Premium: flagship models from each provider.
    if lower.contains("opus")
        || lower.contains("o3")
        || lower.contains("gpt-5")
        || lower.contains("gemini-2.5-pro")
        || lower.contains("gemini-3-pro")
        || lower.contains("gemini-3.1-pro")
    {
        return CostTier::Premium;
    }

    // Economy: explicit small/mini/flash/haiku/nano variants.
    if lower.contains("haiku")
        || lower.contains("mini")
        || lower.contains("nano")
        || lower.contains("flash")
        || lower.contains("3b")
        || lower.contains("8b")
    {
        return CostTier::Economy;
    }

    // Everything else (sonnet, gpt-4.x, gemini pro non-flagship, grok) → Standard.
    CostTier::Standard
}

/// Format a model name for display.
fn format_display_name(name: &str) -> String {
    // Convert snake_case or kebab-case to Title Case
    name.split(&['-', '_'][..])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generate system prompt section for sub-agent model selection guidance.
pub fn generate_subagent_guidance(registry: &ModelRegistry) -> String {
    let mut guidance = String::from(
        "## Sub-Agent Model Selection\n\n\
        When spawning sub-agents, choose models based on task complexity:\n\n",
    );

    // List defaults by complexity
    for (complexity, default_id) in registry.subagent_defaults() {
        let tier = complexity.recommended_tier();
        let model_name = registry
            .get(default_id)
            .map(|m| m.display_name.as_str())
            .unwrap_or(default_id);
        guidance.push_str(&format!(
            "- **{:?}** tasks → {} {} (default: {})\n",
            complexity,
            tier.emoji(),
            tier.display(),
            model_name
        ));
    }

    guidance.push_str("\n\
        **Spawn sub-agents freely!** The async architecture handles multiple concurrent agents efficiently.\n\
        Use cheaper models for:\n\
        - Simple file operations, grep, formatting\n\
        - Routine code edits with clear instructions\n\
        - Data transformation, summarization\n\
        - Background monitoring tasks\n\n\
        Reserve premium models for:\n\
        - Complex debugging and architecture decisions\n\
        - Security-sensitive operations\n\
        - Tasks requiring deep reasoning\n\n\
        Sub-agents run asynchronously — you can spawn several and continue working.\n"
    );

    guidance
}

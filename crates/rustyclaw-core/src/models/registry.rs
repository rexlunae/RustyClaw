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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CostTier {
    /// Free models (local Ollama, free API tiers)
    Free,
    /// Economy models (fast, cheap — good for simple tasks)
    Economy,
    /// Standard models (balanced cost/capability)
    Standard,
    /// Premium models (highest capability, highest cost)
    Premium,
}

impl CostTier {
    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "free" => Some(Self::Free),
            "economy" | "eco" | "cheap" => Some(Self::Economy),
            "standard" | "std" | "balanced" => Some(Self::Standard),
            "premium" | "pro" | "expensive" => Some(Self::Premium),
            _ => None,
        }
    }

    /// Display name.
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

impl Default for CostTier {
    fn default() -> Self {
        Self::Standard
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
}

impl ModelEntry {
    /// Create a new model entry.
    pub fn new(id: impl Into<String>, provider: impl Into<String>, tier: CostTier) -> Self {
        let id = id.into();
        let provider = provider.into();
        let name = id.split('/').last().unwrap_or(&id).to_string();
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

    /// Check if model can be used (enabled + available).
    pub fn is_usable(&self) -> bool {
        self.enabled && self.available
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
        format!("{} {} {} ({})", status, self.tier.emoji(), self.display_name, self.provider)
    }
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
        // Anthropic
        self.register(ModelEntry::new("anthropic/claude-opus-4", "anthropic", CostTier::Premium)
            .with_context(200_000).with_vision().with_thinking());
        self.register(ModelEntry::new("anthropic/claude-sonnet-4", "anthropic", CostTier::Standard)
            .with_context(200_000).with_vision().with_thinking());
        self.register(ModelEntry::new("anthropic/claude-haiku-4", "anthropic", CostTier::Economy)
            .with_context(200_000).with_vision());

        // OpenAI
        self.register(ModelEntry::new("openai/gpt-4.1", "openai", CostTier::Standard)
            .with_context(128_000).with_vision());
        self.register(ModelEntry::new("openai/gpt-4.1-mini", "openai", CostTier::Economy)
            .with_context(128_000).with_vision());
        self.register(ModelEntry::new("openai/gpt-4.1-nano", "openai", CostTier::Economy)
            .with_context(128_000));
        self.register(ModelEntry::new("openai/o3", "openai", CostTier::Premium)
            .with_context(200_000).with_thinking());
        self.register(ModelEntry::new("openai/o4-mini", "openai", CostTier::Standard)
            .with_context(200_000).with_thinking());

        // Google
        self.register(ModelEntry::new("google/gemini-2.5-pro", "google", CostTier::Standard)
            .with_context(1_000_000).with_vision().with_thinking());
        self.register(ModelEntry::new("google/gemini-2.5-flash", "google", CostTier::Economy)
            .with_context(1_000_000).with_vision());
        self.register(ModelEntry::new("google/gemini-2.0-flash", "google", CostTier::Economy)
            .with_context(1_000_000).with_vision());

        // xAI
        self.register(ModelEntry::new("xai/grok-3", "xai", CostTier::Standard)
            .with_context(131_072).with_vision());
        self.register(ModelEntry::new("xai/grok-3-mini", "xai", CostTier::Economy)
            .with_context(131_072).with_thinking());

        // GitHub Copilot (via proxy)
        self.register(ModelEntry::new("github-copilot/claude-opus-4", "github-copilot", CostTier::Free)
            .with_context(200_000).with_vision().with_thinking()
            .with_notes("Via Copilot subscription"));
        self.register(ModelEntry::new("github-copilot/claude-sonnet-4", "github-copilot", CostTier::Free)
            .with_context(200_000).with_vision().with_thinking()
            .with_notes("Via Copilot subscription"));
        self.register(ModelEntry::new("github-copilot/gpt-4.1", "github-copilot", CostTier::Free)
            .with_context(128_000).with_vision()
            .with_notes("Via Copilot subscription"));

        // Local (Ollama)
        self.register(ModelEntry::new("ollama/llama3.1", "ollama", CostTier::Free)
            .with_context(128_000).with_notes("Local inference"));
        self.register(ModelEntry::new("ollama/llama3.2:3b", "ollama", CostTier::Free)
            .with_context(128_000).with_notes("Lightweight local"));
        self.register(ModelEntry::new("ollama/qwen2.5-coder", "ollama", CostTier::Free)
            .with_context(32_000).with_notes("Code-focused local"));

        // Set default subagent models
        self.subagent_defaults.insert(TaskComplexity::Simple, "ollama/llama3.2:3b".to_string());
        self.subagent_defaults.insert(TaskComplexity::Medium, "anthropic/claude-haiku-4".to_string());
        self.subagent_defaults.insert(TaskComplexity::Complex, "anthropic/claude-sonnet-4".to_string());
        self.subagent_defaults.insert(TaskComplexity::Critical, "anthropic/claude-opus-4".to_string());
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

    /// Get a mutable model by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut ModelEntry> {
        self.models.get_mut(id)
    }

    /// List all models.
    pub fn all(&self) -> Vec<&ModelEntry> {
        let mut models: Vec<_> = self.models.values().collect();
        models.sort_by(|a, b| {
            a.tier.cmp(&b.tier)
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

    /// Enable a model.
    pub fn enable(&mut self, id: &str) -> Result<(), String> {
        let model = self.models.get_mut(id)
            .ok_or_else(|| format!("Model not found: {}", id))?;
        model.enabled = true;
        info!(model_id = %id, "Model enabled");
        Ok(())
    }

    /// Disable a model.
    pub fn disable(&mut self, id: &str) -> Result<(), String> {
        let model = self.models.get_mut(id)
            .ok_or_else(|| format!("Model not found: {}", id))?;
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
    pub fn set_active(&mut self, id: &str) -> Result<(), String> {
        if !self.models.contains_key(id) {
            return Err(format!("Model not found: {}", id));
        }
        self.active_model = Some(id.to_string());
        info!(model_id = %id, "Active model set");
        Ok(())
    }

    /// Get the active model.
    pub fn active(&self) -> Option<&ModelEntry> {
        self.active_model.as_ref().and_then(|id| self.models.get(id))
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
        When spawning sub-agents, choose models based on task complexity:\n\n"
    );

    // List defaults by complexity
    for (complexity, default_id) in registry.subagent_defaults() {
        let tier = complexity.recommended_tier();
        let model_name = registry.get(default_id)
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

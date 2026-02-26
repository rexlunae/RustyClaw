//! Model management module.
//!
//! Provides:
//! - Model registry with enable/disable
//! - Cost tiers for intelligent model selection
//! - Sub-agent model recommendations

mod registry;

pub use registry::{
    CostTier, ModelEntry, ModelRegistry, SharedModelRegistry, TaskComplexity,
    create_model_registry, generate_subagent_guidance,
};

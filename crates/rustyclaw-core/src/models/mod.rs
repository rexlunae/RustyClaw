//! Model management module.
//!
//! Provides:
//! - Model registry with enable/disable
//! - Cost tiers for intelligent model selection
//! - Sub-agent model recommendations

pub mod failover;
mod registry;

pub use failover::{AuthProfile, FailoverConfig, FailoverManager, FailoverStrategy, HealthTracker};
pub use registry::{
    CostTier, ModelEntry, ModelRegistry, SharedModelRegistry, TaskComplexity,
    create_model_registry, generate_subagent_guidance,
};

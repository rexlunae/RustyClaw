//! Model handler — gateway-side model tool dispatch.
//!
//! Handles model_* tool calls by interacting with the shared ModelRegistry.

use serde_json::{json, Value};
use tracing::{debug, instrument};

use crate::models::{CostTier, TaskComplexity, SharedModelRegistry};

/// Check if a tool name is a model tool.
pub fn is_model_tool(name: &str) -> bool {
    matches!(
        name,
        "model_list" | "model_enable" | "model_disable" | "model_set" | "model_recommend"
    )
}

/// Execute a model tool call.
#[instrument(skip(model_registry, args), fields(tool = %name))]
pub async fn execute_model_tool(
    name: &str,
    args: &Value,
    model_registry: &SharedModelRegistry,
) -> Result<String, String> {
    match name {
        "model_list" => exec_model_list(args, model_registry).await,
        "model_enable" => exec_model_enable(args, model_registry).await,
        "model_disable" => exec_model_disable(args, model_registry).await,
        "model_set" => exec_model_set(args, model_registry).await,
        "model_recommend" => exec_model_recommend(args, model_registry).await,
        _ => Err(format!("Unknown model tool: {}", name)),
    }
}

/// List available models.
async fn exec_model_list(
    args: &Value,
    model_registry: &SharedModelRegistry,
) -> Result<String, String> {
    let tier_filter = args.get("tier")
        .and_then(|v| v.as_str())
        .and_then(CostTier::from_str);
    let enabled_only = args.get("enabledOnly").and_then(|v| v.as_bool()).unwrap_or(false);
    let usable_only = args.get("usableOnly").and_then(|v| v.as_bool()).unwrap_or(false);

    let registry = model_registry.read().await;

    let models: Vec<_> = registry.all()
        .into_iter()
        .filter(|m| {
            if let Some(tier) = tier_filter {
                if m.tier != tier {
                    return false;
                }
            }
            if enabled_only && !m.enabled {
                return false;
            }
            if usable_only && !m.is_usable() {
                return false;
            }
            true
        })
        .map(|m| {
            json!({
                "id": m.id,
                "provider": m.provider,
                "name": m.name,
                "displayName": m.display_name,
                "tier": format!("{} {}", m.tier.emoji(), m.tier.display()),
                "tierCode": format!("{:?}", m.tier).to_lowercase(),
                "enabled": m.enabled,
                "available": m.available,
                "usable": m.is_usable(),
                "contextWindow": m.context_window,
                "vision": m.supports_vision,
                "thinking": m.supports_thinking,
            })
        })
        .collect();

    let active = registry.active().map(|m| m.id.as_str());

    Ok(json!({
        "models": models,
        "count": models.len(),
        "activeModel": active,
    }).to_string())
}

/// Enable a model.
async fn exec_model_enable(
    args: &Value,
    model_registry: &SharedModelRegistry,
) -> Result<String, String> {
    let model_id = parse_model_id(args)?;

    let mut registry = model_registry.write().await;
    registry.enable(&model_id)?;

    Ok(json!({
        "success": true,
        "modelId": model_id,
        "message": format!("Model '{}' enabled", model_id),
    }).to_string())
}

/// Disable a model.
async fn exec_model_disable(
    args: &Value,
    model_registry: &SharedModelRegistry,
) -> Result<String, String> {
    let model_id = parse_model_id(args)?;

    let mut registry = model_registry.write().await;
    registry.disable(&model_id)?;

    Ok(json!({
        "success": true,
        "modelId": model_id,
        "message": format!("Model '{}' disabled", model_id),
    }).to_string())
}

/// Set the active model.
async fn exec_model_set(
    args: &Value,
    model_registry: &SharedModelRegistry,
) -> Result<String, String> {
    let model_id = parse_model_id(args)?;

    let mut registry = model_registry.write().await;
    
    // Check if model exists and is usable
    {
        let model = registry.get(&model_id)
            .ok_or_else(|| format!("Model not found: {}", model_id))?;
        if !model.is_usable() {
            return Err(format!(
                "Model '{}' is not usable (enabled: {}, available: {})",
                model_id, model.enabled, model.available
            ));
        }
    }
    
    registry.set_active(&model_id)?;

    Ok(json!({
        "success": true,
        "modelId": model_id,
        "message": format!("Active model set to '{}'", model_id),
    }).to_string())
}

/// Get model recommendation for task complexity.
async fn exec_model_recommend(
    args: &Value,
    model_registry: &SharedModelRegistry,
) -> Result<String, String> {
    let complexity_str = args.get("complexity")
        .and_then(|v| v.as_str())
        .unwrap_or("medium");

    let complexity = match complexity_str.to_lowercase().as_str() {
        "simple" => TaskComplexity::Simple,
        "medium" => TaskComplexity::Medium,
        "complex" => TaskComplexity::Complex,
        "critical" => TaskComplexity::Critical,
        _ => return Err(format!("Unknown complexity: {}. Use: simple, medium, complex, critical", complexity_str)),
    };

    let registry = model_registry.read().await;

    let recommended = registry.recommend_for_subagent(complexity);

    match recommended {
        Some(model) => Ok(json!({
            "complexity": complexity_str,
            "recommendedTier": format!("{} {}", complexity.recommended_tier().emoji(), complexity.recommended_tier().display()),
            "model": {
                "id": model.id,
                "displayName": model.display_name,
                "tier": format!("{} {}", model.tier.emoji(), model.tier.display()),
                "provider": model.provider,
            },
            "suggestion": format!(
                "For {} tasks, use '{}' ({})",
                complexity_str,
                model.id,
                model.tier.display()
            ),
        }).to_string()),
        None => Ok(json!({
            "complexity": complexity_str,
            "recommendedTier": format!("{} {}", complexity.recommended_tier().emoji(), complexity.recommended_tier().display()),
            "model": null,
            "error": "No usable model found for this complexity level",
        }).to_string()),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn parse_model_id(args: &Value) -> Result<String, String> {
    args.get("id")
        .or_else(|| args.get("model"))
        .or_else(|| args.get("modelId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing required parameter: id (model ID)".to_string())
}

/// Generate system prompt section for model selection guidance.
pub async fn generate_model_prompt_section(
    model_registry: &SharedModelRegistry,
) -> String {
    let registry = model_registry.read().await;
    crate::models::generate_subagent_guidance(&registry)
}

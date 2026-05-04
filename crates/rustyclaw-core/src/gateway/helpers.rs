use tracing::debug;

use super::protocol::types::ChatMessage;

// ── Context window helpers ──────────────────────────────────────────────────

/// Return the context-window size (in tokens) for a given model name.
/// Conservative defaults — these are *input* token limits.
pub fn context_window_for_model(model: &str) -> usize {
    let m = model.to_lowercase();
    let window =
        if m.contains("claude-opus") || m.contains("claude-sonnet") || m.contains("claude-haiku") {
            200_000
        } else if m.starts_with("gpt-4.1") {
            1_000_000
        } else if m.starts_with("o3") || m.starts_with("o4") {
            200_000
        } else if m.contains("gemini-2.5-pro")
            || m.contains("gemini-2.5-flash")
            || m.contains("gemini-2.0-flash")
        {
            1_000_000
        } else if m.contains("grok-3") {
            131_072
        } else if m.contains("llama") || m.contains("mistral") || m.contains("deepseek") {
            128_000
        } else {
            // Fallback: 128k is a safe default for modern models
            128_000
        };
    debug!(model, window, "Context window for model");
    window
}

/// Fast token estimate: roughly 1 token ≈ 4 characters for English text.
/// This is intentionally conservative (over-estimates) to trigger compaction
/// early rather than hitting the provider's hard limit.
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    let total_chars: usize = messages
        .iter()
        .map(|m| m.role.len() + m.content.len())
        .sum();
    // ~3.5 chars/token for English; we round down to be conservative.
    total_chars / 3
}

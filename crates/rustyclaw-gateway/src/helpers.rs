use tracing::debug;

use rustyclaw_core::gateway::ChatMessage;

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

// ── Persistence ─────────────────────────────────────────────────────────────

/// Persist thread state, logging rather than discarding a failure.
///
/// Every call site used to be `let _ = thread_mgr.save_to_file(path)`. A
/// failed write there is silent *and* invisible: the in-memory thread keeps
/// its messages for the rest of the session, so nothing looks wrong until the
/// gateway restarts and the thread comes back short — or empty. Losing a
/// conversation deserves a log line at minimum.
pub fn persist_threads(
    thread_mgr: &rustyclaw_core::threads::ThreadManager,
    path: &std::path::Path,
) {
    if let Err(e) = thread_mgr.save_to_file(path) {
        tracing::error!(
            path = %path.display(),
            error = %e,
            "Failed to persist thread history — messages added this session \
             will be missing after a restart"
        );
    }
}

/// Persist project state, logging rather than discarding a failure.
pub fn persist_projects(
    project_mgr: &rustyclaw_core::projects::ProjectManager,
    path: &std::path::Path,
) {
    if let Err(e) = project_mgr.save_to_file(path) {
        tracing::error!(
            path = %path.display(),
            error = %e,
            "Failed to persist projects"
        );
    }
}

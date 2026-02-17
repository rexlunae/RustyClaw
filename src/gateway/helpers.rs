use anyhow::{Context, Result};
use serde_json::json;
use std::net::SocketAddr;
use url::Url;

use super::protocol::types::ChatMessage;

// ── Context window helpers ──────────────────────────────────────────────────

/// Return the context-window size (in tokens) for a given model name.
/// Conservative defaults — these are *input* token limits.
pub fn context_window_for_model(model: &str) -> usize {
    let m = model.to_lowercase();
    // Anthropic
    if m.contains("claude-opus") {
        return 200_000;
    }
    if m.contains("claude-sonnet") {
        return 200_000;
    }
    if m.contains("claude-haiku") {
        return 200_000;
    }
    // OpenAI
    if m.starts_with("gpt-4.1") {
        return 1_000_000;
    }
    if m.starts_with("o3") || m.starts_with("o4") {
        return 200_000;
    }
    // Google Gemini
    if m.contains("gemini-2.5-pro") {
        return 1_000_000;
    }
    if m.contains("gemini-2.5-flash") {
        return 1_000_000;
    }
    if m.contains("gemini-2.0-flash") {
        return 1_000_000;
    }
    // xAI
    if m.contains("grok-3") {
        return 131_072;
    }
    // Ollama / unknown — conservative
    if m.contains("llama") {
        return 128_000;
    }
    if m.contains("mistral") {
        return 128_000;
    }
    if m.contains("deepseek") {
        return 128_000;
    }
    // Fallback: 128k is a safe default for modern models
    128_000
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

// ── Address resolution ──────────────────────────────────────────────────────

pub fn resolve_listen_addr(listen: &str) -> Result<SocketAddr> {
    let trimmed = listen.trim();
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        let url = Url::parse(trimmed).context("Invalid WebSocket URL")?;
        let host = url.host_str().context("WebSocket URL missing host")?;
        let port = url
            .port_or_known_default()
            .context("WebSocket URL missing port")?;
        let addr = format!("{}:{}", host, port);
        return addr
            .parse()
            .with_context(|| format!("Invalid listen address {}", addr));
    }

    trimmed
        .parse()
        .with_context(|| format!("Invalid listen address {}", trimmed))
}

// ── Status reporting ─────────────────────────────────────────────────────────

/// Build a JSON status frame to push to connected clients.
///
/// Status frames use `{ "type": "status", "status": "…", "detail": "…" }`.
/// The TUI uses these to update the gateway badge and display progress.
pub fn status_frame(status: &str, detail: &str) -> String {
    json!({
        "type": "status",
        "status": status,
        "detail": detail,
    })
    .to_string()
}

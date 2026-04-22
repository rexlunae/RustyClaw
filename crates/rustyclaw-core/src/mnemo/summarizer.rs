//! Summarization backends for memory compaction.

use super::traits::{MemoryEntry, Summarizer, SummaryKind};
use anyhow::Result;
use async_trait::async_trait;

/// LLM-backed summarizer using provider infrastructure.
pub struct LlmSummarizer {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl LlmSummarizer {
    pub fn new(base_url: String, model: String, api_key: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            model,
            api_key,
        }
    }

    /// Set the API key for authentication.
    pub fn with_api_key(mut self, key: String) -> Self {
        self.api_key = Some(key);
        self
    }

    async fn call_llm(&self, prompt: &str) -> Result<String> {
        let messages = vec![
            serde_json::json!({
                "role": "system",
                "content": "You are a concise summarizer. Preserve key facts, decisions, and context. Be brief but complete."
            }),
            serde_json::json!({
                "role": "user",
                "content": prompt
            }),
        ];

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": 500,
            "temperature": 0.3,
        });

        let mut request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);

        if let Some(ref key) = self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request.send().await?;
        let json: serde_json::Value = response.json().await?;

        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }
}

#[async_trait]
impl Summarizer for LlmSummarizer {
    fn name(&self) -> &str {
        "llm"
    }

    async fn summarize(&self, entries: &[MemoryEntry], kind: SummaryKind) -> Result<String> {
        let formatted: Vec<String> = entries
            .iter()
            .map(|e| {
                if e.depth == 0 {
                    format!("- {}: {}", e.role, e.content)
                } else {
                    format!("- summary-d{}: {}", e.depth, e.content)
                }
            })
            .collect();

        let prompt = match kind {
            SummaryKind::Leaf => format!(
                "Summarize these conversation messages into a brief summary (max 150 words). Preserve key facts, decisions, and action items:\n\n{}",
                formatted.join("\n")
            ),
            SummaryKind::Condensed => format!(
                "Condense these summaries into a single meta-summary (max 100 words). Preserve the most important facts:\n\n{}",
                formatted.join("\n")
            ),
        };

        self.call_llm(&prompt).await
    }
}

/// Deterministic fallback summarizer (truncation-based).
pub struct DeterministicSummarizer {
    chars_per_entry: usize,
    max_total_chars: usize,
}

impl DeterministicSummarizer {
    pub fn new(chars_per_entry: usize, max_total_chars: usize) -> Self {
        Self {
            chars_per_entry,
            max_total_chars,
        }
    }

    fn truncate(&self, text: &str, max: usize) -> String {
        if text.len() <= max {
            text.to_string()
        } else {
            format!("{}…", &text[..max.saturating_sub(1)])
        }
    }
}

#[async_trait]
impl Summarizer for DeterministicSummarizer {
    fn name(&self) -> &str {
        "deterministic"
    }

    async fn summarize(&self, entries: &[MemoryEntry], _kind: SummaryKind) -> Result<String> {
        let mut parts = Vec::new();
        let mut total = 0;

        for e in entries {
            let truncated = self.truncate(&e.content, self.chars_per_entry);
            let line = if e.depth == 0 {
                format!("- {}: {}", e.role, truncated)
            } else {
                format!("- summary-d{}: {}", e.depth, truncated)
            };
            let line_len = line.len();

            if total + line_len > self.max_total_chars {
                break;
            }

            parts.push(line);
            total += line_len;
        }

        Ok(parts.join("\n"))
    }
}

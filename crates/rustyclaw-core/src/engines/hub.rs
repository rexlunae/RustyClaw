//! Hugging Face Hub model discovery.
//!
//! Lets users find models by keyword instead of having to know an exact
//! repo id: `search_models` queries the public Hub API, and
//! `list_gguf_files` enumerates the GGUF files (quantizations) inside a
//! repo so a specific file can be picked before pulling.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

/// One model repo returned by a Hub search.
#[derive(Debug, Clone)]
pub struct HubModel {
    /// Repo id, e.g. `Qwen/Qwen3-4B-GGUF`.
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
    pub tags: Vec<String>,
}

/// Wire shape of a Hub search entry.  Depending on API era, entries carry
/// `id`, `modelId`, or both — a serde `alias` would reject the "both"
/// case as a duplicate field, so keep them separate and merge.
#[derive(Deserialize)]
struct RawHubModel {
    id: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    tags: Vec<String>,
}

impl RawHubModel {
    fn into_model(self) -> Option<HubModel> {
        Some(HubModel {
            id: self.id.or(self.model_id)?,
            downloads: self.downloads,
            likes: self.likes,
            tags: self.tags,
        })
    }
}

impl HubModel {
    /// Whether the repo carries GGUF files (per its tags).
    pub fn is_gguf(&self) -> bool {
        self.tags.iter().any(|t| t == "gguf")
    }

    /// One-line summary for display in the TUI/CLI.
    pub fn display_line(&self) -> String {
        let mut parts = vec![self.id.clone()];
        parts.push(format!("{} downloads", format_count(self.downloads)));
        if self.likes > 0 {
            parts.push(format!("{} likes", format_count(self.likes)));
        }
        if self.is_gguf() {
            parts.push("gguf".to_string());
        }
        parts.join(" · ")
    }
}

/// One file inside a Hub repo.
#[derive(Debug, Clone, Deserialize)]
pub struct HubFile {
    /// Path within the repo, e.g. `qwen3-4b-Q4_K_M.gguf`.
    pub path: String,
    #[serde(default)]
    pub size: u64,
}

impl HubFile {
    /// One-line summary for display in the TUI/CLI.
    pub fn display_line(&self) -> String {
        if self.size > 0 {
            format!("{} · {}", self.path, format_size(self.size))
        } else {
            self.path.clone()
        }
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1e6)
    } else {
        format!("{:.0} KB", bytes as f64 / 1e3)
    }
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("RustyClaw")
        .build()
        .context("failed to build HTTP client")
}

/// Search the Hugging Face Hub for models matching `query`, most-downloaded
/// first.  `gguf_only` restricts results to repos that ship GGUF files —
/// what the local engines (llama.cpp, Joshua, Ollama-importable) can run.
pub async fn search_models(query: &str, gguf_only: bool, limit: usize) -> Result<Vec<HubModel>> {
    let mut url = format!(
        "https://huggingface.co/api/models?search={}&sort=downloads&direction=-1&limit={}",
        urlencoding::encode(query),
        limit,
    );
    if gguf_only {
        url.push_str("&filter=gguf");
    }

    let resp = http_client()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {} failed", url))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Hugging Face Hub search returned HTTP {}: {}",
            status,
            crate::providers::truncate_for_error(&body)
        ));
    }
    let raw: Vec<RawHubModel> = resp
        .json()
        .await
        .context("failed to parse Hub search response")?;
    Ok(raw
        .into_iter()
        .filter_map(RawHubModel::into_model)
        .collect())
}

/// List the GGUF files in a Hub repo (one per quantization, typically),
/// largest metadata first as returned by the API.
pub async fn list_gguf_files(repo: &str) -> Result<Vec<HubFile>> {
    let url = format!(
        "https://huggingface.co/api/models/{}/tree/main?recursive=true",
        repo.trim_matches('/'),
    );
    let resp = http_client()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {} failed", url))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Hugging Face Hub returned HTTP {} for '{}': {}",
            status,
            repo,
            crate::providers::truncate_for_error(&body)
        ));
    }

    #[derive(Deserialize)]
    struct TreeEntry {
        #[serde(rename = "type")]
        kind: String,
        path: String,
        #[serde(default)]
        size: u64,
    }

    let entries: Vec<TreeEntry> = resp
        .json()
        .await
        .context("failed to parse Hub file-tree response")?;
    let mut files: Vec<HubFile> = entries
        .into_iter()
        .filter(|e| e.kind == "file" && e.path.to_lowercase().ends_with(".gguf"))
        .map(|e| HubFile {
            path: e.path,
            size: e.size,
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_model_display_line() {
        let m = HubModel {
            id: "Qwen/Qwen3-4B-GGUF".into(),
            downloads: 1_234_567,
            likes: 890,
            tags: vec!["gguf".into(), "text-generation".into()],
        };
        assert!(m.is_gguf());
        let line = m.display_line();
        assert!(line.contains("Qwen/Qwen3-4B-GGUF"));
        assert!(line.contains("1.2M downloads"));
        assert!(line.contains("gguf"));
    }

    #[test]
    fn hub_file_display_line_includes_size() {
        let f = HubFile {
            path: "model-Q4_K_M.gguf".into(),
            size: 2_500_000_000,
        };
        assert_eq!(f.display_line(), "model-Q4_K_M.gguf · 2.5 GB");
    }

    #[test]
    fn raw_hub_model_merges_id_variants() {
        // Legacy shape: only modelId.
        let m: RawHubModel =
            serde_json::from_str(r#"{"modelId":"a/b","downloads":5,"likes":1,"tags":[]}"#).unwrap();
        assert_eq!(m.into_model().unwrap().id, "a/b");

        // Current shape: only id.
        let m: RawHubModel = serde_json::from_str(r#"{"id":"c/d"}"#).unwrap();
        assert_eq!(m.into_model().unwrap().id, "c/d");

        // Both present (real Hub responses do this) — id wins, no error.
        let m: RawHubModel = serde_json::from_str(r#"{"id":"c/d","modelId":"c/d"}"#).unwrap();
        assert_eq!(m.into_model().unwrap().id, "c/d");

        // Neither — dropped rather than erroring the whole page.
        let m: RawHubModel = serde_json::from_str(r#"{"downloads":1}"#).unwrap();
        assert!(m.into_model().is_none());
    }
}

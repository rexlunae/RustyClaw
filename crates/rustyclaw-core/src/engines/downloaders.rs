//! Model-downloader tools, managed like local engines.
//!
//! Engines such as llama.cpp and Joshua pull GGUF models via the Hugging
//! Face CLI, but nothing installed it — pulls just failed when the binary
//! was missing.  This module models downloader tools as install-only
//! [`LocalEngine`] entries so the existing `/engines` panel, install
//! action, and gateway protocol all apply without new plumbing.

use super::*;
use crate::ignore::Ignore;
use std::path::PathBuf;

/// Locate the Hugging Face CLI binary.
///
/// `huggingface_hub` ≥ 0.34 ships the CLI as `hf`; older installs used
/// `huggingface-cli`.  Returns the binary name to invoke, preferring the
/// modern one.
pub async fn hf_cli() -> Option<&'static str> {
    for bin in ["hf", "huggingface-cli"] {
        let found = tokio::process::Command::new("which")
            .arg(bin)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if found {
            return Some(bin);
        }
    }
    None
}

/// Error message shown when a pull needs the Hugging Face CLI.
pub const HF_CLI_MISSING_HINT: &str = "The Hugging Face CLI is not installed. Run /engines install huggingface \
     (or: pip install -U 'huggingface_hub[cli]').";

/// The Hugging Face CLI as a managed downloader tool.
///
/// Not an inference server: it can be installed, it can pull any Hugging
/// Face repo into the shared HF cache, and it can list/remove cached
/// models — but it cannot start, stop, load, or unload anything.
pub struct HuggingFaceDownloader;

impl HuggingFaceDownloader {
    /// Root of the Hugging Face hub cache (`$HF_HOME/hub` or
    /// `~/.cache/huggingface/hub`).
    fn cache_dir() -> PathBuf {
        if let Ok(home) = std::env::var("HF_HOME") {
            return PathBuf::from(home).join("hub");
        }
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("huggingface")
            .join("hub")
    }

    /// Cache directory name for a repo id ("Org/Name" → "models--Org--Name").
    fn cache_entry_name(repo: &str) -> String {
        format!("models--{}", repo.replace('/', "--"))
    }

    /// Repo id from a cache directory name (inverse of [`cache_entry_name`]).
    fn repo_from_entry(entry: &str) -> Option<String> {
        entry
            .strip_prefix("models--")
            .map(|rest| rest.replace("--", "/"))
    }

    fn dir_size(path: &std::path::Path) -> u64 {
        let mut total = 0;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    total += Self::dir_size(&p);
                } else if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
        }
        total
    }
}

#[async_trait::async_trait]
impl LocalEngine for HuggingFaceDownloader {
    fn id(&self) -> &str {
        "huggingface"
    }

    fn display_name(&self) -> &str {
        "Hugging Face CLI (model downloader)"
    }

    fn default_endpoint(&self) -> &str {
        "https://huggingface.co"
    }

    async fn detect(&self) -> EnginePresence {
        let Some(bin) = hf_cli().await else {
            return EnginePresence::default();
        };
        let sh = |script: String| async move {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(script)
                .output()
                .await
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        };
        let version = sh(format!(
            "{bin} version 2>/dev/null || {bin} --version 2>&1 | head -1"
        ))
        .await;
        let binary_path = sh(format!("which {bin}")).await;
        EnginePresence {
            installed: true,
            version: version.filter(|v| !v.is_empty()),
            binary_path,
        }
    }

    async fn status(&self, _cfg: &EngineConfig) -> EngineStatus {
        // A CLI tool, not a server — it is never "running".
        EngineStatus {
            presence: self.detect().await,
            run_status: EngineRunStatus::Stopped,
        }
    }

    async fn install(&self, sink: Option<ProgressSink>) -> Result<String> {
        if hf_cli().await.is_some() {
            return Ok("Hugging Face CLI is already installed.".into());
        }
        // Prefer isolated tool installers, then brew, then plain pip.
        let script = concat!(
            "if command -v uv >/dev/null 2>&1; then uv tool install 'huggingface_hub[cli]' 2>&1; ",
            "elif command -v pipx >/dev/null 2>&1; then pipx install 'huggingface_hub[cli]' 2>&1; ",
            "elif command -v brew >/dev/null 2>&1; then brew install huggingface-cli 2>&1; ",
            "elif command -v pip3 >/dev/null 2>&1; then pip3 install --user -U 'huggingface_hub[cli]' 2>&1; ",
            "else echo 'No installer found — install uv, pipx, brew, or pip3 first.'; exit 1; fi",
        );
        stream_shell(script, "huggingface", sink.as_ref()).await
    }

    async fn start(&self, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("The Hugging Face CLI is a downloader tool, not a server — nothing to start.")
    }

    async fn stop(&self, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("The Hugging Face CLI is a downloader tool, not a server — nothing to stop.")
    }

    /// List models present in the shared Hugging Face cache.
    async fn list_models(&self, _cfg: &EngineConfig) -> Result<Vec<LocalModel>> {
        let cache = Self::cache_dir();
        let mut models = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&cache) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let Some(repo) = Self::repo_from_entry(&name) else {
                    continue;
                };
                let size_bytes = Self::dir_size(&entry.path());
                let modified_at = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs().to_string());
                models.push(LocalModel {
                    name: repo,
                    size_bytes,
                    quantization: None,
                    context_length: None,
                    loaded: false,
                    vram_bytes: None,
                    family: None,
                    format: None,
                    modified_at,
                });
            }
        }
        models.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(models)
    }

    /// Download a Hugging Face repo into the shared cache.
    ///
    /// `model` doesn't have to be an exact repo id — fuzzy names are
    /// resolved by searching the Hub first (see [`hub::resolve_for_pull`]).
    async fn pull(
        &self,
        model: &str,
        _cfg: &EngineConfig,
        sink: Option<ProgressSink>,
    ) -> Result<String> {
        let Some(bin) = hf_cli().await else {
            anyhow::bail!("{}", HF_CLI_MISSING_HINT);
        };
        let (repo, note) = super::hub::resolve_for_pull(model, false).await?;
        if let (Some(sink), Some(note)) = (sink.as_ref(), note) {
            sink.send(PullProgress {
                model: repo.clone(),
                status: note,
                percent: 0.0,
                downloaded_bytes: 0,
                total_bytes: 0,
            })
            .await
            .ignore();
        }
        stream_shell(
            &format!("{} download {} 2>&1", bin, sh_quote(&repo)),
            &repo,
            sink.as_ref(),
        )
        .await
    }

    /// Remove a repo from the shared cache.
    async fn remove(&self, model: &str, _cfg: &EngineConfig) -> Result<String> {
        let entry = Self::cache_dir().join(Self::cache_entry_name(model));
        if !entry.exists() {
            anyhow::bail!("'{}' is not in the Hugging Face cache.", model);
        }
        std::fs::remove_dir_all(&entry)?;
        Ok(format!("Removed {} from the Hugging Face cache.", model))
    }

    async fn load(&self, _model: &str, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("The Hugging Face CLI cannot load models — use an inference engine.")
    }

    async fn unload(&self, _model: &str, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("The Hugging Face CLI cannot unload models — use an inference engine.")
    }

    fn capabilities(&self) -> EngineCaps {
        EngineCaps {
            can_install: true,
            can_start: false,
            can_stop: false,
            can_pull: true,
            can_remove: true,
            can_load: false,
            can_unload: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_entry_round_trip() {
        let entry = HuggingFaceDownloader::cache_entry_name("Qwen/Qwen3-4B-GGUF");
        assert_eq!(entry, "models--Qwen--Qwen3-4B-GGUF");
        assert_eq!(
            HuggingFaceDownloader::repo_from_entry(&entry).as_deref(),
            Some("Qwen/Qwen3-4B-GGUF"),
        );
        assert_eq!(HuggingFaceDownloader::repo_from_entry("datasets--x"), None);
    }

    #[test]
    fn downloader_capabilities_are_install_and_pull_only() {
        let caps = HuggingFaceDownloader.capabilities();
        assert!(caps.can_install);
        assert!(caps.can_pull);
        assert!(caps.can_remove);
        assert!(!caps.can_start);
        assert!(!caps.can_stop);
        assert!(!caps.can_load);
        assert!(!caps.can_unload);
    }
}

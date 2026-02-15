use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── ClawHub constants ───────────────────────────────────────────────────────

/// Default ClawHub registry URL.
pub const DEFAULT_REGISTRY_URL: &str = "https://clawhub.ai";

// ── Skill types ─────────────────────────────────────────────────────────────

/// Where a skill was installed from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[derive(Default)]
pub enum SkillSource {
    /// Locally authored (found on disk, not from a registry).
    #[default]
    Local,
    /// Installed from a ClawHub registry.
    Registry {
        /// The registry URL it was fetched from.
        registry_url: String,
        /// The version that is currently installed (semver tag or `latest`).
        version: String,
    },
}


/// Represents a skill that can be loaded and executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: Option<String>,
    pub path: PathBuf,
    pub enabled: bool,
    /// Raw instructions from SKILL.md (after frontmatter)
    #[serde(default)]
    pub instructions: String,
    /// Parsed metadata from frontmatter
    #[serde(default)]
    pub metadata: SkillMetadata,
    /// Where this skill was installed from.
    #[serde(default)]
    pub source: SkillSource,
    /// Secrets linked to this skill (vault key names).
    /// When the skill is the active context, `SkillOnly` credentials
    /// whose allowed-list includes this skill's name are accessible.
    #[serde(default)]
    pub linked_secrets: Vec<String>,
}

/// OpenClaw-compatible skill metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Always include this skill (skip gating)
    #[serde(default)]
    pub always: bool,
    /// Optional emoji for UI
    pub emoji: Option<String>,
    /// Homepage URL
    pub homepage: Option<String>,
    /// Required OS platforms (darwin, linux, win32)
    #[serde(default)]
    pub os: Vec<String>,
    /// Gating requirements
    #[serde(default)]
    pub requires: SkillRequirements,
    /// Primary env var for API key
    #[serde(rename = "primaryEnv")]
    pub primary_env: Option<String>,
}

/// Skill gating requirements
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillRequirements {
    /// All these binaries must exist on PATH
    #[serde(default)]
    pub bins: Vec<String>,
    /// At least one of these binaries must exist
    #[serde(rename = "anyBins", default)]
    pub any_bins: Vec<String>,
    /// All these env vars must be set
    #[serde(default)]
    pub env: Vec<String>,
    /// All these config paths must be truthy
    #[serde(default)]
    pub config: Vec<String>,
}

/// Result of checking skill requirements
#[derive(Debug, Clone)]
pub struct GateCheckResult {
    pub passed: bool,
    pub missing_bins: Vec<String>,
    pub missing_env: Vec<String>,
    pub missing_config: Vec<String>,
    pub wrong_os: bool,
}

// ── ClawHub registry types ──────────────────────────────────────────────────

/// Manifest used when publishing a skill to ClawHub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Skill name (must be unique within the registry namespace).
    pub name: String,
    /// Semver version string.
    pub version: String,
    /// Human-readable description.
    pub description: String,
    /// Author / maintainer.
    #[serde(default)]
    pub author: String,
    /// SPDX licence identifier.
    #[serde(default)]
    pub license: String,
    /// Repository URL (source code).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Names of secrets this skill needs (informational; the user still
    /// controls which vault entries to link).
    #[serde(default)]
    pub required_secrets: Vec<String>,
    /// Gating metadata.
    #[serde(default)]
    pub metadata: SkillMetadata,
}

/// A single entry returned by a registry search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Skill slug (used for installation)
    #[serde(alias = "slug")]
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// Description text
    #[serde(alias = "summary")]
    pub description: String,
    /// Display name (optional)
    #[serde(rename = "displayName", default)]
    pub display_name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub required_secrets: Vec<String>,
}

/// Response wrapper from the ClawHub API.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistrySearchResponse {
    /// ClawHub uses "results" array
    #[serde(default)]
    results: Vec<RegistryEntry>,
    /// Legacy field (keep for compatibility)
    #[serde(default)]
    skills: Vec<RegistryEntry>,
    #[serde(default)]
    total: usize,
}

// ── Skill manager ───────────────────────────────────────────────────────────

/// Manages skills compatible with OpenClaw
pub struct SkillManager {
    skills_dirs: Vec<PathBuf>,
    skills: Vec<Skill>,
    /// Environment variables to check against
    env_vars: HashMap<String, String>,
    /// ClawHub registry URL (overridable via config).
    registry_url: String,
    /// ClawHub auth token (optional; needed for publish / private skills).
    registry_token: Option<String>,
}

impl SkillManager {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dirs: vec![skills_dir],
            skills: Vec::new(),
            env_vars: std::env::vars().collect(),
            registry_url: DEFAULT_REGISTRY_URL.to_string(),
            registry_token: None,
        }
    }

    /// Create with multiple skill directories (for precedence)
    pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        Self {
            skills_dirs: dirs,
            skills: Vec::new(),
            env_vars: std::env::vars().collect(),
            registry_url: DEFAULT_REGISTRY_URL.to_string(),
            registry_token: None,
        }
    }

    /// Configure the ClawHub registry URL and optional auth token.
    pub fn set_registry(&mut self, url: &str, token: Option<String>) {
        self.registry_url = url.to_string();
        self.registry_token = token;
    }

    /// Get the primary skills directory (last in the list — user's writable dir).
    /// Skills are loaded from first to last, with later dirs overriding earlier ones.
    /// Installation goes to the last dir (user-writable, highest priority).
    pub fn primary_skills_dir(&self) -> Option<&Path> {
        self.skills_dirs.last().map(|p| p.as_path())
    }

    /// Load skills from all configured directories
    /// Later directories have higher precedence (override earlier ones by name)
    pub fn load_skills(&mut self) -> Result<()> {
        self.skills.clear();
        let mut seen_names: HashMap<String, usize> = HashMap::new();

        for dir in &self.skills_dirs.clone() {
            if !dir.exists() {
                continue;
            }

            // Look for skill directories containing SKILL.md
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    let skill_file = path.join("SKILL.md");
                    if skill_file.exists() {
                        if let Ok(skill) = self.load_skill_md(&skill_file) {
                            // Check if we already have this skill (override by precedence)
                            if let Some(&idx) = seen_names.get(&skill.name) {
                                self.skills[idx] = skill.clone();
                            } else {
                                seen_names.insert(skill.name.clone(), self.skills.len());
                                self.skills.push(skill);
                            }
                        }
                    }
                }

                // Also support legacy .skill/.json/.yaml files
                if path.is_file() {
                    if let Some(ext) = path.extension() {
                        if ext == "skill" || ext == "json" || ext == "yaml" || ext == "yml" {
                            if let Ok(skill) = self.load_skill_legacy(&path) {
                                if let Some(&idx) = seen_names.get(&skill.name) {
                                    self.skills[idx] = skill.clone();
                                } else {
                                    seen_names.insert(skill.name.clone(), self.skills.len());
                                    self.skills.push(skill);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Load a skill from SKILL.md format (AgentSkills compatible)
    fn load_skill_md(&self, path: &Path) -> Result<Skill> {
        let content = std::fs::read_to_string(path)?;
        let (frontmatter, instructions) = parse_frontmatter(&content)?;

        // Parse frontmatter as YAML
        let name = frontmatter
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Skill missing 'name' in frontmatter"))?
            .to_string();

        let description = frontmatter
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Parse metadata if present
        let metadata = if let Some(meta_val) = frontmatter.get("metadata") {
            // metadata can be a string (JSON) or an object
            if let Some(meta_str) = meta_val.as_str() {
                serde_json::from_str(meta_str).unwrap_or_default()
            } else if let Some(openclaw) = meta_val.get("openclaw") {
                // Convert YAML Value to JSON Value via serialization round-trip
                let json_str = serde_json::to_string(&openclaw).unwrap_or_default();
                serde_json::from_str(&json_str).unwrap_or_default()
            } else {
                SkillMetadata::default()
            }
        } else {
            SkillMetadata::default()
        };

        // Replace {baseDir} placeholder in instructions
        let base_dir = path.parent().unwrap_or(Path::new("."));
        let instructions = instructions.replace("{baseDir}", &base_dir.display().to_string());

        // Extract linked_secrets from frontmatter if present.
        let linked_secrets: Vec<String> = frontmatter
            .get("linked_secrets")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(Skill {
            name,
            description,
            path: path.to_path_buf(),
            enabled: true,
            instructions,
            metadata,
            source: SkillSource::Local,
            linked_secrets,
        })
    }

    /// Load a legacy skill file (.skill/.json/.yaml)
    fn load_skill_legacy(&self, path: &Path) -> Result<Skill> {
        let is_json = path.extension().is_some_and(|e| e == "json" || e == "skill");
        let is_yaml = path.extension().is_some_and(|e| e == "yaml" || e == "yml");

        if !is_json && !is_yaml {
            anyhow::bail!("Unsupported skill file format: {:?}", path);
        }

        let content = std::fs::read_to_string(path)?;

        let skill: Skill = if is_yaml {
            serde_yaml::from_str(&content)?
        } else {
            serde_json::from_str(&content)?
        };

        Ok(skill)
    }

    /// Check if a skill passes its gating requirements
    pub fn check_gates(&self, skill: &Skill) -> GateCheckResult {
        let mut result = GateCheckResult {
            passed: true,
            missing_bins: Vec::new(),
            missing_env: Vec::new(),
            missing_config: Vec::new(),
            wrong_os: false,
        };

        // Always-enabled skills skip all gates
        if skill.metadata.always {
            return result;
        }

        // Check OS requirement
        if !skill.metadata.os.is_empty() {
            let current_os = if cfg!(target_os = "macos") {
                "darwin"
            } else if cfg!(target_os = "linux") {
                "linux"
            } else if cfg!(target_os = "windows") {
                "win32"
            } else {
                "unknown"
            };

            if !skill.metadata.os.iter().any(|os| os == current_os) {
                result.wrong_os = true;
                result.passed = false;
            }
        }

        // Check required binaries
        for bin in &skill.metadata.requires.bins {
            if !self.binary_exists(bin) {
                result.missing_bins.push(bin.clone());
                result.passed = false;
            }
        }

        // Check anyBins (at least one must exist)
        if !skill.metadata.requires.any_bins.is_empty() {
            let any_found = skill
                .metadata
                .requires
                .any_bins
                .iter()
                .any(|bin| self.binary_exists(bin));
            if !any_found {
                result.missing_bins.extend(skill.metadata.requires.any_bins.clone());
                result.passed = false;
            }
        }

        // Check required env vars
        for env_var in &skill.metadata.requires.env {
            if !self.env_vars.contains_key(env_var) {
                result.missing_env.push(env_var.clone());
                result.passed = false;
            }
        }

        // Config checks would require access to config - mark as missing for now
        // In a real implementation, this would check openclaw.json
        result.missing_config = skill.metadata.requires.config.clone();
        if !result.missing_config.is_empty() {
            // Don't fail on config checks for now - they require config integration
        }

        result
    }

    /// Check if a binary exists on PATH
    fn binary_exists(&self, name: &str) -> bool {
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join(name);
                if candidate.exists() {
                    return true;
                }
                // On Windows, also check with .exe
                #[cfg(windows)]
                {
                    let candidate_exe = dir.join(format!("{}.exe", name));
                    if candidate_exe.exists() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Get all loaded skills
    pub fn get_skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Get only enabled skills that pass gating
    pub fn get_eligible_skills(&self) -> Vec<&Skill> {
        self.skills
            .iter()
            .filter(|s| s.enabled && self.check_gates(s).passed)
            .collect()
    }

    /// Get a specific skill by name
    pub fn get_skill(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// Enable or disable a skill
    pub fn set_skill_enabled(&mut self, name: &str, enabled: bool) -> Result<()> {
        if let Some(skill) = self.skills.iter_mut().find(|s| s.name == name) {
            skill.enabled = enabled;
            Ok(())
        } else {
            anyhow::bail!("Skill not found: {}", name)
        }
    }

    /// Generate prompt context for all eligible skills
    pub fn generate_prompt_context(&self) -> String {
        let eligible = self.get_eligible_skills();

        let mut context = String::from("## Skills (mandatory)\n\n");
        context.push_str("Before replying: scan <available_skills> <description> entries.\n");
        context.push_str("- If exactly one skill clearly applies: read its SKILL.md at <location> with `read_file`, then follow it.\n");
        context.push_str("- If multiple could apply: choose the most specific one, then read/follow it.\n");
        context.push_str("- If none clearly apply: do not read any SKILL.md.\n");
        context.push_str("Constraints: never read more than one skill up front; only read after selecting.\n\n");
        context.push_str("The following skills provide specialized instructions for specific tasks.\n");
        context.push_str("Use the read_file tool to load a skill's file when the task matches its description.\n");
        context.push_str("When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.\n\n");

        if eligible.is_empty() {
            context.push_str("No skills are currently loaded.\n\n");
            context.push_str("To find and install skills:\n");
            context.push_str("- Browse: https://clawhub.com\n");
            context.push_str("- Install: `npm i -g clawhub && clawhub install <skill-name>`\n");
            return context;
        }

        context.push_str("<available_skills>\n");

        for skill in eligible {
            context.push_str("  <skill>\n");
            context.push_str(&format!("    <name>{}</name>\n", skill.name));
            if let Some(ref desc) = skill.description {
                context.push_str(&format!("    <description>{}</description>\n", desc));
            }
            context.push_str(&format!("    <location>{}</location>\n", skill.path.display()));
            context.push_str("  </skill>\n");
        }

        context.push_str("</available_skills>\n\n");

        // Add note about ClawHub for finding more skills
        context.push_str("To find more skills: https://clawhub.com\n");
        context.push_str("To install a skill: `clawhub install <skill-name>` (requires npm i -g clawhub)\n");

        context
    }

    /// Get full instructions for a skill (for when agent reads SKILL.md)
    pub fn get_skill_instructions(&self, name: &str) -> Option<String> {
        self.get_skill(name).map(|s| s.instructions.clone())
    }

    // ── Secret linking ──────────────────────────────────────────────

    /// Link a vault credential to a skill so the skill can access it
    /// via the `SkillOnly` policy.
    pub fn link_secret(&mut self, skill_name: &str, secret_name: &str) -> Result<()> {
        let skill = self
            .skills
            .iter_mut()
            .find(|s| s.name == skill_name)
            .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", skill_name))?;

        if !skill.linked_secrets.contains(&secret_name.to_string()) {
            skill.linked_secrets.push(secret_name.to_string());
        }
        Ok(())
    }

    /// Unlink a vault credential from a skill.
    pub fn unlink_secret(&mut self, skill_name: &str, secret_name: &str) -> Result<()> {
        let skill = self
            .skills
            .iter_mut()
            .find(|s| s.name == skill_name)
            .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", skill_name))?;

        skill.linked_secrets.retain(|s| s != secret_name);
        Ok(())
    }

    /// Return the linked secrets for a skill (empty vec if not found).
    pub fn get_linked_secrets(&self, skill_name: &str) -> Vec<String> {
        self.get_skill(skill_name)
            .map(|s| s.linked_secrets.clone())
            .unwrap_or_default()
    }

    // ── Skill removal ───────────────────────────────────────────────

    /// Remove a skill by name.  If it was installed from a registry,
    /// its directory is deleted from disk.
    pub fn remove_skill(&mut self, name: &str) -> Result<()> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", name))?;

        let skill = self.skills.remove(idx);

        // If the skill lives inside one of our managed skill directories,
        // remove it from disk.
        if let Some(parent) = skill.path.parent() {
            for dir in &self.skills_dirs {
                if parent.starts_with(dir) || parent == dir.as_path() {
                    if parent.is_dir() {
                        let _ = std::fs::remove_dir_all(parent);
                    }
                    break;
                }
            }
        }

        Ok(())
    }

    // ── Detailed info ───────────────────────────────────────────────

    /// Return a human-readable summary of a skill.
    pub fn skill_info(&self, name: &str) -> Option<String> {
        let skill = self.get_skill(name)?;
        let gate = self.check_gates(skill);
        let mut out = String::new();
        out.push_str(&format!("Skill: {}\n", skill.name));
        if let Some(ref desc) = skill.description {
            out.push_str(&format!("Description: {}\n", desc));
        }
        out.push_str(&format!("Enabled: {}\n", skill.enabled));
        out.push_str(&format!("Gates passed: {}\n", gate.passed));
        out.push_str(&format!("Path: {}\n", skill.path.display()));
        match &skill.source {
            SkillSource::Local => out.push_str("Source: local\n"),
            SkillSource::Registry { registry_url, version } => {
                out.push_str(&format!("Source: registry ({}@{})\n", registry_url, version));
            }
        }
        if !skill.linked_secrets.is_empty() {
            out.push_str(&format!("Linked secrets: {}\n", skill.linked_secrets.join(", ")));
        }
        if !gate.missing_bins.is_empty() {
            out.push_str(&format!("Missing binaries: {}\n", gate.missing_bins.join(", ")));
        }
        if !gate.missing_env.is_empty() {
            out.push_str(&format!("Missing env vars: {}\n", gate.missing_env.join(", ")));
        }
        Some(out)
    }

    // ── ClawHub registry operations ─────────────────────────────────

    /// Try to reach the registry with a short timeout.  Returns `true`
    /// if the base URL responds, `false` on any network error.
    fn registry_reachable(&self) -> bool {
        let client = reqwest::blocking::Client::new();
        client
            .head(&self.registry_url)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .is_ok()
    }

    /// Search the ClawHub registry for skills matching a query.
    ///
    /// If the registry is unreachable, falls back to matching against
    /// locally-loaded skills so the user still gets useful results.
    pub fn search_registry(&self, query: &str) -> Result<Vec<RegistryEntry>> {
        // ── Try remote registry first ───────────────────────────
        match self.search_registry_remote(query) {
            Ok(results) => return Ok(results),
            Err(_) => {
                // Fall through to local search.
            }
        }

        // ── Fallback: search locally loaded skills ──────────────
        let q_lower = query.to_lowercase();
        let local_results: Vec<RegistryEntry> = self
            .skills
            .iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&q_lower)
                    || s.description
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&q_lower)
            })
            .map(|s| RegistryEntry {
                name: s.name.clone(),
                display_name: String::new(),
                version: match &s.source {
                    SkillSource::Registry { version, .. } => version.clone(),
                    SkillSource::Local => "local".to_string(),
                },
                description: s.description.clone().unwrap_or_default(),
                author: String::new(),
                downloads: 0,
                required_secrets: s.linked_secrets.clone(),
            })
            .collect();

        Ok(local_results)
    }

    /// Internal: attempt a remote registry search.
    fn search_registry_remote(&self, query: &str) -> Result<Vec<RegistryEntry>> {
        // ClawHub API: /api/search?q=<query>
        let url = format!(
            "{}/api/search?q={}",
            self.registry_url,
            urlencoding::encode(query),
        );

        let client = reqwest::blocking::Client::new();
        let mut req = client.get(&url);
        if let Some(ref token) = self.registry_token {
            req = req.bearer_auth(token);
        }

        let resp = req
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .context("ClawHub registry is not reachable")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "ClawHub search failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        let body: RegistrySearchResponse = resp.json().context("Failed to parse registry response")?;

        // ClawHub returns "results", legacy might return "skills"
        let entries = if !body.results.is_empty() {
            body.results
        } else {
            body.skills
        };

        Ok(entries)
    }

    /// Install a skill from the ClawHub registry into the primary
    /// skills directory.  Returns the installed `Skill`.
    pub fn install_from_registry(&mut self, name: &str, version: Option<&str>) -> Result<Skill> {
        if !self.registry_reachable() {
            anyhow::bail!(
                "ClawHub registry ({}) is not reachable. \
                 Check your internet connection or set a custom registry URL \
                 with `clawhub_url` in your config.",
                self.registry_url,
            );
        }

        // ClawHub download API: /api/v1/download?slug=<name>&version=<version>
        let mut url = format!("{}/api/v1/download?slug={}", self.registry_url, urlencoding::encode(name));
        if let Some(v) = version {
            url.push_str(&format!("&version={}", urlencoding::encode(v)));
        }

        let client = reqwest::blocking::Client::new();
        let mut req = client.get(&url);
        if let Some(ref token) = self.registry_token {
            req = req.bearer_auth(token);
        }

        let resp = req
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .context("Failed to download skill from ClawHub")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "ClawHub install failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        // Response is a zip file
        let zip_bytes = resp.bytes().context("Failed to read zip data")?;

        // Use last directory (user's writable dir) for installations, not first (bundled/read-only)
        let skills_dir = self
            .skills_dirs
            .last()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No skills directory configured"))?;

        let skill_dir = skills_dir.join(name);
        std::fs::create_dir_all(&skill_dir)?;

        // Extract zip to skill directory
        let cursor = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(cursor).context("Invalid zip archive")?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let outpath = skill_dir.join(file.name());

            if file.name().ends_with('/') {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut outfile = std::fs::File::create(&outpath)?;
                std::io::copy(&mut file, &mut outfile)?;
            }
        }

        // Write .clawhub metadata
        let clawhub_dir = skill_dir.join(".clawhub");
        std::fs::create_dir_all(&clawhub_dir)?;
        let meta = serde_json::json!({
            "version": 1,
            "registry": self.registry_url,
            "slug": name,
            "installedVersion": version.unwrap_or("latest"),
            "installedAt": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        });
        std::fs::write(clawhub_dir.join("install.json"), serde_json::to_string_pretty(&meta)?)?;

        // Load the newly-installed skill.
        let skill_md_path = skill_dir.join("SKILL.md");
        let mut skill = self.load_skill_md(&skill_md_path)?;
        skill.source = SkillSource::Registry {
            registry_url: self.registry_url.clone(),
            version: version.unwrap_or("latest").to_string(),
        };

        // Add or replace in the in-memory list.
        if let Some(idx) = self.skills.iter().position(|s| s.name == skill.name) {
            self.skills[idx] = skill.clone();
        } else {
            self.skills.push(skill.clone());
        }

        Ok(skill)
    }

    /// Publish a local skill to the ClawHub registry.
    pub fn publish_to_registry(&self, skill_name: &str) -> Result<String> {
        let skill = self
            .get_skill(skill_name)
            .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", skill_name))?;

        let token = self
            .registry_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ClawHub auth token required for publishing. Set clawhub_token in config."))?;

        // Read the skill content.
        let content = std::fs::read_to_string(&skill.path)
            .context("Failed to read skill file")?;

        let manifest = SkillManifest {
            name: skill.name.clone(),
            version: "0.1.0".to_string(), // TODO: extract from frontmatter
            description: skill.description.clone().unwrap_or_default(),
            author: String::new(),
            license: "MIT".to_string(),
            repository: skill.metadata.homepage.clone(),
            required_secrets: skill.linked_secrets.clone(),
            metadata: skill.metadata.clone(),
        };

        let payload = serde_json::json!({
            "manifest": manifest,
            "skill_md": content,
        });

        if !self.registry_reachable() {
            anyhow::bail!(
                "ClawHub registry ({}) is not reachable. \
                 Check your internet connection or set a custom registry URL \
                 with `clawhub_url` in your config.",
                self.registry_url,
            );
        }

        let url = format!("{}/skills/publish", self.registry_url);
        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(&url)
            .bearer_auth(token)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .context("Failed to publish to ClawHub")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "ClawHub publish failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        Ok(format!(
            "Published {} v{} to {}",
            manifest.name, manifest.version, self.registry_url,
        ))
    }
}

/// Parse YAML frontmatter from a markdown file
fn parse_frontmatter(content: &str) -> Result<(serde_yaml::Value, String)> {
    let content = content.trim_start();

    if !content.starts_with("---") {
        // No frontmatter, treat entire content as instructions
        return Ok((serde_yaml::Value::Mapping(Default::default()), content.to_string()));
    }

    // Find the closing ---
    let after_first = &content[3..];
    if let Some(end_idx) = after_first.find("\n---") {
        let frontmatter_str = &after_first[..end_idx];
        let instructions = after_first[end_idx + 4..].trim_start().to_string();

        let frontmatter: serde_yaml::Value = serde_yaml::from_str(frontmatter_str)
            .context("Failed to parse YAML frontmatter")?;

        Ok((frontmatter, instructions))
    } else {
        // No closing ---, treat as no frontmatter
        Ok((serde_yaml::Value::Mapping(Default::default()), content.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_manager_creation() {
        let temp_dir = std::env::temp_dir().join("rustyclaw_test_skills");
        let manager = SkillManager::new(temp_dir);
        assert_eq!(manager.get_skills().len(), 0);
    }

    #[test]
    fn test_parse_frontmatter_with_yaml() {
        let content = r#"---
name: test-skill
description: A test skill
---

# Instructions

Do the thing.
"#;
        let (fm, instructions) = parse_frontmatter(content).unwrap();
        assert_eq!(fm["name"].as_str(), Some("test-skill"));
        assert_eq!(fm["description"].as_str(), Some("A test skill"));
        assert!(instructions.contains("Do the thing"));
    }

    #[test]
    fn test_parse_frontmatter_without_yaml() {
        let content = "# Just some markdown\n\nNo frontmatter here.";
        let (fm, instructions) = parse_frontmatter(content).unwrap();
        assert!(fm.is_mapping());
        assert!(instructions.contains("Just some markdown"));
    }

    #[test]
    fn test_binary_exists() {
        let manager = SkillManager::new(std::env::temp_dir());
        // 'ls' or 'dir' should exist on most systems
        #[cfg(unix)]
        assert!(manager.binary_exists("ls"));
        #[cfg(windows)]
        assert!(manager.binary_exists("cmd"));
    }

    #[test]
    fn test_gate_check_always() {
        let manager = SkillManager::new(std::env::temp_dir());
        let skill = Skill {
            name: "test".into(),
            description: None,
            path: PathBuf::new(),
            enabled: true,
            instructions: String::new(),
            metadata: SkillMetadata {
                always: true,
                ..Default::default()
            },
            source: SkillSource::Local,
            linked_secrets: vec![],
        };
        let result = manager.check_gates(&skill);
        assert!(result.passed);
    }

    #[test]
    fn test_gate_check_missing_bin() {
        let manager = SkillManager::new(std::env::temp_dir());
        let skill = Skill {
            name: "test".into(),
            description: None,
            path: PathBuf::new(),
            enabled: true,
            instructions: String::new(),
            metadata: SkillMetadata {
                requires: SkillRequirements {
                    bins: vec!["nonexistent_binary_12345".into()],
                    ..Default::default()
                },
                ..Default::default()
            },
            source: SkillSource::Local,
            linked_secrets: vec![],
        };
        let result = manager.check_gates(&skill);
        assert!(!result.passed);
        assert!(result.missing_bins.contains(&"nonexistent_binary_12345".to_string()));
    }

    #[test]
    fn test_generate_prompt_context() {
        let mut manager = SkillManager::new(std::env::temp_dir());
        manager.skills.push(Skill {
            name: "test-skill".into(),
            description: Some("Does testing".into()),
            path: PathBuf::from("/skills/test/SKILL.md"),
            enabled: true,
            instructions: "Test instructions".into(),
            metadata: SkillMetadata::default(),
            source: SkillSource::Local,
            linked_secrets: vec![],
        });
        let context = manager.generate_prompt_context();
        assert!(context.contains("test-skill"));
        assert!(context.contains("Does testing"));
        assert!(context.contains("<available_skills>"));
    }

    #[test]
    fn test_link_and_unlink_secret() {
        let mut manager = SkillManager::new(std::env::temp_dir());
        manager.skills.push(Skill {
            name: "deploy".into(),
            description: Some("Deploy things".into()),
            path: PathBuf::from("/skills/deploy/SKILL.md"),
            enabled: true,
            instructions: String::new(),
            metadata: SkillMetadata::default(),
            source: SkillSource::Local,
            linked_secrets: vec![],
        });

        manager.link_secret("deploy", "AWS_KEY").unwrap();
        manager.link_secret("deploy", "AWS_SECRET").unwrap();
        assert_eq!(manager.get_linked_secrets("deploy"), vec!["AWS_KEY", "AWS_SECRET"]);

        // Linking the same secret again should not duplicate.
        manager.link_secret("deploy", "AWS_KEY").unwrap();
        assert_eq!(manager.get_linked_secrets("deploy").len(), 2);

        manager.unlink_secret("deploy", "AWS_KEY").unwrap();
        assert_eq!(manager.get_linked_secrets("deploy"), vec!["AWS_SECRET"]);
    }

    #[test]
    fn test_link_secret_skill_not_found() {
        let mut manager = SkillManager::new(std::env::temp_dir());
        assert!(manager.link_secret("nonexistent", "key").is_err());
    }

    #[test]
    fn test_skill_info() {
        let mut manager = SkillManager::new(std::env::temp_dir());
        manager.skills.push(Skill {
            name: "web-scrape".into(),
            description: Some("Scrape web pages".into()),
            path: PathBuf::from("/skills/web-scrape/SKILL.md"),
            enabled: true,
            instructions: String::new(),
            metadata: SkillMetadata::default(),
            source: SkillSource::Registry {
                registry_url: "https://registry.clawhub.dev/api/v1".into(),
                version: "1.0.0".into(),
            },
            linked_secrets: vec!["SCRAPER_KEY".into()],
        });

        let info = manager.skill_info("web-scrape").unwrap();
        assert!(info.contains("web-scrape"));
        assert!(info.contains("registry"));
        assert!(info.contains("SCRAPER_KEY"));
        assert!(manager.skill_info("nonexistent").is_none());
    }

    #[test]
    fn test_remove_skill() {
        let mut manager = SkillManager::new(std::env::temp_dir());
        manager.skills.push(Skill {
            name: "temp-skill".into(),
            description: None,
            path: PathBuf::from("/nonexistent/SKILL.md"),
            enabled: true,
            instructions: String::new(),
            metadata: SkillMetadata::default(),
            source: SkillSource::Local,
            linked_secrets: vec![],
        });
        assert_eq!(manager.get_skills().len(), 1);
        manager.remove_skill("temp-skill").unwrap();
        assert_eq!(manager.get_skills().len(), 0);
        assert!(manager.remove_skill("temp-skill").is_err());
    }

    #[test]
    fn test_skill_source_default() {
        assert_eq!(SkillSource::default(), SkillSource::Local);
    }

    #[test]
    fn test_base64_decode() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let encoded = "SGVsbG8=";
        let decoded = STANDARD.decode(encoded).unwrap();
        assert_eq!(decoded, b"Hello");
    }
}

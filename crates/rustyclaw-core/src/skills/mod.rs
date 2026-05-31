use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

// ── Blocking retry helpers (for ClawHub HTTP calls) ─────────────────────────

/// Maximum retry attempts for ClawHub API calls.
const CLAWHUB_MAX_RETRIES: u32 = 4;
/// Base delay for exponential backoff.
const CLAWHUB_BASE_DELAY: Duration = Duration::from_millis(500);
/// Maximum delay cap.
const CLAWHUB_MAX_DELAY: Duration = Duration::from_secs(15);

/// Send a blocking request with retry + exponential backoff for 429 / 5xx.
/// Returns the successful response or the last error.
fn blocking_request_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    token: Option<&str>,
    timeout: Duration,
) -> Result<reqwest::blocking::Response> {
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 1..=CLAWHUB_MAX_RETRIES {
        let mut req = client.get(url);
        if let Some(tok) = token {
            req = req.bearer_auth(tok);
        }

        match req.timeout(timeout).send() {
            Ok(resp) => {
                let status = resp.status();

                // Success — return immediately.
                if status.is_success() {
                    return Ok(resp);
                }

                // Retry on 429 or 5xx — honour Retry-After header if present.
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    let retry_after = resp
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .map(Duration::from_secs);

                    let body = resp.text().unwrap_or_default();

                    let backoff = backoff_delay(attempt);
                    let delay = retry_after.unwrap_or(backoff);

                    last_err = Some(anyhow::anyhow!("HTTP {} from ClawHub: {}", status, body,));

                    if attempt < CLAWHUB_MAX_RETRIES {
                        std::thread::sleep(delay);
                        continue;
                    }

                    // Final attempt — fall through to bail below.
                    anyhow::bail!(
                        "ClawHub request failed (HTTP {}) after {} retries: {}",
                        status,
                        CLAWHUB_MAX_RETRIES,
                        body,
                    );
                }

                // Non-retryable error — bail immediately.
                anyhow::bail!(
                    "ClawHub request failed (HTTP {}): {}",
                    status,
                    resp.text().unwrap_or_default(),
                );
            }
            Err(e) => {
                // Retry on timeout / connect errors.
                if (e.is_timeout() || e.is_connect()) && attempt < CLAWHUB_MAX_RETRIES {
                    let delay = backoff_delay(attempt);
                    last_err = Some(e.into());
                    std::thread::sleep(delay);
                    continue;
                }
                return Err(e.into());
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("ClawHub request failed after retries")))
}

/// Exponential backoff: 500ms → 1s → 2s → 4s … capped at CLAWHUB_MAX_DELAY.
fn backoff_delay(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(31);
    let multiplier = 1u64 << shift;
    let millis = CLAWHUB_BASE_DELAY.as_millis() as u64 * multiplier;
    Duration::from_millis(millis).min(CLAWHUB_MAX_DELAY)
}

// ── ClawHub constants ───────────────────────────────────────────────────────

/// Default ClawHub registry URL.
pub const DEFAULT_REGISTRY_URL: &str = "https://clawhub.ai";

// ── Skill types ─────────────────────────────────────────────────────────────

/// Where a skill was installed from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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
        let is_json = path
            .extension()
            .is_some_and(|e| e == "json" || e == "skill");
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
                result
                    .missing_bins
                    .extend(skill.metadata.requires.any_bins.clone());
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
        // Get all enabled skills (not just those passing gates)
        let enabled_skills: Vec<&Skill> = self.skills.iter().filter(|s| s.enabled).collect();

        let mut context = String::from("## Skills (mandatory)\n\n");
        context.push_str("Before replying: scan <available_skills> <description> entries.\n");
        context.push_str("- If exactly one skill clearly applies: read its SKILL.md at <location> with `read`, then follow it.\n");
        context.push_str(
            "- If multiple could apply: choose the most specific one, then read/follow it.\n",
        );
        context.push_str("- If none clearly apply: do not read any SKILL.md.\n");
        context.push_str(
            "Constraints: never read more than one skill up front; only read after selecting.\n\n",
        );
        context.push_str(
            "The following skills provide specialized instructions for specific tasks.\n",
        );
        context.push_str(
            "Use the read tool to load a skill's file when the task matches its description.\n",
        );
        context.push_str("When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.\n\n");

        if enabled_skills.is_empty() {
            context.push_str("No skills are currently loaded.\n\n");
            context.push_str("To find and install skills:\n");
            context.push_str("- Browse: https://clawhub.com\n");
            context.push_str("- Install: `/skill install <skill-name>` or `rustyclaw clawhub install <skill-name>`\n\n");
            self.append_skill_creation_instructions(&mut context);
            return context;
        }

        context.push_str("<available_skills>\n");

        for skill in enabled_skills {
            let gate_result = self.check_gates(skill);
            let available = gate_result.passed;

            context.push_str("  <skill>\n");
            context.push_str(&format!("    <name>{}</name>\n", skill.name));
            if let Some(ref desc) = skill.description {
                context.push_str(&format!("    <description>{}</description>\n", desc));
            }
            context.push_str(&format!(
                "    <location>{}</location>\n",
                skill.path.display()
            ));

            if available {
                context.push_str("    <available>true</available>\n");
            } else {
                context.push_str("    <available>false</available>\n");

                // Show what's missing
                let mut missing = Vec::new();
                if gate_result.wrong_os {
                    missing.push(format!("OS: requires {:?}", skill.metadata.os));
                }
                if !gate_result.missing_bins.is_empty() {
                    missing.push(format!("bins: {}", gate_result.missing_bins.join(", ")));
                }
                if !gate_result.missing_env.is_empty() {
                    missing.push(format!("env: {}", gate_result.missing_env.join(", ")));
                }
                if !missing.is_empty() {
                    context.push_str(&format!(
                        "    <requires>{}</requires>\n",
                        missing.join("; ")
                    ));
                }
            }

            context.push_str("  </skill>\n");
        }

        context.push_str("</available_skills>\n\n");

        // Add note about ClawHub for finding more skills
        context.push_str("To find more skills: https://clawhub.com\n");
        context.push_str("To install a skill: `/skill install <skill-name>` or `rustyclaw clawhub install <skill-name>`\n\n");

        // Add skill creation instructions so the agent can create skills from conversation
        self.append_skill_creation_instructions(&mut context);

        context
    }

    /// Append instructions that teach the agent how to create new skills.
    fn append_skill_creation_instructions(&self, context: &mut String) {
        context.push_str("## Creating New Skills\n\n");
        context.push_str("When a user asks you to create, author, or scaffold a new skill, use the `skill_create` tool.\n");
        context.push_str(
            "This tool creates the skill directory and SKILL.md file in the correct location.\n\n",
        );
        context.push_str("A skill is a directory containing a `SKILL.md` file with YAML frontmatter and markdown instructions.\n\n");
        context.push_str("<skill_template>\n");
        context.push_str("```\n");
        context.push_str("---\n");
        context.push_str("name: my-skill-name\n");
        context.push_str("description: A concise one-line description of what this skill does\n");
        context.push_str("metadata: {\"openclaw\": {\"emoji\": \"🔧\"}}\n");
        context.push_str("---\n\n");
        context.push_str("# Skill Title\n\n");
        context.push_str(
            "Detailed instructions for the agent to follow when this skill is activated.\n",
        );
        context
            .push_str("Include step-by-step guidance, tool usage patterns, and any constraints.\n");
        context.push_str("```\n");
        context.push_str("</skill_template>\n\n");
        context.push_str("Frontmatter fields:\n");
        context
            .push_str("- `name` (required): kebab-case identifier, used as the directory name\n");
        context
            .push_str("- `description` (required): shown in skill listings, used for matching\n");
        context.push_str("- `metadata` (optional): JSON with gating requirements, e.g.\n");
        context.push_str("  `{\"openclaw\": {\"emoji\": \"⚡\", \"always\": false, \"requires\": {\"bins\": [\"git\", \"node\"]}}}`\n\n");

        if let Some(dir) = self.primary_skills_dir() {
            context.push_str(&format!("Skills directory: {}\n", dir.display()));
        }
    }

    /// Get full instructions for a skill (for when agent reads SKILL.md)
    pub fn get_skill_instructions(&self, name: &str) -> Option<String> {
        self.get_skill(name).map(|s| s.instructions.clone())
    }

    // ── Skill creation ──────────────────────────────────────────────

    /// Create a new skill on disk from name, description, and instructions.
    ///
    /// Writes `<primary_skills_dir>/<name>/SKILL.md` with YAML frontmatter
    /// and the supplied markdown body, then reloads the skill list so the
    /// new skill is immediately available.
    pub fn create_skill(
        &mut self,
        name: &str,
        description: &str,
        instructions: &str,
        metadata_json: Option<&str>,
    ) -> Result<PathBuf> {
        // Validate name is kebab-case-ish (no slashes, no spaces, no dots-leading)
        if name.is_empty() {
            anyhow::bail!("Skill name cannot be empty");
        }
        if name.contains('/') || name.contains('\\') || name.contains(' ') {
            anyhow::bail!("Skill name must be a simple identifier (no slashes or spaces): {name}");
        }

        let skills_dir = self
            .primary_skills_dir()
            .ok_or_else(|| anyhow::anyhow!("No skills directory configured"))?
            .to_path_buf();

        let skill_dir = skills_dir.join(name);
        if skill_dir.join("SKILL.md").exists() {
            anyhow::bail!("Skill already exists: {name} (at {})", skill_dir.display());
        }

        std::fs::create_dir_all(&skill_dir)?;

        // Build frontmatter
        let mut fm = format!("---\nname: {name}\ndescription: {description}\n");
        if let Some(meta) = metadata_json {
            fm.push_str(&format!("metadata: {meta}\n"));
        }
        fm.push_str("---\n\n");

        let content = format!("{fm}{instructions}\n");
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_path, &content)?;

        // Reload so the new skill is immediately visible
        self.load_skills()?;

        Ok(skill_path)
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
            SkillSource::Registry {
                registry_url,
                version,
            } => {
                out.push_str(&format!(
                    "Source: registry ({}@{})\n",
                    registry_url, version
                ));
            }
        }
        if !skill.linked_secrets.is_empty() {
            out.push_str(&format!(
                "Linked secrets: {}\n",
                skill.linked_secrets.join(", ")
            ));
        }
        if !gate.missing_bins.is_empty() {
            out.push_str(&format!(
                "Missing binaries: {}\n",
                gate.missing_bins.join(", ")
            ));
        }
        if !gate.missing_env.is_empty() {
            out.push_str(&format!(
                "Missing env vars: {}\n",
                gate.missing_env.join(", ")
            ));
        }
        Some(out)
    }
}

/// Parse YAML frontmatter from a markdown file
fn parse_frontmatter(content: &str) -> Result<(serde_yaml::Value, String)> {
    let content = content.trim_start();

    if !content.starts_with("---") {
        // No frontmatter, treat entire content as instructions
        return Ok((
            serde_yaml::Value::Mapping(Default::default()),
            content.to_string(),
        ));
    }

    // Find the closing ---
    let after_first = &content[3..];
    if let Some(end_idx) = after_first.find("\n---") {
        let frontmatter_str = &after_first[..end_idx];
        let instructions = after_first[end_idx + 4..].trim_start().to_string();

        let frontmatter: serde_yaml::Value =
            serde_yaml::from_str(frontmatter_str).context("Failed to parse YAML frontmatter")?;

        Ok((frontmatter, instructions))
    } else {
        // No closing ---, treat as no frontmatter
        Ok((
            serde_yaml::Value::Mapping(Default::default()),
            content.to_string(),
        ))
    }
}

mod clawhub;
pub use clawhub::*;

#[cfg(test)]
mod tests;

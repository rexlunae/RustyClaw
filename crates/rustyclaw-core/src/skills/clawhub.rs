//! ClawHub registry types + SkillManager registry methods.

#![allow(unused_imports)]
use super::*;

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

// ── ClawHub extended API types ──────────────────────────────────────────────

/// A trending / featured skill from the ClawHub API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendingEntry {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub stars: u64,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub version: String,
}

/// Response wrapper for trending skills.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrendingResponse {
    #[serde(default)]
    results: Vec<TrendingEntry>,
    #[serde(default)]
    skills: Vec<TrendingEntry>,
}

/// A skill category on ClawHub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub count: u64,
}

/// Response wrapper for categories.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CategoriesResponse {
    #[serde(default)]
    categories: Vec<Category>,
}

/// ClawHub user profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubProfile {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub published_count: u64,
    #[serde(default)]
    pub starred_count: u64,
    #[serde(default)]
    pub joined: String,
}

/// Response wrapper for profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileResponse {
    #[serde(default)]
    pub profile: Option<ClawHubProfile>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A starred skill entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarredEntry {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub starred_at: String,
}

/// Response wrapper for starred skills.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StarredResponse {
    #[serde(default)]
    results: Vec<StarredEntry>,
}

/// Auth response from ClawHub login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

/// Detailed info about a single skill from the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySkillDetail {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub stars: u64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub readme: Option<String>,
    #[serde(default)]
    pub required_secrets: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

impl SkillManager {
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
        let resp = blocking_request_with_retry(
            &client,
            &url,
            self.registry_token.as_deref(),
            Duration::from_secs(10),
        )
        .context("ClawHub registry is not reachable")?;

        let body: RegistrySearchResponse =
            resp.json().context("Failed to parse registry response")?;

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
        let mut url = format!(
            "{}/api/v1/download?slug={}",
            self.registry_url,
            urlencoding::encode(name)
        );
        if let Some(v) = version {
            url.push_str(&format!("&version={}", urlencoding::encode(v)));
        }

        let client = reqwest::blocking::Client::new();
        let resp = blocking_request_with_retry(
            &client,
            &url,
            self.registry_token.as_deref(),
            Duration::from_secs(30),
        )
        .context("Failed to download skill from ClawHub")?;

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
        std::fs::write(
            clawhub_dir.join("install.json"),
            serde_json::to_string_pretty(&meta)?,
        )?;

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

        let token = self.registry_token.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "ClawHub auth token required for publishing. Set clawhub_token in config."
            )
        })?;

        // Read the skill content.
        let content = std::fs::read_to_string(&skill.path).context("Failed to read skill file")?;

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

    // ── ClawHub extended API operations ─────────────────────────────

    /// Return the registry URL for display or browser opening.
    pub fn registry_url(&self) -> &str {
        &self.registry_url
    }

    /// Return the registry auth token (if set).
    pub fn registry_token(&self) -> Option<&str> {
        self.registry_token.as_deref()
    }

    /// Authenticate with ClawHub using a username and password.
    /// Returns the API token on success, which should be saved to config.
    pub fn auth_login(&self, username: &str, password: &str) -> Result<AuthResponse> {
        let url = format!("{}/api/v1/auth/login", self.registry_url);
        let client = reqwest::blocking::Client::new();
        let payload = serde_json::json!({
            "username": username,
            "password": password,
        });

        let resp = client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .context("Failed to connect to ClawHub for authentication")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("ClawHub auth failed (HTTP {}): {}", status, body);
        }

        let auth: AuthResponse = resp.json().context("Failed to parse auth response")?;
        Ok(auth)
    }

    /// Authenticate with ClawHub using a pre-existing API token.
    /// Validates the token and returns the profile info.
    pub fn auth_token(&self, token: &str) -> Result<AuthResponse> {
        let url = format!("{}/api/v1/auth/verify", self.registry_url);
        let client = reqwest::blocking::Client::new();

        let resp = client
            .get(&url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .context("Failed to connect to ClawHub for token verification")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!(
                "ClawHub token verification failed (HTTP {}): {}",
                status,
                body
            );
        }

        let auth: AuthResponse = resp.json().context("Failed to parse auth response")?;
        Ok(auth)
    }

    /// Check authentication status (whether a token is configured and valid).
    pub fn auth_status(&self) -> Result<String> {
        match &self.registry_token {
            Some(token) => match self.auth_token(token) {
                Ok(resp) if resp.ok => {
                    let user = resp.username.unwrap_or_else(|| "unknown".into());
                    Ok(format!(
                        "Authenticated as '{}' on {}",
                        user, self.registry_url
                    ))
                }
                Ok(_) => Ok(format!(
                    "Token configured but invalid on {}",
                    self.registry_url
                )),
                Err(_) => Ok(format!(
                    "Token configured but registry unreachable ({})",
                    self.registry_url,
                )),
            },
            None => Ok(
                "Not authenticated. Run `/clawhub auth login` or set clawhub_token in config."
                    .to_string(),
            ),
        }
    }

    /// Fetch trending / popular skills from the ClawHub registry.
    pub fn trending(
        &self,
        category: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<TrendingEntry>> {
        let mut url = format!("{}/api/v1/trending", self.registry_url);
        let mut params = vec![];
        if let Some(cat) = category {
            params.push(format!("category={}", urlencoding::encode(cat)));
        }
        if let Some(n) = limit {
            params.push(format!("limit={}", n));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

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
                "ClawHub trending request failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        let body: TrendingResponse = resp.json().context("Failed to parse trending response")?;
        let entries = if !body.results.is_empty() {
            body.results
        } else {
            body.skills
        };

        Ok(entries)
    }

    /// Fetch available categories from the ClawHub registry.
    pub fn categories(&self) -> Result<Vec<Category>> {
        let url = format!("{}/api/v1/categories", self.registry_url);
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
                "ClawHub categories request failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        let body: CategoriesResponse =
            resp.json().context("Failed to parse categories response")?;
        Ok(body.categories)
    }

    /// Fetch the authenticated user's profile from ClawHub.
    pub fn profile(&self) -> Result<ClawHubProfile> {
        let token = self.registry_token.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Not authenticated. Run `/clawhub auth login` or set clawhub_token in config."
            )
        })?;

        let url = format!("{}/api/v1/profile", self.registry_url);
        let client = reqwest::blocking::Client::new();

        let resp = client
            .get(&url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .context("ClawHub registry is not reachable")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "ClawHub profile request failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        let body: ProfileResponse = resp.json().context("Failed to parse profile response")?;
        match body.profile {
            Some(profile) => Ok(profile),
            None => anyhow::bail!(body.error.unwrap_or_else(|| "Profile not found".into())),
        }
    }

    /// Fetch the authenticated user's starred skills from ClawHub.
    pub fn starred(&self) -> Result<Vec<StarredEntry>> {
        let token = self.registry_token.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Not authenticated. Run `/clawhub auth login` or set clawhub_token in config."
            )
        })?;

        let url = format!("{}/api/v1/starred", self.registry_url);
        let client = reqwest::blocking::Client::new();

        let resp = client
            .get(&url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .context("ClawHub registry is not reachable")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "ClawHub starred request failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        let body: StarredResponse = resp.json().context("Failed to parse starred response")?;
        Ok(body.results)
    }

    /// Star a skill on ClawHub.
    pub fn star(&self, skill_name: &str) -> Result<String> {
        let token = self.registry_token.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Not authenticated. Run `/clawhub auth login` first.")
        })?;

        let url = format!(
            "{}/api/v1/skills/{}/star",
            self.registry_url,
            urlencoding::encode(skill_name),
        );
        let client = reqwest::blocking::Client::new();

        let resp = client
            .post(&url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .context("ClawHub registry is not reachable")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "ClawHub star failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        Ok(format!("Starred '{}'", skill_name))
    }

    /// Unstar a skill on ClawHub.
    pub fn unstar(&self, skill_name: &str) -> Result<String> {
        let token = self.registry_token.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Not authenticated. Run `/clawhub auth login` first.")
        })?;

        let url = format!(
            "{}/api/v1/skills/{}/star",
            self.registry_url,
            urlencoding::encode(skill_name),
        );
        let client = reqwest::blocking::Client::new();

        let resp = client
            .delete(&url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .context("ClawHub registry is not reachable")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "ClawHub unstar failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        Ok(format!("Unstarred '{}'", skill_name))
    }

    /// Get detailed info about a registry skill (not a locally installed one).
    pub fn registry_info(&self, skill_name: &str) -> Result<RegistrySkillDetail> {
        let url = format!(
            "{}/api/v1/skills/{}",
            self.registry_url,
            urlencoding::encode(skill_name),
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
                "ClawHub skill info failed (HTTP {}): {}",
                resp.status(),
                resp.text().unwrap_or_default(),
            );
        }

        let detail: RegistrySkillDetail = resp
            .json()
            .context("Failed to parse skill detail response")?;
        Ok(detail)
    }
}

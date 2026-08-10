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

/// Where a name from an archive is allowed to land under `base`, or a phrase
/// saying why it is not allowed anywhere.
///
/// `Path::join` looks like appending but is not: given an absolute path it
/// *replaces* the base entirely, and `..` components walk upward from it. The
/// installer joined archive entry names straight onto the skill directory, so
/// an entry called `../../.ssh/authorized_keys` or `/etc/cron.d/x` was written
/// wherever it asked, as the user running RustyClaw (issue #428).
///
/// This refuses rather than sanitises. Stripping the `..` from a hostile name
/// would install a file the archive did not describe, under a name nobody
/// saw, and still report success — the install would be wrong in a way that
/// looks exactly like it working.
///
/// Written out rather than delegated to `ZipFile::enclosed_name`, which
/// answers `None` without saying which rule was broken, and which cannot be
/// pointed at the slug — the other string that reaches a `join` here.
fn contained_path(base: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() {
        return Err("the empty string".to_string());
    }
    if relative.contains('\0') {
        return Err(format!("`{relative}`, which contains a NUL byte"));
    }
    // Zip entry names use `/` as the separator, always. A backslash is either
    // a Windows path — which would mean a directory on one platform and a
    // filename character on another — or a name chosen to read differently to
    // the checker than to the filesystem. Neither is worth supporting.
    if relative.contains('\\') {
        return Err(format!("`{relative}`, which uses a backslash"));
    }
    // `C:\x` and `C:x` are absolute and drive-relative on Windows. On Unix
    // `Path::components` sees `C:x` as one ordinary filename, so a check via
    // `Path` alone would only work on the platform that needs it least, and
    // the Linux-hosted tests would pass while Windows stayed open.
    if relative.as_bytes().get(1) == Some(&b':') {
        return Err(format!("`{relative}`, which names a drive"));
    }
    if relative.starts_with('/') {
        return Err(format!("`{relative}`, which is an absolute path"));
    }

    let mut out = base.to_path_buf();
    for part in relative.split('/') {
        match part {
            // `a//b`, and the trailing slash that marks a directory entry.
            "" | "." => continue,
            ".." => {
                return Err(format!(
                    "`{relative}`, which climbs above {}",
                    base.display()
                ));
            }
            _ => out.push(part),
        }
    }
    Ok(out)
}

/// Unpack a downloaded skill archive into `skill_dir`.
///
/// Split out of `install_from_registry` so it can be run against a hostile
/// archive in a test — the previous version was reachable only behind an HTTP
/// download, which is why the escape below went unnoticed.
///
/// Every entry is checked before any entry is written. A one-pass version that
/// refused on reaching a bad entry would still have written whatever came
/// before it, which for an archive that puts its payload last is the whole
/// payload.
fn extract_archive_into(skill_dir: &Path, zip_bytes: &[u8]) -> Result<(), SkillError> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| SkillError::context("Invalid zip archive", e))?;

    let mut planned = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let raw_name = file.name().to_string();

        // A symlink entry is the other way out of a directory: the link is
        // created inside, and points anywhere. Today's extractor writes the
        // target path into a regular file rather than creating a link, so
        // this is not currently an escape — it is refused so that teaching
        // the extractor about links later cannot quietly open one.
        if file.unix_mode().is_some_and(|mode| mode & 0xF000 == 0xA000) {
            return Err(SkillError::msg(format!(
                "Refusing archive entry `{raw_name}`, which is a symlink"
            )));
        }

        let dest = contained_path(skill_dir, &raw_name)
            .map_err(|why| SkillError::msg(format!("Refusing archive entry {why}")))?;
        planned.push((i, dest, file.is_dir()));
    }

    for (i, dest, is_dir) in planned {
        if is_dir {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = archive.by_index(i)?;
        let mut outfile = std::fs::File::create(&dest)?;
        std::io::copy(&mut file, &mut outfile)?;
    }
    Ok(())
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
    pub fn search_registry(&self, query: &str) -> Result<Vec<RegistryEntry>, SkillError> {
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
    fn search_registry_remote(&self, query: &str) -> Result<Vec<RegistryEntry>, SkillError> {
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
        .map_err(|e| SkillError::context("ClawHub registry is not reachable", e))?;

        let body: RegistrySearchResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse registry response", e))?;

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
    pub fn install_from_registry(
        &mut self,
        name: &str,
        version: Option<&str>,
    ) -> Result<Skill, SkillError> {
        if !self.registry_reachable() {
            return Err(SkillError::Unreachable(self.registry_url.clone()));
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
        .map_err(|e| SkillError::context("Failed to download skill from ClawHub", e))?;

        // Response is a zip file
        let zip_bytes = resp
            .bytes()
            .map_err(|e| SkillError::context("Failed to read zip data", e))?;

        // Use last directory (user's writable dir) for installations, not first (bundled/read-only)
        let skills_dir = self
            .skills_dirs
            .last()
            .cloned()
            .ok_or(SkillError::NoSkillsDir)?;

        // The slug reaches this from a tool argument or a registry search
        // result, and lands in a `join` — so it gets the same treatment as an
        // archive entry rather than being trusted for having arrived earlier.
        let skill_dir = contained_path(&skills_dir, name)
            .map_err(|why| SkillError::msg(format!("Refusing to install a skill named {why}")))?;
        std::fs::create_dir_all(&skill_dir)?;

        extract_archive_into(&skill_dir, &zip_bytes)?;

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
    pub fn publish_to_registry(&self, skill_name: &str) -> Result<String, SkillError> {
        let skill = self
            .get_skill(skill_name)
            .ok_or_else(|| SkillError::NotFound(skill_name.to_string()))?;

        let token = self
            .registry_token
            .as_ref()
            .ok_or(SkillError::NotAuthenticated)?;

        // Read the skill content.
        let content = std::fs::read_to_string(&skill.path)
            .map_err(|e| SkillError::context("Failed to read skill file", e))?;

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
            return Err(SkillError::Unreachable(self.registry_url.clone()));
        }

        let url = format!("{}/skills/publish", self.registry_url);
        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(&url)
            .bearer_auth(token)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .map_err(|e| SkillError::context("Failed to publish to ClawHub", e))?;

        if !resp.status().is_success() {
            return Err(SkillError::status(
                "ClawHub publish",
                resp.status(),
                resp.text().unwrap_or_default(),
            ));
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
    pub fn auth_login(&self, username: &str, password: &str) -> Result<AuthResponse, SkillError> {
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
            .map_err(|e| {
                SkillError::context("Failed to connect to ClawHub for authentication", e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(SkillError::status("ClawHub auth", status, body));
        }

        let auth: AuthResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse auth response", e))?;
        Ok(auth)
    }

    /// Authenticate with ClawHub using a pre-existing API token.
    /// Validates the token and returns the profile info.
    pub fn auth_token(&self, token: &str) -> Result<AuthResponse, SkillError> {
        let url = format!("{}/api/v1/auth/verify", self.registry_url);
        let client = reqwest::blocking::Client::new();

        let resp = client
            .get(&url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .map_err(|e| {
                SkillError::context("Failed to connect to ClawHub for token verification", e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(SkillError::status(
                "ClawHub token verification",
                status,
                body,
            ));
        }

        let auth: AuthResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse auth response", e))?;
        Ok(auth)
    }

    /// Check authentication status (whether a token is configured and valid).
    pub fn auth_status(&self) -> Result<String, SkillError> {
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
    ) -> Result<Vec<TrendingEntry>, SkillError> {
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
            .map_err(|e| SkillError::context("ClawHub registry is not reachable", e))?;

        if !resp.status().is_success() {
            return Err(SkillError::status(
                "ClawHub trending request",
                resp.status(),
                resp.text().unwrap_or_default(),
            ));
        }

        let body: TrendingResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse trending response", e))?;
        let entries = if !body.results.is_empty() {
            body.results
        } else {
            body.skills
        };

        Ok(entries)
    }

    /// Fetch available categories from the ClawHub registry.
    pub fn categories(&self) -> Result<Vec<Category>, SkillError> {
        let url = format!("{}/api/v1/categories", self.registry_url);
        let client = reqwest::blocking::Client::new();
        let mut req = client.get(&url);
        if let Some(ref token) = self.registry_token {
            req = req.bearer_auth(token);
        }

        let resp = req
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .map_err(|e| SkillError::context("ClawHub registry is not reachable", e))?;

        if !resp.status().is_success() {
            return Err(SkillError::status(
                "ClawHub categories request",
                resp.status(),
                resp.text().unwrap_or_default(),
            ));
        }

        let body: CategoriesResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse categories response", e))?;
        Ok(body.categories)
    }

    /// Fetch the authenticated user's profile from ClawHub.
    pub fn profile(&self) -> Result<ClawHubProfile, SkillError> {
        let token = self
            .registry_token
            .as_ref()
            .ok_or(SkillError::NotAuthenticated)?;

        let url = format!("{}/api/v1/profile", self.registry_url);
        let client = reqwest::blocking::Client::new();

        let resp = client
            .get(&url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .map_err(|e| SkillError::context("ClawHub registry is not reachable", e))?;

        if !resp.status().is_success() {
            return Err(SkillError::status(
                "ClawHub profile request",
                resp.status(),
                resp.text().unwrap_or_default(),
            ));
        }

        let body: ProfileResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse profile response", e))?;
        match body.profile {
            Some(profile) => Ok(profile),
            None => Err(SkillError::msg(
                body.error.unwrap_or_else(|| "Profile not found".into()),
            )),
        }
    }

    /// Fetch the authenticated user's starred skills from ClawHub.
    pub fn starred(&self) -> Result<Vec<StarredEntry>, SkillError> {
        let token = self
            .registry_token
            .as_ref()
            .ok_or(SkillError::NotAuthenticated)?;

        let url = format!("{}/api/v1/starred", self.registry_url);
        let client = reqwest::blocking::Client::new();

        let resp = client
            .get(&url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .map_err(|e| SkillError::context("ClawHub registry is not reachable", e))?;

        if !resp.status().is_success() {
            return Err(SkillError::status(
                "ClawHub starred request",
                resp.status(),
                resp.text().unwrap_or_default(),
            ));
        }

        let body: StarredResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse starred response", e))?;
        Ok(body.results)
    }

    /// Star a skill on ClawHub.
    pub fn star(&self, skill_name: &str) -> Result<String, SkillError> {
        let token = self
            .registry_token
            .as_ref()
            .ok_or(SkillError::NotAuthenticated)?;

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
            .map_err(|e| SkillError::context("ClawHub registry is not reachable", e))?;

        if !resp.status().is_success() {
            return Err(SkillError::status(
                "ClawHub star",
                resp.status(),
                resp.text().unwrap_or_default(),
            ));
        }

        Ok(format!("Starred '{}'", skill_name))
    }

    /// Unstar a skill on ClawHub.
    pub fn unstar(&self, skill_name: &str) -> Result<String, SkillError> {
        let token = self
            .registry_token
            .as_ref()
            .ok_or(SkillError::NotAuthenticated)?;

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
            .map_err(|e| SkillError::context("ClawHub registry is not reachable", e))?;

        if !resp.status().is_success() {
            return Err(SkillError::status(
                "ClawHub unstar",
                resp.status(),
                resp.text().unwrap_or_default(),
            ));
        }

        Ok(format!("Unstarred '{}'", skill_name))
    }

    /// Get detailed info about a registry skill (not a locally installed one).
    pub fn registry_info(&self, skill_name: &str) -> Result<RegistrySkillDetail, SkillError> {
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
            .map_err(|e| SkillError::context("ClawHub registry is not reachable", e))?;

        if !resp.status().is_success() {
            return Err(SkillError::status(
                "ClawHub skill info",
                resp.status(),
                resp.text().unwrap_or_default(),
            ));
        }

        let detail: RegistrySkillDetail = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse skill detail response", e))?;
        Ok(detail)
    }
}

/// Extraction is where a downloaded archive gets to choose filenames, so the
/// tests here build hostile archives and run the real extractor against them
/// rather than asserting on a path-checking helper in isolation.
#[cfg(test)]
mod extract_tests {
    use super::{contained_path, extract_archive_into};
    use std::io::Write;
    use std::path::Path;
    use zip::write::SimpleFileOptions;

    /// An archive containing exactly the named entries, each holding `payload`.
    fn archive_of(entries: &[&str], payload: &[u8]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for entry in entries {
                if let Some(dir) = entry.strip_suffix('/') {
                    zip.add_directory(dir, opts).expect("directory entry");
                } else {
                    zip.start_file(*entry, opts).expect("file entry");
                    zip.write_all(payload).expect("entry body");
                }
            }
            zip.finish().expect("archive finishes");
        }
        buf.into_inner()
    }

    /// A scratch skill directory, plus the directory it must not escape from.
    fn scratch() -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().expect("temp dir");
        let skill_dir = root.path().join("skills").join("some-skill");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        (root, skill_dir)
    }

    /// The reported gap in issue #428: `Path::join` with `..` walks upward, so
    /// an entry named this way was written outside the skill directory.
    #[test]
    fn an_entry_that_climbs_out_is_refused_and_writes_nothing() {
        let (root, skill_dir) = scratch();
        let zip = archive_of(&["../../pwned.txt"], b"payload");

        let err = extract_archive_into(&skill_dir, &zip).expect_err("must be refused");
        assert!(err.to_string().contains("climbs above"), "got: {err}");
        assert!(
            !root.path().join("pwned.txt").exists(),
            "the escaping entry was written anyway"
        );
    }

    /// And the other half of the same `join` surprise: an absolute path does
    /// not append to the base, it replaces it.
    #[test]
    fn an_absolute_entry_is_refused() {
        let (_root, skill_dir) = scratch();
        let zip = archive_of(&["/etc/cron.d/anything"], b"payload");

        let err = extract_archive_into(&skill_dir, &zip).expect_err("must be refused");
        assert!(err.to_string().contains("absolute path"), "got: {err}");
    }

    /// A symlink entry points out of the directory without any `..` in its
    /// name. Today's extractor writes the target as a regular file rather than
    /// creating a link, so this is refusing a door before it is opened.
    #[test]
    fn a_symlink_entry_is_refused() {
        let (_root, skill_dir) = scratch();
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.add_symlink("link", "/etc/passwd", SimpleFileOptions::default())
                .expect("symlink entry");
            zip.finish().expect("archive finishes");
        }

        let err = extract_archive_into(&skill_dir, &buf.into_inner()).expect_err("must be refused");
        assert!(err.to_string().contains("symlink"), "got: {err}");
    }

    /// The reason every entry is checked before any is written: an archive
    /// that puts its payload first and its escape last would otherwise have
    /// delivered the payload before the refusal.
    #[test]
    fn a_refusal_late_in_the_archive_still_writes_nothing() {
        let (_root, skill_dir) = scratch();
        let zip = archive_of(&["SKILL.md", "../escape"], b"payload");

        assert!(extract_archive_into(&skill_dir, &zip).is_err());
        assert!(
            !skill_dir.join("SKILL.md").exists(),
            "an entry before the bad one was written"
        );
    }

    /// An ordinary archive must still install, nested directories and all —
    /// a check that refuses everything would pass every test above.
    #[test]
    fn an_ordinary_archive_extracts() {
        let (_root, skill_dir) = scratch();
        let zip = archive_of(&["SKILL.md", "scripts/", "scripts/run.sh"], b"hello");

        extract_archive_into(&skill_dir, &zip).expect("an ordinary archive installs");
        assert_eq!(
            std::fs::read(skill_dir.join("SKILL.md")).expect("SKILL.md exists"),
            b"hello"
        );
        assert!(skill_dir.join("scripts").join("run.sh").exists());
    }

    /// Windows-shaped names, checked on whatever platform the tests run on:
    /// on Unix `Path` reads `C:x` as one ordinary filename, so a check built
    /// only on `Path::components` would pass here and leave Windows open.
    #[test]
    fn windows_shaped_names_are_refused_everywhere() {
        let base = Path::new("/skills/some-skill");
        for name in ["C:\\Windows\\evil", "C:evil", "..\\..\\evil", "sub\\evil"] {
            assert!(
                contained_path(base, name).is_err(),
                "{name} should be refused"
            );
        }
    }

    /// The slug goes through the same check, because it reaches the same
    /// `join` from a tool argument.
    #[test]
    fn a_slug_that_climbs_out_is_refused() {
        let base = Path::new("/skills");
        assert!(contained_path(base, "../../etc").is_err());
        assert!(contained_path(base, "").is_err());
        assert!(contained_path(base, "ordinary-skill").is_ok());
    }
}

//! ClawHub registry types + SkillManager registry methods.

#![allow(unused_imports)]
use super::*;

/// The registry's actual HTTP routes.
///
/// Gathered in one place because they were previously invented at each call
/// site, and half of them did not exist: `/api/v1/auth/verify`,
/// `/api/v1/auth/login`, `/api/v1/profile`, `/api/v1/categories`,
/// `/api/v1/starred`, `/api/v1/skills/{slug}/star` and `/skills/publish` all
/// 404 (see issue #411). Auth never succeeded, and publish posted to the SPA,
/// got HTML and a 200 back, and reported success having stored nothing.
///
/// Taken from the registry's own route table — `packages/schema/src/routes.ts`
/// in `github.com/openclaw/clawhub`, published as `clawhub-schema` — rather
/// than from observed behaviour, so the names are the ones the server matches
/// on and not a guess that happened to work once.
pub mod routes {
    /// `GET`, bearer token. The identity endpoint; there is no `/auth/verify`
    /// and no `/profile`.
    pub const WHOAMI: &str = "/api/v1/whoami";
    /// `GET ?q=`.
    pub const SEARCH: &str = "/api/v1/search";
    /// `GET ?q=`, the older search endpoint. Still served, and still what
    /// this client uses: the response parsing here was written against it,
    /// and the v1 shape has not been confirmed from a live call (see #411 —
    /// clawhub.ai is unreachable from CI). Named so it reads as a deliberate
    /// choice rather than another invented path.
    pub const LEGACY_SEARCH: &str = "/api/search";
    /// `GET ?slug=&version=`.
    pub const DOWNLOAD: &str = "/api/v1/download";
    /// `GET`.
    pub const TRENDING: &str = "/api/v1/trending";
    /// `GET /{slug}` for detail. `POST` publishes via upload tickets.
    pub const SKILLS: &str = "/api/v1/skills";
    /// `POST` to star, `DELETE` to unstar. Not `/skills/{slug}/star`, and
    /// there is no route that lists what a user has starred.
    pub const STARS: &str = "/api/v1/stars";

    /// `POST`, bearer token: mints a one-shot Convex storage upload URL.
    pub const CLI_UPLOAD_URL: &str = "/api/cli/upload-url";
    /// `POST`, bearer token: registers the uploaded files as a version.
    pub const CLI_PUBLISH: &str = "/api/cli/publish";
    /// `POST`: starts the device-code login the official CLI uses.
    pub const CLI_DEVICE_CODE: &str = "/api/cli/device/code";
    /// `POST`: exchanges a device code for a token.
    pub const CLI_DEVICE_TOKEN: &str = "/api/cli/device/token";
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

/// ClawHub user profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClawHubProfile {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub bio: String,
    /// `None` when the registry did not report it.
    ///
    /// Not `0`: whoami carries no counts, and a defaulted zero renders as
    /// "Published: 0  Starred: 0" — a claim about the user's activity that
    /// this client made up. Leaving a field at its default is still
    /// inventing data when the caller prints it unconditionally.
    #[serde(default)]
    pub published_count: Option<u64>,
    #[serde(default)]
    pub starred_count: Option<u64>,
    #[serde(default)]
    pub joined: String,
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

/// What `/api/v1/whoami` actually returns.
///
/// Nothing like the `{ok, token, username}` shape the client used to expect
/// from its imaginary `/auth/verify`: the handle is nested, and nullable for
/// an account that has not chosen one.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhoamiResponse {
    user: WhoamiUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhoamiUser {
    #[serde(default)]
    handle: Option<String>,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    image: Option<String>,
}

/// One-shot Convex storage URL from `/api/cli/upload-url`.
#[derive(Debug, Clone, Deserialize)]
struct UploadTicket {
    #[serde(rename = "uploadUrl")]
    upload_url: String,
}

/// Convex storage answers a successful PUT with the id to register.
#[derive(Debug, Clone, Deserialize)]
struct StoredUpload {
    #[serde(rename = "storageId")]
    storage_id: String,
}

/// `/api/cli/publish` reply.
#[derive(Debug, Clone, Deserialize)]
struct PublishAccepted {
    /// `None` when the reply omitted the flag.
    ///
    /// Not `false`. The HTTP status is the primary success signal, and the
    /// registry's schema declares this field — but that schema has not been
    /// confirmed against the live service, and defaulting a missing flag to
    /// "rejected" would report a stored skill as unpublished and send the
    /// user into a re-publish that then collides as a duplicate version.
    /// Only an explicit `false` is a rejection.
    #[serde(default)]
    ok: Option<bool>,
    #[serde(rename = "skillId", default)]
    skill_id: String,
}

/// Join a route onto the configured registry URL.
///
/// `set_registry_url` stores whatever the user configured, and a trailing
/// slash is a perfectly ordinary thing to configure — `https://clawhub.ai/`
/// concatenated with `/api/v1/whoami` gives `//api/v1/whoami`, which is a
/// different path as far as the router is concerned. One seam so there is
/// one place for that to be handled, and something for a test to hold onto.
pub(crate) fn endpoint(registry_url: &str, route: &str) -> String {
    format!("{}{}", registry_url.trim_end_matches('/'), route)
}

/// Whether an upload target the registry named is safe to send a file to.
///
/// The registry hands back a URL and the client PUTs the skill to it, so the
/// destination is chosen by whatever `clawhub_url` points at. A hostile or
/// misconfigured registry could name a plaintext endpoint, or an address
/// inside the caller's network, and the upload would go there.
///
/// The rule is relative rather than absolute, so self-hosting still works:
/// the upload target may not be *less* protected than the registry the user
/// chose. An https registry cannot redirect to http, and a registry on the
/// public internet cannot redirect to loopback. A registry that is itself on
/// localhost may name localhost, because that is a developer running both
/// halves on one machine.
fn upload_target_is_acceptable(registry: &url::Url, upload: &url::Url) -> Result<(), String> {
    if registry.scheme() == "https" && upload.scheme() != "https" {
        return Err(format!(
            "refused: the registry is https but named a {} upload target, which \
             would send the skill in plaintext",
            upload.scheme()
        ));
    }
    if !is_local_host(registry) && is_local_host(upload) {
        return Err(
            "refused: a remote registry named an upload target inside this \
             machine or network"
                .to_string(),
        );
    }
    Ok(())
}

/// Loopback, link-local, or an RFC1918 address — somewhere only this machine
/// or this network can reach.
fn is_local_host(u: &url::Url) -> bool {
    match u.host() {
        Some(url::Host::Domain(name)) => {
            let name = name.to_ascii_lowercase();
            name == "localhost" || name.ends_with(".localhost") || name.ends_with(".internal")
        }
        Some(url::Host::Ipv4(ip)) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        Some(url::Host::Ipv6(ip)) => {
            // `is_unique_local` and `is_unicast_link_local` are still
            // unstable, so the ranges are matched by prefix: fc00::/7 for
            // unique-local (fd00::/8 in practice) and fe80::/10 for
            // link-local. Without these, `https://[fd00::1]/x` walked
            // straight through a check written to stop exactly that.
            let first = ip.segments()[0];
            let unique_local = (first & 0xfe00) == 0xfc00;
            let link_local = (first & 0xffc0) == 0xfe80;
            // An IPv4 address wearing an IPv6 hat is still that address.
            let mapped_v4 = ip
                .to_ipv4_mapped()
                .is_some_and(|v4| v4.is_loopback() || v4.is_private() || v4.is_link_local());
            ip.is_loopback() || ip.is_unspecified() || unique_local || link_local || mapped_v4
        }
        None => false,
    }
}

/// Build a profile from what whoami actually returns.
///
/// A free function rather than inline in `profile()` so it can be exercised
/// against a real response body — the alternative was a test that built a
/// `ClawHubProfile` and asserted the defaults it had just written, which is
/// the kind of test `CONTRIBUTING.md` asks contributors not to write.
///
/// whoami carries a handle, a display name and an avatar. It carries no
/// counts, no bio and no join date, so those stay `None`/empty and the
/// callers omit them, rather than rendering a zero the registry never sent.
fn profile_from_whoami(whoami: WhoamiResponse) -> ClawHubProfile {
    ClawHubProfile {
        username: whoami.user.handle.clone().unwrap_or_default(),
        display_name: whoami
            .user
            .display_name
            .or(whoami.user.handle)
            .unwrap_or_default(),
        ..Default::default()
    }
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
        let url = format!(
            "{}?q={}",
            endpoint(&self.registry_url, routes::LEGACY_SEARCH),
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
            "{}?slug={}",
            endpoint(&self.registry_url, routes::DOWNLOAD),
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

        let skill_dir = skills_dir.join(name);
        std::fs::create_dir_all(&skill_dir)?;

        // Extract zip to skill directory
        let cursor = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| SkillError::context("Invalid zip archive", e))?;

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
    /// Publish a skill, having been told the publisher accepts the terms.
    ///
    /// The version is a parameter because it was a hardcoded `"0.1.0"`, which
    /// was harmless only while publish posted to a route that did not exist.
    /// Now that it reaches the registry, a constant version means a skill can
    /// be published exactly once and never updated — every later attempt
    /// collides with the version already stored, and the changelog has
    /// nothing to attach to.
    ///
    /// `accept_license_terms` is a parameter rather than a constant because
    /// the registry distributes skills under MIT-0 and requires the publisher
    /// to agree. Assuming it here would agree on the user's behalf.
    pub fn publish_to_registry(
        &self,
        skill_name: &str,
        version: &str,
        changelog: Option<String>,
        accept_license_terms: bool,
    ) -> Result<String, SkillError> {
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
            version: version.to_string(),
            description: skill.description.clone().unwrap_or_default(),
            author: String::new(),
            license: "MIT".to_string(),
            repository: skill.metadata.homepage.clone(),
            required_secrets: skill.linked_secrets.clone(),
            metadata: skill.metadata.clone(),
        };

        if !self.registry_reachable() {
            return Err(SkillError::Unreachable(self.registry_url.clone()));
        }

        // Publishing is three steps, not one. The old code posted a JSON blob
        // to `/skills/publish`, which is not an API route at all — the SPA
        // answered 200 with HTML and the command reported success having
        // stored nothing.
        //
        // The real flow, as the official CLI uses it: mint a one-shot upload
        // URL, PUT the file to Convex storage, then register the returned
        // storage id as a version.
        let client = reqwest::blocking::Client::new();

        let ticket_url = endpoint(&self.registry_url, routes::CLI_UPLOAD_URL);
        let ticket: UploadTicket = client
            .post(&ticket_url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .map_err(|e| SkillError::context("Failed to request an upload URL", e))?
            .error_for_status()
            .map_err(|e| SkillError::context("ClawHub refused the upload request", e))?
            .json()
            .map_err(|e| SkillError::context("Failed to parse the upload URL response", e))?;

        // Where the registry told us to send the file is the registry's
        // choice, not ours — check it before handing over the contents.
        {
            let registry = url::Url::parse(&self.registry_url)
                .map_err(|e| SkillError::msg(format!("Registry URL is not a URL: {e}")))?;
            let upload = url::Url::parse(&ticket.upload_url).map_err(|e| {
                SkillError::msg(format!(
                    "ClawHub returned an upload URL that is not a URL: {e}"
                ))
            })?;
            upload_target_is_acceptable(&registry, &upload).map_err(SkillError::msg)?;
        }

        let stored: StoredUpload = client
            .post(&ticket.upload_url)
            .header(reqwest::header::CONTENT_TYPE, "text/markdown")
            .body(content.clone())
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .map_err(|e| SkillError::context("Failed to upload the skill", e))?
            .error_for_status()
            .map_err(|e| SkillError::context("Storage refused the upload", e))?
            .json()
            .map_err(|e| SkillError::context("Failed to parse the storage response", e))?;

        // The registry checks this against what it received, so it is the
        // digest of exactly the bytes that were sent.
        let sha256 = {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(content.as_bytes());
            hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };

        let payload = serde_json::json!({
            "slug": manifest.name,
            "displayName": manifest.name,
            "version": manifest.version,
            "changelog": changelog.unwrap_or_default(),
            // The registry publishes under MIT-0 terms and requires the
            // publisher to say so. Passed through from the caller rather than
            // hardcoded, so nobody agrees to a licence on the user's behalf.
            "acceptLicenseTerms": accept_license_terms,
            "files": [{
                "path": "SKILL.md",
                "size": content.len(),
                "storageId": stored.storage_id,
                "sha256": sha256,
                "contentType": "text/markdown",
            }],
        });

        let publish_url = endpoint(&self.registry_url, routes::CLI_PUBLISH);
        let resp = client
            .post(&publish_url)
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

        // A success that cannot be parsed is not a success worth reporting:
        // that is exactly how the old code claimed to have published HTML.
        let done: PublishAccepted = resp.json().map_err(|e| {
            SkillError::context("ClawHub returned an unrecognised publish reply", e)
        })?;
        if done.ok == Some(false) {
            return Err(SkillError::msg(
                "ClawHub did not accept the publish".to_string(),
            ));
        }

        Ok(format!(
            "Published {} v{} to {} (skill {})",
            manifest.name, manifest.version, self.registry_url, done.skill_id,
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
    /// Password login, which this registry does not offer.
    ///
    /// The old implementation posted to `/api/v1/auth/login`, a route that
    /// has never existed, so every attempt failed with a 404 dressed up as an
    /// auth failure. There is no password API to point at instead: the
    /// registry issues API tokens from its settings page, and the official
    /// CLI uses a device-code flow. Say that, rather than making a request
    /// that cannot succeed.
    pub fn auth_login(&self, _username: &str, _password: &str) -> Result<AuthResponse, SkillError> {
        Err(SkillError::msg(format!(
            "ClawHub has no password login. Create an API token at \
             {}/settings/tokens and run `clawhub auth login <token>`.",
            self.registry_url
        )))
    }

    /// Authenticate with ClawHub using a pre-existing API token.
    /// Validates the token and returns the profile info.
    pub fn auth_token(&self, token: &str) -> Result<AuthResponse, SkillError> {
        let url = endpoint(&self.registry_url, routes::WHOAMI);
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

        let whoami: WhoamiResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse whoami response", e))?;
        Ok(AuthResponse {
            ok: true,
            // The caller already holds the token; whoami does not reissue it.
            token: Some(token.to_string()),
            username: whoami.user.handle,
            message: None,
        })
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
                // A 404 used to be reported as "registry unreachable", which
                // sent people to check their network for a bug in this client.
                // The registry answered; it just had no such route.
                Err(SkillError::Status { status, .. }) if status.as_u16() == 404 => Ok(format!(
                    "Token configured, but {} did not recognise the identity \
                     endpoint. The registry is reachable — this build is asking \
                     for a route it does not serve.",
                    self.registry_url
                )),
                Err(SkillError::Status { status, .. }) if status.as_u16() == 401 => Ok(format!(
                    "Token configured but rejected by {}. Create a new one at \
                     {}/settings/tokens.",
                    self.registry_url, self.registry_url
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
        let mut url = endpoint(&self.registry_url, routes::TRENDING);
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
        // `/api/v1/categories` has never existed; the old call 404'd and the
        // failure was reported as if the registry were broken. There is no
        // category route to move to — the catalogue is exposed through feeds
        // and promotions instead — so this is an absence to state, not an
        // endpoint to correct.
        Err(SkillError::msg(
            "ClawHub does not expose a categories API. Browse the catalogue at \
             this registry's website, or use `search` and `trending`."
                .to_string(),
        ))
    }

    /// Fetch the authenticated user's profile from ClawHub.
    pub fn profile(&self) -> Result<ClawHubProfile, SkillError> {
        let token = self
            .registry_token
            .as_ref()
            .ok_or(SkillError::NotAuthenticated)?;

        let url = endpoint(&self.registry_url, routes::WHOAMI);
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

        let whoami: WhoamiResponse = resp
            .json()
            .map_err(|e| SkillError::context("Failed to parse whoami response", e))?;
        Ok(profile_from_whoami(whoami))
    }

    /// Fetch the authenticated user's starred skills from ClawHub.
    pub fn starred(&self) -> Result<Vec<StarredEntry>, SkillError> {
        // Stars can be added and removed (`POST`/`DELETE /api/v1/stars`) but
        // not listed: there is no `GET`. The old `/api/v1/starred` 404'd.
        Err(SkillError::msg(
            "ClawHub has no API for listing starred skills — stars can only be \
             added and removed. Your stars are visible on the registry website."
                .to_string(),
        ))
    }

    /// Star a skill on ClawHub.
    pub fn star(&self, skill_name: &str) -> Result<String, SkillError> {
        let token = self
            .registry_token
            .as_ref()
            .ok_or(SkillError::NotAuthenticated)?;

        // One `stars` collection, with the slug in the body — not a
        // `/skills/{slug}/star` sub-resource, which 404'd.
        let url = endpoint(&self.registry_url, routes::STARS);
        let client = reqwest::blocking::Client::new();

        let resp = client
            .post(&url)
            .bearer_auth(token)
            .json(&serde_json::json!({ "slug": skill_name }))
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

        let url = endpoint(&self.registry_url, routes::STARS);
        let client = reqwest::blocking::Client::new();

        let resp = client
            .delete(&url)
            .bearer_auth(token)
            .json(&serde_json::json!({ "slug": skill_name }))
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
            "{}/{}",
            endpoint(&self.registry_url, routes::SKILLS),
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

#[cfg(test)]
mod route_tests {
    use super::*;

    /// The URLs the client actually builds.
    ///
    /// Replaces two change-detectors that asserted the route constants
    /// equalled literal copies of themselves and that one hardcoded array
    /// did not contain another — vacuous, and the pattern CONTRIBUTING.md
    /// asks contributors not to write. This exercises the joining instead,
    /// which is where a real defect lives: `set_registry_url` stores what
    /// the user configured, and a trailing slash is an ordinary thing to
    /// configure.
    #[test]
    fn a_trailing_slash_on_the_registry_url_does_not_double_up() {
        for configured in [
            "https://clawhub.ai",
            "https://clawhub.ai/",
            "https://clawhub.ai///",
        ] {
            assert_eq!(
                endpoint(configured, routes::WHOAMI),
                "https://clawhub.ai/api/v1/whoami",
                "built from {configured}"
            );
        }
    }

    /// Each route reaches its endpoint under the path the server matches on.
    #[test]
    fn every_route_builds_the_path_the_registry_serves() {
        let base = "https://clawhub.ai";
        for (route, expected) in [
            (routes::WHOAMI, "https://clawhub.ai/api/v1/whoami"),
            (routes::STARS, "https://clawhub.ai/api/v1/stars"),
            (routes::TRENDING, "https://clawhub.ai/api/v1/trending"),
            (routes::SKILLS, "https://clawhub.ai/api/v1/skills"),
            (routes::DOWNLOAD, "https://clawhub.ai/api/v1/download"),
            (
                routes::CLI_UPLOAD_URL,
                "https://clawhub.ai/api/cli/upload-url",
            ),
            (routes::CLI_PUBLISH, "https://clawhub.ai/api/cli/publish"),
            (routes::LEGACY_SEARCH, "https://clawhub.ai/api/search"),
        ] {
            assert_eq!(endpoint(base, route), expected);
        }
    }

    /// A self-hosted registry under a path prefix keeps that prefix.
    #[test]
    fn a_registry_behind_a_path_prefix_keeps_it() {
        assert_eq!(
            endpoint("https://example.test/clawhub/", routes::WHOAMI),
            "https://example.test/clawhub/api/v1/whoami"
        );
    }

    /// The commands with no backing route refuse instead of requesting one.
    ///
    /// A 404 dressed up as "the registry is unreachable" sent people to check
    /// their network for a missing feature. These have no endpoint to move
    /// to, so the honest answer is that the registry does not offer them.
    #[test]
    fn unsupported_commands_say_so_without_a_request() {
        let mgr = SkillManager::new(std::path::PathBuf::from("/nonexistent"));

        let categories = mgr.categories().expect_err("no categories API exists");
        assert!(
            categories.to_string().contains("does not expose"),
            "got: {categories}"
        );

        let starred = mgr.starred().expect_err("no starred-list API exists");
        assert!(starred.to_string().contains("no API"), "got: {starred}");

        // Password login is the same: refused locally rather than posted to a
        // route that never existed.
        let login = mgr
            .auth_login("someone", "hunter2")
            .expect_err("no password login exists");
        assert!(
            login.to_string().contains("no password login"),
            "got: {login}"
        );
        assert!(
            login.to_string().contains("/settings/tokens"),
            "the refusal should say how to authenticate instead: {login}"
        );
        // The command it names has to be one that exists — `auth token` does
        // not; the subcommand is `auth login <token>`.
        assert!(
            login.to_string().contains("auth login"),
            "the refusal must point at a real command: {login}"
        );
    }

    /// whoami reports no counts, so the profile must not claim any.
    ///
    /// Driven through the real deserialisation and mapping. The first attempt
    /// at this test built a `ClawHubProfile` by hand and asserted the
    /// defaults it had just set, which could only have failed if the type
    /// changed — exactly what `CONTRIBUTING.md` says not to write.
    #[test]
    fn a_profile_from_whoami_claims_no_counts() {
        let body =
            r#"{"user":{"handle":"someone","displayName":"Someone","image":"https://x/y.png"}}"#;
        let whoami: WhoamiResponse = serde_json::from_str(body).expect("whoami body parses");
        let profile = profile_from_whoami(whoami);

        assert_eq!(profile.username, "someone");
        assert_eq!(profile.display_name, "Someone");
        assert_eq!(profile.published_count, None, "whoami sends no counts");
        assert_eq!(profile.starred_count, None, "whoami sends no counts");
        assert!(profile.bio.is_empty());
        assert!(profile.joined.is_empty());
    }

    /// A handle is nullable, and the display name is optional.
    #[test]
    fn a_profile_survives_the_fields_whoami_may_omit() {
        // Only the required key, with the handle null.
        let sparse: WhoamiResponse =
            serde_json::from_str(r#"{"user":{"handle":null}}"#).expect("sparse body parses");
        let profile = profile_from_whoami(sparse);
        assert!(profile.username.is_empty());
        assert!(profile.display_name.is_empty());

        // No display name: the handle stands in, rather than showing blank.
        let handle_only: WhoamiResponse =
            serde_json::from_str(r#"{"user":{"handle":"someone"}}"#).expect("body parses");
        let profile = profile_from_whoami(handle_only);
        assert_eq!(profile.display_name, "someone");
    }
}

#[cfg(test)]
mod upload_target_tests {
    use super::upload_target_is_acceptable;

    fn check(registry: &str, upload: &str) -> Result<(), String> {
        upload_target_is_acceptable(
            &url::Url::parse(registry).unwrap(),
            &url::Url::parse(upload).unwrap(),
        )
    }

    /// The normal case: a hosted registry naming its storage bucket.
    #[test]
    fn a_https_registry_may_name_a_https_bucket() {
        assert!(check("https://clawhub.ai", "https://files.convex.cloud/abc").is_ok());
    }

    /// The registry chooses where the file goes, so it must not be able to
    /// choose plaintext — the skill would cross the network in the clear.
    #[test]
    fn a_https_registry_may_not_downgrade_the_upload_to_http() {
        let err = check("https://clawhub.ai", "http://files.example/abc")
            .expect_err("a downgrade must be refused");
        assert!(err.contains("plaintext"), "got: {err}");
    }

    /// Nor point the upload inward: that turns publish into a request the
    /// caller never intended to make against their own network.
    #[test]
    fn a_remote_registry_may_not_name_a_local_target() {
        for inside in [
            "https://localhost/upload",
            "https://127.0.0.1/upload",
            "https://10.1.2.3/upload",
            "https://192.168.0.5/upload",
            "https://169.254.169.254/latest/meta-data",
            "https://build.internal/upload",
        ] {
            let err =
                check("https://clawhub.ai", inside).expect_err("a local target must be refused");
            assert!(err.contains("inside this"), "got: {err}");
        }
    }

    /// IPv6 has its own inside-the-network ranges, and the first version of
    /// this guard checked only loopback — so `[fd00::1]` walked through a
    /// check written to stop exactly that.
    #[test]
    fn a_remote_registry_may_not_name_a_local_ipv6_target() {
        for inside in [
            "https://[::1]/upload",
            "https://[fd00::1]/upload",
            "https://[fdff:1234::5]/upload",
            "https://[fe80::1]/upload",
            "https://[::ffff:127.0.0.1]/upload",
            "https://[::ffff:10.0.0.1]/upload",
        ] {
            assert!(
                check("https://clawhub.ai", inside).is_err(),
                "{inside} should be refused"
            );
        }
    }

    /// A routable IPv6 address is not inside the network and must still work.
    #[test]
    fn a_public_ipv6_target_is_fine() {
        assert!(check("https://clawhub.ai", "https://[2606:4700::1111]/x").is_ok());
    }

    /// But a developer running the registry locally is not an attack, and
    /// must keep working — which is why the rule is relative.
    #[test]
    fn a_local_registry_may_name_a_local_target() {
        assert!(check("http://localhost:3000", "http://localhost:3000/upload").is_ok());
        assert!(check("http://127.0.0.1:3000", "http://127.0.0.1:9000/x").is_ok());
    }
}

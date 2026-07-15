//! External triggers: user/agent-defined programs that fire agent runs.
//!
//! A trigger is a small piece of code (a script) that the gateway runs as a
//! child process for as long as the gateway itself is running. When the
//! trigger's condition is met — a webhook arrives, a file changes, a poll
//! succeeds, whatever the code decides — it calls back to the gateway
//! (endpoint + token are provided via environment variables) with a JSON
//! *trigger context*, and the gateway runs the target agent with that
//! context.
//!
//! Trigger definitions are stored **encrypted** in a dedicated
//! [`securestore`] vault under `<settings_dir>/triggers/`. The vault always
//! uses an auto-generated keyfile (never the user's vault password) so
//! trigger definitions can be loaded when the gateway boots, even while the
//! main secrets vault is password-locked.

use anyhow::{Context, Result, bail};
use securestore::KeySource;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::agents::{is_valid_agent_id, sanitize_agent_id};

/// Vault key prefix for trigger definitions.
const KEY_PREFIX: &str = "trigger:";

/// Environment variables handed to running trigger processes.
pub const ENV_TRIGGER_ID: &str = "RUSTYCLAW_TRIGGER_ID";
pub const ENV_TRIGGER_AGENT: &str = "RUSTYCLAW_TRIGGER_AGENT";
pub const ENV_TRIGGER_ENDPOINT: &str = "RUSTYCLAW_TRIGGER_ENDPOINT";
pub const ENV_TRIGGER_TOKEN: &str = "RUSTYCLAW_TRIGGER_TOKEN";

/// One trigger definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDef {
    /// Stable id (same character set as agent ids).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// What this trigger watches for / why it exists.
    #[serde(default)]
    pub description: Option<String>,
    /// Agent to run when the trigger fires.
    pub agent_id: String,
    /// The trigger program: script text executed by `interpreter`. The code
    /// receives the callback endpoint and token via the `RUSTYCLAW_TRIGGER_*`
    /// environment variables and fires by POSTing
    /// `{"token": "...", "context": {...}}` to `http://<endpoint>/fire`.
    pub code: String,
    /// Interpreter for `code` (default: `sh`). The code is fed to the
    /// interpreter over stdin — never written to disk — so any interpreter
    /// that runs a program from a non-tty stdin works (`sh`, `bash`,
    /// `python3`, `node`, `ruby`, …).
    #[serde(default)]
    pub interpreter: Option<String>,
    /// Disabled triggers stay stored but are not started.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Agent that created this trigger, when created via a tool.
    #[serde(default)]
    pub created_by: Option<String>,
    /// Unix timestamp (seconds) of creation.
    #[serde(default)]
    pub created_at: Option<u64>,
}

fn default_true() -> bool {
    true
}

impl TriggerDef {
    /// Interpreter binary to run the trigger's code with.
    pub fn interpreter(&self) -> &str {
        self.interpreter.as_deref().unwrap_or("sh")
    }

    /// Content fingerprint — when it changes, a running trigger process is
    /// restarted to pick up the new definition.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.agent_id.hash(&mut h);
        self.code.hash(&mut h);
        self.interpreter().hash(&mut h);
        h.finish()
    }
}

/// Serialize access to the on-disk vault: tools (sync, any thread) and the
/// gateway's trigger manager (async task) both do load-modify-save cycles.
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// Encrypted, filesystem-backed store of trigger definitions.
///
/// Layout under `<settings_dir>/triggers/`:
/// - `store.vault` — securestore vault holding one `trigger:<id>` JSON
///   secret per trigger
/// - `store.key`   — auto-generated master key (owner-only permissions)
/// - `runs/`       — per-trigger fire history (JSONL)
///
/// Trigger code is never written to disk in plaintext: the gateway feeds it
/// to the interpreter over stdin at spawn time.
#[derive(Debug, Clone)]
pub struct TriggerStore {
    dir: PathBuf,
}

impl TriggerStore {
    /// Open (lazily) the trigger store rooted at
    /// `<settings_dir>/triggers`. Nothing touches the disk until the first
    /// read or write.
    pub fn open(settings_dir: &Path) -> Self {
        Self {
            dir: settings_dir.join("triggers"),
        }
    }

    /// Directory holding the vault, scripts, and run logs.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn vault_path(&self) -> PathBuf {
        self.dir.join("store.vault")
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join("store.key")
    }

    /// Path of the JSONL fire-history log for a trigger.
    pub fn runs_path(&self, id: &str) -> PathBuf {
        self.dir.join("runs").join(format!("{}.jsonl", id))
    }

    /// Load the vault, creating it (and its keyfile) on first use.
    fn load_vault(&self) -> Result<securestore::SecretsManager> {
        std::fs::create_dir_all(&self.dir).context("Failed to create triggers directory")?;
        if self.vault_path().exists() {
            securestore::SecretsManager::load(
                self.vault_path(),
                KeySource::from_file(self.key_path()),
            )
            .context("Failed to load trigger vault")
        } else {
            let vault = securestore::SecretsManager::new(KeySource::Csprng)
                .context("Failed to create trigger vault")?;
            vault
                .export_key(self.key_path())
                .context("Failed to export trigger vault key")?;
            set_owner_only(&self.key_path())
                .context("Failed to secure trigger vault key permissions")?;
            vault
                .save_as(self.vault_path())
                .context("Failed to save trigger vault")?;
            securestore::SecretsManager::load(
                self.vault_path(),
                KeySource::from_file(self.key_path()),
            )
            .context("Failed to reload trigger vault")
        }
    }

    /// List all trigger definitions, sorted by id.
    pub fn list(&self) -> Result<Vec<TriggerDef>> {
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if !self.vault_path().exists() {
            return Ok(Vec::new());
        }
        let vault = self.load_vault()?;
        let keys: Vec<String> = vault.keys().map(String::from).collect();
        let mut defs = Vec::new();
        for key in keys {
            if key.strip_prefix(KEY_PREFIX).is_none() {
                continue;
            }
            if let Ok(json) = vault.get(&key) {
                match serde_json::from_str::<TriggerDef>(&json) {
                    Ok(def) => defs.push(def),
                    Err(e) => tracing::warn!(key = %key, error = %e, "Corrupt trigger entry"),
                }
            }
        }
        defs.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(defs)
    }

    /// Fetch one trigger by id.
    pub fn get(&self, id: &str) -> Result<Option<TriggerDef>> {
        if !is_valid_agent_id(id) {
            return Ok(None);
        }
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if !self.vault_path().exists() {
            return Ok(None);
        }
        let vault = self.load_vault()?;
        match vault.get(&format!("{}{}", KEY_PREFIX, id)) {
            Ok(json) => Ok(Some(
                serde_json::from_str(&json).context("Corrupt trigger entry")?,
            )),
            Err(e) if e.kind() == securestore::ErrorKind::SecretNotFound => Ok(None),
            Err(e) => Err(anyhow::anyhow!("Failed to read trigger: {}", e)),
        }
    }

    /// Insert or update a trigger definition (validates its ids first).
    pub fn upsert(&self, def: &TriggerDef) -> Result<()> {
        if !is_valid_agent_id(&def.id) {
            bail!(
                "Invalid trigger id '{}': use lowercase letters, digits, '-' or '_'",
                def.id
            );
        }
        if !is_valid_agent_id(&def.agent_id) {
            bail!("Invalid agent id '{}'", def.agent_id);
        }
        if def.name.trim().is_empty() {
            bail!("Trigger name must not be empty");
        }
        if def.code.trim().is_empty() {
            bail!("Trigger code must not be empty");
        }
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut vault = self.load_vault()?;
        let json = serde_json::to_string(def).context("Failed to serialize trigger")?;
        vault.set(&format!("{}{}", KEY_PREFIX, def.id), json);
        vault.save().context("Failed to save trigger vault")?;
        Ok(())
    }

    /// Remove a trigger. Errors if it does not exist.
    pub fn remove(&self, id: &str) -> Result<()> {
        if !is_valid_agent_id(id) {
            bail!("Invalid trigger id '{}'", id);
        }
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut vault = self.load_vault()?;
        vault
            .remove(&format!("{}{}", KEY_PREFIX, id))
            .map_err(|_| anyhow::anyhow!("Trigger '{}' not found", id))?;
        vault.save().context("Failed to save trigger vault")?;
        Ok(())
    }

    /// Enable or disable a trigger.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut def = self
            .get(id)?
            .ok_or_else(|| anyhow::anyhow!("Trigger '{}' not found", id))?;
        def.enabled = enabled;
        self.upsert(&def)
    }

    /// Append one fire record to the trigger's run history.
    pub fn record_run(&self, id: &str, record: &TriggerRunRecord) -> Result<()> {
        let path = self.runs_path(id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        use std::io::Write;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(line.as_bytes())?;
        Ok(())
    }
}

/// One entry in a trigger's fire history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerRunRecord {
    /// Unix timestamp (seconds) when the fire was received.
    pub ts: u64,
    /// Agent that was run.
    pub agent_id: String,
    /// Whether the agent turn completed without error.
    pub ok: bool,
    /// Response excerpt or error message.
    pub detail: String,
}

/// Derive a trigger id from a display name.
pub fn derive_trigger_id(name: &str) -> String {
    sanitize_agent_id(name)
}

fn set_owner_only(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn def(id: &str) -> TriggerDef {
        TriggerDef {
            id: id.to_string(),
            name: format!("Trigger {}", id),
            description: None,
            agent_id: "main".to_string(),
            code: "echo hi".to_string(),
            interpreter: None,
            enabled: true,
            created_by: None,
            created_at: Some(0),
        }
    }

    #[test]
    fn crud_roundtrip() {
        let tmp = tempdir().unwrap();
        let store = TriggerStore::open(tmp.path());

        assert!(store.list().unwrap().is_empty());
        assert!(store.get("watcher").unwrap().is_none());

        store.upsert(&def("watcher")).unwrap();
        store.upsert(&def("alpha")).unwrap();

        let defs = store.list().unwrap();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].id, "alpha"); // sorted
        assert_eq!(defs[1].id, "watcher");

        store.set_enabled("watcher", false).unwrap();
        assert!(!store.get("watcher").unwrap().unwrap().enabled);

        store.remove("alpha").unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(store.remove("alpha").is_err());
    }

    #[test]
    fn vault_is_encrypted_on_disk() {
        let tmp = tempdir().unwrap();
        let store = TriggerStore::open(tmp.path());
        let mut d = def("secretive");
        d.code = "curl -H 'X-Api-Key: SUPERSECRETVALUE' https://example.com".to_string();
        store.upsert(&d).unwrap();

        let raw = std::fs::read_to_string(tmp.path().join("triggers/store.vault")).unwrap();
        assert!(
            !raw.contains("SUPERSECRETVALUE"),
            "trigger code must not appear in plaintext on disk"
        );
        // But it decrypts fine through the store.
        assert!(
            store
                .get("secretive")
                .unwrap()
                .unwrap()
                .code
                .contains("SUPERSECRETVALUE")
        );
    }

    #[test]
    fn invalid_ids_rejected() {
        let tmp = tempdir().unwrap();
        let store = TriggerStore::open(tmp.path());

        let mut bad = def("ok");
        bad.id = "../escape".to_string();
        assert!(store.upsert(&bad).is_err());

        let mut bad_agent = def("ok");
        bad_agent.agent_id = "Not Valid".to_string();
        assert!(store.upsert(&bad_agent).is_err());

        let mut empty_code = def("ok");
        empty_code.code = "  ".to_string();
        assert!(store.upsert(&empty_code).is_err());

        assert!(store.get("../escape").unwrap().is_none());
        assert!(store.remove("../escape").is_err());
    }

    #[test]
    fn fingerprint_tracks_content() {
        let a = def("x");
        let mut b = def("x");
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.code = "echo changed".to_string();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}

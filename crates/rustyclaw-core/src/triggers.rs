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

use securestore::KeySource;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use thiserror::Error;

use crate::agents::{is_valid_agent_id, sanitize_agent_id};

/// Vault key prefix for trigger definitions.
const KEY_PREFIX: &str = "trigger:";

/// Errors from trigger-store operations.
#[derive(Debug, Error)]
pub enum TriggerStoreError {
    /// Filesystem I/O failure (vault dir, run history).
    #[error("Trigger store I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON (de)serialization failure for a trigger definition or record.
    #[error("Trigger store serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// Encrypted-vault backend failure.
    #[error("Trigger vault error: {0}")]
    Vault(String),
    /// A supplied trigger definition failed validation.
    #[error("Invalid trigger: {0}")]
    Invalid(String),
    /// No trigger with the given id exists.
    #[error("Trigger not found: {0}")]
    NotFound(String),
}

/// Convenience result alias for the trigger store.
pub type TriggerResult<T> = std::result::Result<T, TriggerStoreError>;

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
    /// Run the trigger's process inside the sandbox layer (default: true).
    /// Sandboxed triggers keep network access (so callbacks work) and their
    /// agent's workspace, but lose access to the rest of the host. Set to
    /// `false` for a trigger that genuinely needs broad host access (e.g.
    /// watching a path outside the workspace or running system tools).
    #[serde(default = "default_true")]
    pub sandboxed: bool,
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
        self.sandboxed.hash(&mut h);
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
    fn load_vault(&self) -> TriggerResult<securestore::SecretsManager> {
        std::fs::create_dir_all(&self.dir)?;
        let vault = if self.vault_path().exists() {
            securestore::SecretsManager::load(
                self.vault_path(),
                KeySource::from_file(self.key_path()),
            )
            .map_err(|e| TriggerStoreError::Vault(format!("load failed: {e}")))?
        } else {
            let vault = securestore::SecretsManager::new(KeySource::Csprng)
                .map_err(|e| TriggerStoreError::Vault(format!("create failed: {e}")))?;
            vault
                .export_key(self.key_path())
                .map_err(|e| TriggerStoreError::Vault(format!("export key failed: {e}")))?;
            set_owner_only(&self.key_path())?;
            vault
                .save_as(self.vault_path())
                .map_err(|e| TriggerStoreError::Vault(format!("save failed: {e}")))?;
            securestore::SecretsManager::load(
                self.vault_path(),
                KeySource::from_file(self.key_path()),
            )
            .map_err(|e| TriggerStoreError::Vault(format!("reload failed: {e}")))?
        };
        Ok(vault)
    }

    /// List all trigger definitions, sorted by id.
    pub fn list(&self) -> TriggerResult<Vec<TriggerDef>> {
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
    pub fn get(&self, id: &str) -> TriggerResult<Option<TriggerDef>> {
        if !is_valid_agent_id(id) {
            return Ok(None);
        }
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if !self.vault_path().exists() {
            return Ok(None);
        }
        let vault = self.load_vault()?;
        match vault.get(&format!("{}{}", KEY_PREFIX, id)) {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(e) if e.kind() == securestore::ErrorKind::SecretNotFound => Ok(None),
            Err(e) => Err(TriggerStoreError::Vault(format!("read failed: {e}"))),
        }
    }

    /// Insert or update a trigger definition (validates its ids first).
    pub fn upsert(&self, def: &TriggerDef) -> TriggerResult<()> {
        if !is_valid_agent_id(&def.id) {
            return Err(TriggerStoreError::Invalid(format!(
                "id '{}': use lowercase letters, digits, '-' or '_'",
                def.id
            )));
        }
        if !is_valid_agent_id(&def.agent_id) {
            return Err(TriggerStoreError::Invalid(format!(
                "agent id '{}'",
                def.agent_id
            )));
        }
        if def.name.trim().is_empty() {
            return Err(TriggerStoreError::Invalid("name must not be empty".into()));
        }
        if def.code.trim().is_empty() {
            return Err(TriggerStoreError::Invalid("code must not be empty".into()));
        }
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut vault = self.load_vault()?;
        let json = serde_json::to_string(def)?;
        vault.set(&format!("{}{}", KEY_PREFIX, def.id), json);
        vault
            .save()
            .map_err(|e| TriggerStoreError::Vault(format!("save failed: {e}")))?;
        Ok(())
    }

    /// Remove a trigger. Errors if it does not exist.
    pub fn remove(&self, id: &str) -> TriggerResult<()> {
        if !is_valid_agent_id(id) {
            return Err(TriggerStoreError::Invalid(format!("id '{}'", id)));
        }
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut vault = self.load_vault()?;
        vault
            .remove(&format!("{}{}", KEY_PREFIX, id))
            .map_err(|_| TriggerStoreError::NotFound(id.to_string()))?;
        vault
            .save()
            .map_err(|e| TriggerStoreError::Vault(format!("save failed: {e}")))?;
        Ok(())
    }

    /// Enable or disable a trigger.
    ///
    /// The read-modify-write happens under a single `STORE_LOCK` acquisition,
    /// so a concurrent `upsert` to the same trigger cannot be clobbered by a
    /// stale snapshot (only the `enabled` field is changed here).
    pub fn set_enabled(&self, id: &str, enabled: bool) -> TriggerResult<()> {
        if !is_valid_agent_id(id) {
            return Err(TriggerStoreError::Invalid(format!("id '{}'", id)));
        }
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut vault = self.load_vault()?;
        let key = format!("{}{}", KEY_PREFIX, id);
        let json = match vault.get(&key) {
            Ok(j) => j,
            Err(e) if e.kind() == securestore::ErrorKind::SecretNotFound => {
                return Err(TriggerStoreError::NotFound(id.to_string()));
            }
            Err(e) => return Err(TriggerStoreError::Vault(format!("read failed: {e}"))),
        };
        let mut def: TriggerDef = serde_json::from_str(&json)?;
        def.enabled = enabled;
        vault.set(&key, serde_json::to_string(&def)?);
        vault
            .save()
            .map_err(|e| TriggerStoreError::Vault(format!("save failed: {e}")))?;
        Ok(())
    }

    /// Append one fire record to the trigger's run history.
    pub fn record_run(&self, id: &str, record: &TriggerRunRecord) -> TriggerResult<()> {
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
            sandboxed: true,
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
    fn set_enabled_preserves_other_fields() {
        let tmp = tempdir().unwrap();
        let store = TriggerStore::open(tmp.path());
        store.upsert(&def("t")).unwrap();

        // A separate edit lands...
        let mut edited = store.get("t").unwrap().unwrap();
        edited.code = "echo updated".to_string();
        store.upsert(&edited).unwrap();

        // ...then a toggle: it reads current state and changes only `enabled`,
        // never reverting the code (single-lock read-modify-write).
        store.set_enabled("t", false).unwrap();
        let got = store.get("t").unwrap().unwrap();
        assert_eq!(got.code, "echo updated");
        assert!(!got.enabled);

        assert!(store.set_enabled("missing", true).is_err());
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

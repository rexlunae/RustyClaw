//! Client-frame handlers for messenger setup.
//!
//! Three things are configured here, all of them persisted the moment they
//! change so a gateway restart does not lose them:
//!
//! * **Accounts** — which messengers exist and how they log in. Credentials go
//!   to the vault under `messenger/<account>/<field>`; only the reference is
//!   written to `config.toml`.
//! * **Profile** — the name and description the agent presents on each
//!   messenger, defaulting to the agent's own.
//! * **Routes** — which gateway thread a channel's conversation belongs to.
//!
//! # Secrets travel one way
//!
//! Credential values arrive in [`ClientPayload::MessengerAccountSave`] and go
//! straight to the vault. Nothing here ever puts one in a response frame: the
//! view reports *which* secret fields are set and what vault entry holds them,
//! never the value. A client cannot leak what it was never sent.

use anyhow::Result;
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use rustyclaw_core::agents::MAIN_AGENT_ID;
use rustyclaw_core::config::{Config, MessengerConfig};
use rustyclaw_core::gateway::TransportWriter;
use rustyclaw_core::gateway::protocol::frames::*;
use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::messengers::setup::{
    self, ThreadRoute, kind_spec, secret_name, validate_account_name, validate_fields,
};
use rustyclaw_core::secrets::{AccessPolicy, SecretEntry, SecretKind};
use rustyclaw_core::threads::ThreadManager;

use crate::SharedVault;

/// Handle one messenger-setup frame.
///
/// Every mutation replies with its own result frame *and* a refreshed view, so
/// a client never has to guess what the config now looks like.
pub async fn handle_messenger_config(
    writer: &mut dyn TransportWriter,
    payload: ClientPayload,
    config: &mut Config,
    vault: &SharedVault,
) -> Result<()> {
    match payload {
        ClientPayload::MessengerConfigRequest => {}

        ClientPayload::MessengerAccountSave {
            original_name,
            name,
            messenger_type,
            enabled,
            fields,
            secrets,
            display_name,
            bio,
            avatar_path,
            agent_id,
        } => {
            let result = save_account(
                config,
                vault,
                original_name,
                name.clone(),
                messenger_type,
                enabled,
                fields,
                secrets,
                display_name,
                bio,
                avatar_path,
                agent_id,
            )
            .await;
            send_account_result(writer, &name, result).await?;
        }

        ClientPayload::MessengerAccountDelete { name } => {
            let result = delete_account(config, vault, &name).await;
            send_account_result(writer, &name, result).await?;
        }

        ClientPayload::MessengerSecretsMigrate { name } => {
            let result = migrate_secrets(config, vault, &name).await;
            send_account_result(writer, &name, result).await?;
        }

        ClientPayload::MessengerRouteSave {
            messenger,
            channel,
            thread_id,
            agent_id,
            enabled,
        } => {
            let result = save_route(config, messenger, channel, thread_id, agent_id, enabled);
            send_route_result(writer, result).await?;
        }

        ClientPayload::MessengerRouteDelete { messenger, channel } => {
            let result = delete_route(config, &messenger, channel.as_deref());
            send_route_result(writer, result).await?;
        }

        other => {
            warn!(
                ?other,
                "Non-messenger payload reached the messenger handler"
            );
            return Ok(());
        }
    }

    send_frame(writer, &build_view(config, vault).await).await
}

// ── View ────────────────────────────────────────────────────────────────────

/// Whether this build can run a messenger type.
///
/// The schema lists every type the project supports; whether *this* binary can
/// run one depends on the features it was compiled with. Reporting the gap
/// explicitly lets a client say "rebuild with `--features matrix`" instead of
/// hiding Matrix and leaving the user to wonder where it went.
fn feature_available(feature: Option<&str>) -> bool {
    match feature {
        None => true,
        Some("matrix") => cfg!(feature = "matrix"),
        Some("whatsapp") => cfg!(feature = "whatsapp"),
        Some("signal-cli") => cfg!(feature = "signal-cli"),
        // An unrecognised feature name is a schema entry this build was never
        // taught about; call it unavailable rather than promising it works.
        Some(_) => false,
    }
}

/// Messenger types this gateway can actually connect.
fn available_kinds() -> Vec<String> {
    setup::KINDS
        .iter()
        .filter(|k| feature_available(k.feature))
        .map(|k| k.id.to_string())
        .collect()
}

/// Every thread a route may point at, across all agents.
///
/// Read straight off disk rather than from a connection's in-memory manager:
/// routes are gateway-wide and may name a thread belonging to an agent this
/// connection has not switched to.
fn routable_threads(config: &Config) -> Vec<RoutableThreadDto> {
    let mut out = Vec::new();
    for agent in config.agent_registry().list() {
        let path = config.sessions_dir_for(&agent.id).join("threads.json");
        let mgr = ThreadManager::load_or_default(&path);
        for thread in mgr.list() {
            out.push(RoutableThreadDto {
                thread_id: thread.id.0,
                label: thread.label.clone(),
                agent_id: agent.id.clone(),
            });
        }
    }
    out
}

/// Build one account's wire view. Secret *values* are never included.
async fn account_dto(
    messenger: &MessengerConfig,
    config: &Config,
    vault: &SharedVault,
    stored: &[String],
) -> MessengerAccountDto {
    let spec = kind_spec(&messenger.messenger_type);

    // Non-secret fields only. A secret's value is in the vault (or, for a
    // not-yet-migrated account, in plaintext config) and either way it is not
    // this frame's business.
    let fields: BTreeMap<String, String> = spec
        .map(|s| {
            s.fields
                .iter()
                .filter(|f| !f.is_secret())
                .filter_map(|f| {
                    messenger
                        .field_value(f.name)
                        .map(|v| (f.name.to_string(), v))
                })
                .collect()
        })
        .unwrap_or_default();

    // Only report a reference the vault actually still has. A dangling
    // `secret_refs` entry — vault wiped, config kept — would otherwise render
    // as a configured credential right up until the connection fails.
    //
    // `list_secrets` returns raw vault keys, and a typed credential is stored
    // as the pair `cred:<name>` / `val:<name>`. Comparing against the bare
    // name matches neither, which is how this managed to report *every*
    // account as uncredentialed.
    let vaulted: BTreeMap<String, String> = messenger
        .secret_refs
        .iter()
        .filter(|(_, cred)| {
            let value_key = format!("val:{cred}");
            stored.contains(&value_key)
        })
        .map(|(field, cred)| (field.clone(), cred.clone()))
        .collect();

    let profile = messenger.profile.clone().unwrap_or_default();
    let (agent_name, agent_description) = agent_identity(config, profile.agent_id.as_deref());
    let resolved = profile.resolve(&agent_name, agent_description.as_deref());

    let (available, unavailable_reason) = match spec {
        None => (
            false,
            Some(format!(
                "Unknown messenger type '{}'",
                messenger.messenger_type
            )),
        ),
        Some(s) if !feature_available(s.feature) => (
            false,
            s.feature.map(|f| {
                format!(
                    "This gateway was built without the '{f}' feature; rebuild with --features {f}"
                )
            }),
        ),
        Some(_) => (true, None),
    };

    let _ = vault; // Values are deliberately not read here.

    MessengerAccountDto {
        name: messenger.name.clone(),
        messenger_type: messenger.messenger_type.clone(),
        enabled: messenger.enabled,
        fields,
        vaulted,
        plaintext: messenger
            .plaintext_credentials()
            .into_iter()
            .map(|p| (p.field.to_string(), p.label.to_string()))
            .collect(),
        profile: MessengerProfileDto {
            display_name: resolved.display_name,
            bio: resolved.bio,
            avatar_path: resolved.avatar_path,
            agent_id: resolved.agent_id,
            display_name_overridden: profile.display_name.is_some(),
            bio_overridden: profile.bio.is_some(),
        },
        available,
        unavailable_reason,
    }
}

/// An agent's display name and description, for profile fallback.
fn agent_identity(config: &Config, agent_id: Option<&str>) -> (String, Option<String>) {
    let id = agent_id.unwrap_or(MAIN_AGENT_ID);
    match config.agent_registry().get(id) {
        Some(info) => (info.name, info.description),
        // A profile pinned to a deleted agent falls back to the installation
        // name rather than rendering blank.
        None => (config.agent_name.clone(), None),
    }
}

/// The full setup view: accounts, routes, routable threads, available kinds.
async fn build_view(config: &Config, vault: &SharedVault) -> ServerFrame {
    let stored = {
        let mut mgr = vault.lock().await;
        mgr.list_secrets()
    };

    let mut accounts = Vec::with_capacity(config.messengers.len());
    for messenger in &config.messengers {
        accounts.push(account_dto(messenger, config, vault, &stored).await);
    }

    let threads = routable_threads(config);
    let routes = config
        .messenger_routes
        .iter()
        .map(|r| ThreadRouteDto {
            messenger: r.messenger.clone(),
            channel: r.channel.clone(),
            thread_id: r.thread_id,
            agent_id: r.agent().to_string(),
            enabled: r.enabled,
            thread_label: threads
                .iter()
                .find(|t| t.thread_id == r.thread_id && t.agent_id == r.agent())
                .map(|t| t.label.clone()),
        })
        .collect();

    ServerFrame {
        frame_type: ServerFrameType::MessengerConfigResult,
        payload: ServerPayload::MessengerConfigResult {
            accounts,
            routes,
            threads,
            available_kinds: available_kinds(),
        },
    }
}

// ── Accounts ────────────────────────────────────────────────────────────────

/// What a successful mutation has to say for itself.
type Outcome = Result<Option<String>, Vec<String>>;

#[allow(clippy::too_many_arguments)]
async fn save_account(
    config: &mut Config,
    vault: &SharedVault,
    original_name: Option<String>,
    name: String,
    messenger_type: String,
    enabled: bool,
    fields: Vec<(String, String)>,
    secrets: Vec<(String, String)>,
    display_name: Option<String>,
    bio: Option<String>,
    avatar_path: Option<std::path::PathBuf>,
    agent_id: Option<String>,
) -> Outcome {
    let name = name.trim().to_string();
    validate_account_name(&name).map_err(|e| vec![e])?;

    let Some(spec) = kind_spec(&messenger_type) else {
        return Err(vec![format!("Unknown messenger type: '{messenger_type}'")]);
    };
    if !feature_available(spec.feature) {
        return Err(vec![format!(
            "This gateway was built without support for {}",
            spec.label
        )]);
    }

    // Renaming to a name already in use would leave two accounts fighting over
    // one set of vault keys, so it is refused rather than merged.
    let clashes = config
        .messengers
        .iter()
        .any(|m| m.name == name && Some(&m.name) != original_name.as_ref());
    if clashes {
        return Err(vec![format!("An account named '{name}' already exists")]);
    }

    // Start from the existing entry when editing so untouched fields survive.
    let mut entry = original_name
        .as_deref()
        .and_then(|orig| config.messengers.iter().find(|m| m.name == orig))
        .cloned()
        .unwrap_or_default();

    // A type change invalidates the old credentials: they belong to a backend
    // that is no longer in play. Drop the references rather than carrying
    // stale ones forward.
    if !entry.messenger_type.is_empty() && entry.messenger_type != messenger_type {
        entry.secret_refs.clear();
    }

    entry.name = name.clone();
    entry.messenger_type = messenger_type.clone();
    entry.enabled = enabled;

    let mut errors = Vec::new();
    for (field, value) in &fields {
        match setup::field_spec(&messenger_type, field) {
            Some(f) if f.is_secret() => {
                // A secret arriving through the non-secret channel would be
                // written to config.toml. Refuse rather than silently persist.
                errors.push(format!(
                    "{} is a credential and must be sent as one",
                    f.label
                ));
            }
            Some(_) => {
                if let Err(e) = entry.set_field(field, value) {
                    errors.push(e);
                }
            }
            None => errors.push(format!("'{field}' is not a field on {}", spec.label)),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // Stage credentials before touching config: a vault write that fails must
    // not leave config pointing at a credential that was never stored.
    let mut staged: Vec<(String, String, String)> = Vec::new();
    for (field, value) in secrets {
        let Some(f) = setup::field_spec(&messenger_type, &field) else {
            return Err(vec![format!("'{field}' is not a field on {}", spec.label)]);
        };
        if !f.is_secret() {
            return Err(vec![format!("{} is not a credential field", f.label)]);
        }
        if value.trim().is_empty() {
            continue;
        }
        staged.push((field.clone(), f.label.to_string(), value));
    }

    // Everything present counts: values arriving now, and credentials already
    // in the vault from a previous save.
    let arriving: Vec<&str> = staged.iter().map(|(f, _, _)| f.as_str()).collect();
    validate_fields(&messenger_type, |field| {
        arriving.contains(&field)
            || entry.field_is_set(field)
            || fields
                .iter()
                .any(|(name, value)| name == field && !value.trim().is_empty())
    })?;

    {
        let mut mgr = vault.lock().await;
        for (field, label, value) in &staged {
            let cred = secret_name(&name, field);
            let secret_entry = SecretEntry {
                label: format!("{name} — {label}"),
                kind: SecretKind::Token,
                // The gateway reads this through `read_service_credential`;
                // the agent has no business reading its own bot token, so the
                // policy denies the agent-facing path outright.
                policy: AccessPolicy::WithAuth,
                description: Some(format!("{} credential for messenger '{name}'", spec.label)),
                disabled: false,
            };
            mgr.store_credential(&cred, &secret_entry, value, None)
                .map_err(|e| vec![format!("Could not store {label} in the vault: {e}")])?;
            entry.secret_refs.insert(field.clone(), cred);
            // The vault now owns it; leaving the plaintext twin behind would
            // defeat the entire point of moving it.
            let _ = entry.set_field(field, "");
        }
    }

    // Merge the profile rather than replacing it.
    //
    // `None` means "the caller did not send this field", not "clear it".
    // Enabling an account sends no profile at all, and replacing the stored
    // profile with that would silently delete the name and description the
    // user configured — for an operation that has nothing to do with either.
    // An explicit empty string is how a field is cleared, matching the wire
    // documentation on `ClientPayload::MessengerAccountSave`.
    let mut profile = entry.profile.clone().unwrap_or_default();
    if let Some(value) = display_name {
        profile.display_name = override_of(Some(value));
    }
    if let Some(value) = bio {
        profile.bio = override_of(Some(value));
    }
    if let Some(path) = avatar_path {
        profile.avatar_path = (!path.as_os_str().is_empty()).then_some(path);
    }
    if let Some(id) = agent_id {
        profile.agent_id = override_of(Some(id));
    }
    entry.profile = (!profile.is_empty()).then_some(profile);

    // Renaming moves the vault entries too, so credentials follow the account
    // rather than being orphaned under the old name.
    if let Some(orig) = original_name.as_deref().filter(|o| *o != name) {
        rename_credentials(&mut entry, vault, orig, &name).await?;
    }

    match original_name
        .as_deref()
        .and_then(|orig| config.messengers.iter_mut().find(|m| m.name == orig))
    {
        Some(existing) => *existing = entry,
        None => config.messengers.push(entry),
    }

    // Routes are keyed by account name, so a rename has to carry them along or
    // they silently stop matching.
    if let Some(orig) = original_name.as_deref().filter(|o| *o != name) {
        for route in &mut config.messenger_routes {
            if route.messenger == orig {
                route.messenger = name.clone();
            }
        }
    }

    persist(config)?;
    info!(account = %name, messenger_type = %messenger_type, "Messenger account saved");
    Ok(Some(match staged.is_empty() {
        true => format!("Saved '{name}'"),
        false => format!(
            "Saved '{name}'; {} credential(s) stored in the vault",
            staged.len()
        ),
    }))
}

/// Move an account's vault entries so they follow it to a new name.
///
/// Vault keys embed the account name, so a rename that skipped this would
/// leave every credential orphaned under the old key and the account unable to
/// log in — with config still cheerfully claiming the token is set.
///
/// `from` is unused beyond documenting intent: the current reference is read
/// from `secret_refs`, which is authoritative even if an earlier rename left
/// it pointing somewhere unexpected.
async fn rename_credentials(
    entry: &mut MessengerConfig,
    vault: &SharedVault,
    from: &str,
    to: &str,
) -> Result<(), Vec<String>> {
    debug!(%from, %to, "Moving messenger credentials to follow a rename");
    let mut mgr = vault.lock().await;

    let moves: Vec<(String, String, String)> = entry
        .secret_refs
        .iter()
        .map(|(field, current)| (field.clone(), current.clone(), secret_name(to, field)))
        .filter(|(_, current, new)| current != new)
        .collect();

    for (field, old, new) in moves {
        let value = mgr
            .read_service_credential(&old)
            .map_err(|e| vec![format!("Could not read '{old}' while renaming: {e}")])?;
        // Nothing under the old key: the reference was already stale, so
        // repointing it at the new name is the most honest thing available.
        let Some(value) = value else {
            entry.secret_refs.insert(field, new);
            continue;
        };
        let secret_entry = SecretEntry {
            label: format!("{to} — {field}"),
            kind: SecretKind::Token,
            policy: AccessPolicy::WithAuth,
            description: Some(format!("Credential for messenger '{to}'")),
            disabled: false,
        };
        mgr.store_credential(&new, &secret_entry, &value, None)
            .map_err(|e| vec![format!("Could not write '{new}' while renaming: {e}")])?;
        // Only after the copy landed — the reverse order loses the credential
        // if the write fails.
        let _ = mgr.delete_credential(&old);
        entry.secret_refs.insert(field, new);
    }
    Ok(())
}

async fn delete_account(config: &mut Config, vault: &SharedVault, name: &str) -> Outcome {
    let Some(index) = config.messengers.iter().position(|m| m.name == name) else {
        return Err(vec![format!("No account named '{name}'")]);
    };
    let removed = config.messengers.remove(index);

    {
        let mut mgr = vault.lock().await;
        for cred in removed.secret_refs.values() {
            if let Err(e) = mgr.delete_credential(cred) {
                // The account is already gone from config; a stuck vault entry
                // is untidy, not dangerous, and worth a log rather than a
                // failed delete the user cannot retry.
                warn!(credential = %cred, error = %e, "Could not remove messenger credential");
            }
        }
    }

    // A route pointing at a deleted account matches nothing and would sit in
    // the table as a permanent puzzle.
    let before = config.messenger_routes.len();
    config.messenger_routes.retain(|r| r.messenger != name);
    let dropped = before - config.messenger_routes.len();

    persist(config)?;
    info!(account = %name, routes_removed = dropped, "Messenger account deleted");
    Ok(Some(match dropped {
        0 => format!("Deleted '{name}'"),
        n => format!("Deleted '{name}' and {n} route(s)"),
    }))
}

/// Move an account's plaintext credentials into the vault.
async fn migrate_secrets(config: &mut Config, vault: &SharedVault, name: &str) -> Outcome {
    let Some(entry) = config.messengers.iter_mut().find(|m| m.name == name) else {
        return Err(vec![format!("No account named '{name}'")]);
    };
    let pending = entry.plaintext_credentials();
    if pending.is_empty() {
        return Ok(Some(format!("'{name}' has no plaintext credentials")));
    }

    let label = kind_spec(&entry.messenger_type).map_or("Messenger", |s| s.label);
    let mut moved = Vec::new();
    {
        let mut mgr = vault.lock().await;
        for field in &pending {
            let Some(value) = entry.field_value(field.field) else {
                continue;
            };
            let cred = secret_name(name, field.field);
            let secret_entry = SecretEntry {
                label: format!("{name} — {}", field.label),
                kind: SecretKind::Token,
                policy: AccessPolicy::WithAuth,
                description: Some(format!("{label} credential for messenger '{name}'")),
                disabled: false,
            };
            mgr.store_credential(&cred, &secret_entry, &value, None)
                .map_err(|e| vec![format!("Could not store {}: {e}", field.label)])?;
            entry.secret_refs.insert(field.field.to_string(), cred);
            // Clear only after the vault write succeeded — the ordering is the
            // difference between migrating a credential and losing one.
            let _ = entry.set_field(field.field, "");
            moved.push(field.label);
        }
    }

    persist(config)?;
    info!(account = %name, count = moved.len(), "Messenger credentials moved to the vault");
    Ok(Some(format!(
        "Moved {} into the vault for '{name}'",
        moved.join(", ")
    )))
}

// ── Routes ──────────────────────────────────────────────────────────────────

fn save_route(
    config: &mut Config,
    messenger: String,
    channel: Option<String>,
    thread_id: u64,
    agent_id: Option<String>,
    enabled: bool,
) -> Result<Option<String>, String> {
    if !config.messengers.iter().any(|m| m.name == messenger) {
        return Err(format!("No messenger account named '{messenger}'"));
    }

    let agent = agent_id
        .filter(|a| !a.trim().is_empty())
        .unwrap_or_else(|| MAIN_AGENT_ID.to_string());

    // A route to a thread that does not exist would silently never fire, and
    // the user would have no way to tell that from a working one.
    let threads = routable_threads(config);
    let target = threads
        .iter()
        .find(|t| t.thread_id == thread_id && t.agent_id == agent);
    let Some(target) = target else {
        return Err(format!("Agent '{agent}' has no thread #{thread_id}"));
    };
    let label = target.label.clone();

    let channel = channel.filter(|c| !c.trim().is_empty());
    let route = ThreadRoute {
        messenger: messenger.clone(),
        channel: channel.clone(),
        thread_id,
        agent_id: Some(agent),
        enabled,
    };

    match config
        .messenger_routes
        .iter_mut()
        .find(|r| r.messenger == messenger && r.channel == channel)
    {
        Some(existing) => *existing = route,
        None => config.messenger_routes.push(route),
    }

    persist(config).map_err(|e| e.join("; "))?;
    debug!(%messenger, ?channel, thread_id, "Messenger route saved");
    Ok(Some(match channel {
        Some(c) => format!("{messenger} {c} → {label}"),
        None => format!("{messenger} (all channels) → {label}"),
    }))
}

fn delete_route(
    config: &mut Config,
    messenger: &str,
    channel: Option<&str>,
) -> Result<Option<String>, String> {
    let channel = channel.filter(|c| !c.trim().is_empty());
    let before = config.messenger_routes.len();
    config
        .messenger_routes
        .retain(|r| !(r.messenger == messenger && r.channel.as_deref() == channel));
    if config.messenger_routes.len() == before {
        return Err("No such route".to_string());
    }
    persist(config).map_err(|e| e.join("; "))?;
    Ok(Some("Route removed".to_string()))
}

// ── Shared plumbing ─────────────────────────────────────────────────────────

/// Treat a blank override as "clear it" rather than as an empty name.
fn override_of(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn persist(config: &Config) -> Result<(), Vec<String>> {
    config
        .save(None)
        .map_err(|e| vec![format!("Could not save config: {e}")])
}

async fn send_account_result(
    writer: &mut dyn TransportWriter,
    name: &str,
    result: Outcome,
) -> Result<()> {
    let (ok, errors, message) = match result {
        Ok(message) => (true, Vec::new(), message),
        Err(errors) => (false, errors, None),
    };
    send_frame(
        writer,
        &ServerFrame {
            frame_type: ServerFrameType::MessengerAccountResult,
            payload: ServerPayload::MessengerAccountResult {
                ok,
                name: name.to_string(),
                errors,
                message,
            },
        },
    )
    .await
}

async fn send_route_result(
    writer: &mut dyn TransportWriter,
    result: Result<Option<String>, String>,
) -> Result<()> {
    let (ok, message) = match result {
        Ok(message) => (true, message),
        Err(error) => (false, Some(error)),
    };
    send_frame(
        writer,
        &ServerFrame {
            frame_type: ServerFrameType::MessengerRouteResult,
            payload: ServerPayload::MessengerRouteResult { ok, message },
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// A config and a real (temp) vault.
    fn fixture(dir: &std::path::Path) -> (Config, SharedVault) {
        let mut config = Config {
            settings_dir: dir.to_path_buf(),
            agent_name: "Ada".to_string(),
            ..Config::default()
        };
        config.settings_dir = dir.to_path_buf();
        config.credentials_dir = Some(dir.join("credentials"));
        std::fs::create_dir_all(config.credentials_dir()).unwrap();
        let vault = Arc::new(Mutex::new(rustyclaw_core::secrets::SecretsManager::new(
            config.credentials_dir(),
        )));
        (config, vault)
    }

    /// `save_account` with the arguments a "create Telegram account" form sends.
    async fn save_telegram(
        config: &mut Config,
        vault: &SharedVault,
        name: &str,
        token: Option<&str>,
    ) -> Outcome {
        save_account(
            config,
            vault,
            None,
            name.to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            token
                .map(|t| vec![("token".to_string(), t.to_string())])
                .unwrap_or_default(),
            None,
            None,
            None,
            None,
        )
        .await
    }

    #[tokio::test]
    async fn a_saved_credential_goes_to_the_vault_and_not_to_config() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .expect("save should succeed");

        let account = &config.messengers[0];
        assert_eq!(
            account.token, None,
            "the token must not survive in config: {:?}",
            account.token
        );
        assert_eq!(
            account.secret_ref("token"),
            Some("messenger/tg/token"),
            "config should hold a reference instead"
        );

        let stored = vault
            .lock()
            .await
            .read_service_credential("messenger/tg/token")
            .unwrap();
        assert_eq!(stored.as_deref(), Some("123:secret"));
    }

    #[tokio::test]
    async fn a_missing_required_credential_is_refused_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        let errors = save_telegram(&mut config, &vault, "tg", None)
            .await
            .unwrap_err();
        assert_eq!(errors, vec!["Bot token is required"]);
        assert!(
            config.messengers.is_empty(),
            "a rejected save must not half-create an account"
        );
    }

    #[tokio::test]
    async fn re_saving_without_the_token_keeps_the_stored_one() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap();

        // The editor sends no secret when the user leaves the field blank.
        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "tg".to_string(),
            "telegram".to_string(),
            false,
            Vec::new(),
            Vec::new(),
            Some("SupportBot".to_string()),
            None,
            None,
            None,
        )
        .await
        .expect("editing without retyping the token must be allowed");

        assert!(
            !config.messengers[0].enabled,
            "the edit should have applied"
        );
        assert_eq!(
            vault
                .lock()
                .await
                .read_service_credential("messenger/tg/token")
                .unwrap()
                .as_deref(),
            Some("123:secret"),
            "the credential must survive an unrelated edit"
        );
    }

    #[tokio::test]
    async fn a_credential_sent_as_a_plain_field_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        let errors = save_account(
            &mut config,
            &vault,
            None,
            "tg".to_string(),
            "telegram".to_string(),
            true,
            // A token arriving here would be written straight to config.toml.
            vec![("token".to_string(), "123:secret".to_string())],
            Vec::new(),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap_err();

        assert!(
            errors.iter().any(|e| e.contains("must be sent as one")),
            "{errors:?}"
        );
        assert!(config.messengers.is_empty());
    }

    #[tokio::test]
    async fn renaming_an_account_moves_its_credential_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap();

        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "telegram-main".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("rename should succeed");

        assert_eq!(config.messengers.len(), 1, "rename must not duplicate");
        assert_eq!(
            config.messengers[0].secret_ref("token"),
            Some("messenger/telegram-main/token")
        );
        let mut mgr = vault.lock().await;
        assert_eq!(
            mgr.read_service_credential("messenger/telegram-main/token")
                .unwrap()
                .as_deref(),
            Some("123:secret"),
            "the credential must follow the account"
        );
        assert_eq!(
            mgr.read_service_credential("messenger/tg/token").unwrap(),
            None,
            "the old vault entry must not linger"
        );
    }

    #[tokio::test]
    async fn renaming_an_account_carries_its_routes() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();
        config.messenger_routes = vec![ThreadRoute {
            messenger: "tg".into(),
            channel: Some("-100".into()),
            thread_id: 1,
            agent_id: None,
            enabled: true,
        }];

        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "renamed".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            config.messenger_routes[0].messenger, "renamed",
            "a route left pointing at the old name would silently stop matching"
        );
    }

    #[tokio::test]
    async fn renaming_onto_an_existing_account_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "one", Some("a"))
            .await
            .unwrap();
        save_telegram(&mut config, &vault, "two", Some("b"))
            .await
            .unwrap();

        let errors = save_account(
            &mut config,
            &vault,
            Some("two".to_string()),
            "one".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap_err();

        assert!(errors[0].contains("already exists"), "{errors:?}");
        assert_eq!(config.messengers.len(), 2);
    }

    #[tokio::test]
    async fn migrating_moves_a_plaintext_token_and_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        // An account as an older config would have written it.
        config.messengers.push(MessengerConfig {
            name: "legacy".into(),
            messenger_type: "telegram".into(),
            enabled: true,
            token: Some("123:plaintext".into()),
            ..Default::default()
        });

        let message = migrate_secrets(&mut config, &vault, "legacy")
            .await
            .expect("migration should succeed");
        assert!(message.unwrap().contains("Bot token"));

        let account = &config.messengers[0];
        assert_eq!(account.token, None, "the plaintext copy must be cleared");
        assert_eq!(account.secret_ref("token"), Some("messenger/legacy/token"));
        assert_eq!(
            vault
                .lock()
                .await
                .read_service_credential("messenger/legacy/token")
                .unwrap()
                .as_deref(),
            Some("123:plaintext")
        );
    }

    #[tokio::test]
    async fn migrating_an_account_with_nothing_to_move_says_so_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        let message = migrate_secrets(&mut config, &vault, "tg").await.unwrap();
        assert!(message.unwrap().contains("no plaintext"));
    }

    #[tokio::test]
    async fn deleting_an_account_takes_its_credentials_and_routes_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap();
        config.messenger_routes = vec![ThreadRoute {
            messenger: "tg".into(),
            channel: None,
            thread_id: 1,
            agent_id: None,
            enabled: true,
        }];

        delete_account(&mut config, &vault, "tg").await.unwrap();

        assert!(config.messengers.is_empty());
        assert!(
            config.messenger_routes.is_empty(),
            "a route to a deleted account matches nothing and only confuses"
        );
        assert_eq!(
            vault
                .lock()
                .await
                .read_service_credential("messenger/tg/token")
                .unwrap(),
            None,
            "the credential must not outlive the account that used it"
        );
    }

    #[tokio::test]
    async fn changing_an_accounts_type_drops_credentials_that_no_longer_apply() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "acct", Some("123:secret"))
            .await
            .unwrap();

        // Telegram → IRC: the bot token means nothing to an IRC server.
        save_account(
            &mut config,
            &vault,
            Some("acct".to_string()),
            "acct".to_string(),
            "irc".to_string(),
            true,
            vec![("server".to_string(), "irc.libera.chat".to_string())],
            Vec::new(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("switching type should succeed");

        assert_eq!(config.messengers[0].secret_ref("token"), None);
        assert_eq!(
            config.messengers[0].server.as_deref(),
            Some("irc.libera.chat")
        );
    }

    #[tokio::test]
    async fn a_route_to_a_nonexistent_thread_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        let error = save_route(&mut config, "tg".into(), None, 4242, None, true).unwrap_err();
        assert!(error.contains("no thread #4242"), "{error}");
        assert!(config.messenger_routes.is_empty());
    }

    #[tokio::test]
    async fn a_route_to_an_unknown_account_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, _vault) = fixture(dir.path());
        let error = save_route(&mut config, "ghost".into(), None, 1, None, true).unwrap_err();
        assert!(error.contains("No messenger account"), "{error}");
    }

    #[tokio::test]
    async fn a_second_save_for_one_channel_updates_rather_than_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        // Two threads, so the route has somewhere to move to.
        let sessions = config.sessions_dir_for(MAIN_AGENT_ID);
        std::fs::create_dir_all(&sessions).unwrap();
        let mut mgr = ThreadManager::new();
        let first = mgr.create_chat("First");
        let second = mgr.create_chat("Second");
        mgr.save_to_file(&sessions.join("threads.json")).unwrap();

        save_route(
            &mut config,
            "tg".into(),
            Some("-100".into()),
            first.0,
            None,
            true,
        )
        .unwrap();
        save_route(
            &mut config,
            "tg".into(),
            Some("-100".into()),
            second.0,
            None,
            true,
        )
        .unwrap();

        assert_eq!(
            config.messenger_routes.len(),
            1,
            "identity is (account, channel)"
        );
        assert_eq!(config.messenger_routes[0].thread_id, second.0);
    }

    /// Pull the accounts out of a `build_view` frame.
    fn accounts_in(frame: &ServerFrame) -> Vec<MessengerAccountDto> {
        match &frame.payload {
            ServerPayload::MessengerConfigResult { accounts, .. } => accounts.clone(),
            other => panic!("expected a config result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_view_reports_a_saved_credential_as_vaulted() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap();

        // The vault lists raw keys (`val:messenger/tg/token`) while config
        // holds the bare name, so this is where a prefix mismatch shows up as
        // "every account has no credentials".
        let accounts = accounts_in(&build_view(&config, &vault).await);
        assert_eq!(
            accounts[0].vaulted.get("token").map(String::as_str),
            Some("messenger/tg/token"),
            "a credential that was just saved must read as stored: {:?}",
            accounts[0].vaulted
        );
    }

    #[tokio::test]
    async fn the_view_does_not_claim_a_credential_the_vault_no_longer_has() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap();
        // Config keeps pointing at it; the vault does not have it any more.
        vault
            .lock()
            .await
            .delete_credential("messenger/tg/token")
            .unwrap();

        let accounts = accounts_in(&build_view(&config, &vault).await);
        assert!(
            accounts[0].vaulted.is_empty(),
            "a dangling reference must not render as a configured credential"
        );
    }

    #[tokio::test]
    async fn toggling_an_account_leaves_its_profile_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();
        // Give it a presented identity.
        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "tg".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            Vec::new(),
            Some("SupportBot".to_string()),
            Some("Ask me about billing".to_string()),
            None,
            None,
        )
        .await
        .unwrap();

        // A toggle sends no profile fields at all.
        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "tg".to_string(),
            "telegram".to_string(),
            false,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let profile = config.messengers[0].profile.clone().unwrap_or_default();
        assert_eq!(
            profile.display_name.as_deref(),
            Some("SupportBot"),
            "disabling an account must not erase the name it presents"
        );
        assert_eq!(profile.bio.as_deref(), Some("Ask me about billing"));
        assert!(
            !config.messengers[0].enabled,
            "the toggle should still apply"
        );
    }

    #[tokio::test]
    async fn an_explicit_blank_clears_a_profile_override() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();
        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "tg".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            Vec::new(),
            Some("SupportBot".to_string()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // Blank, not absent: the user emptied the row.
        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "tg".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            Vec::new(),
            Some(String::new()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let profile = config.messengers[0].profile.clone().unwrap_or_default();
        assert_eq!(
            profile.display_name, None,
            "an emptied row must fall back to the agent's name"
        );
    }

    #[tokio::test]
    async fn an_edit_that_sends_no_avatar_keeps_the_configured_one() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();
        let avatar = dir.path().join("face.png");
        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "tg".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            Vec::new(),
            None,
            None,
            Some(avatar.clone()),
            None,
        )
        .await
        .unwrap();

        // The editor cannot pre-fill the avatar row, so an ordinary save
        // sends None — which must not be read as "remove the picture".
        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "tg".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            Vec::new(),
            Some("Ada".to_string()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            config.messengers[0]
                .profile
                .as_ref()
                .and_then(|p| p.avatar_path.clone()),
            Some(avatar)
        );
    }

    #[test]
    fn an_unknown_feature_name_reads_as_unavailable_rather_than_supported() {
        assert!(feature_available(None));
        assert!(!feature_available(Some("time-travel")));
    }
}

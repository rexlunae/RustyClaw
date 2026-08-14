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
use rustyclaw_core::ignore::Ignore;
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

use crate::SharedVault;

/// Handle one messenger-setup frame.
///
/// Every mutation replies with its own result frame *and* a refreshed view, so
/// a client never has to guess what the config now looks like.
///
/// Mutations run against the gateway's **shared** config, not the caller's
/// connection snapshot. Persisting the snapshot looked identical in tests
/// with one connection, but the shared copy never learned about the change —
/// so the next handler that persisted *it* (an agent rename, a model switch)
/// rewrote `config.toml` with every messenger account and route missing,
/// while their credentials lived on in the vault. The connection snapshot is
/// re-synced from the shared copy on the way out.
pub async fn handle_messenger_config(
    writer: &mut dyn TransportWriter,
    payload: ClientPayload,
    conn_config: &mut Config,
    shared_config: &crate::SharedConfig,
    vault: &SharedVault,
) -> Result<()> {
    /// A mutation's outcome, carried past the settings lock to the transport.
    enum Reply {
        /// From an account save, delete, or migration.
        Account { name: String, outcome: Outcome },
        /// From a route save or delete.
        Route(Result<Option<String>, String>),
        /// A plain view request has no result frame of its own.
        None,
    }

    // Held across the whole mutation so two clients editing at once serialise
    // instead of interleaving read-modify-write on the settings file — but
    // NOT across any transport write. A frame send can block on a client
    // that is slow to drain, and stalling every other connection's settings
    // access on one laggard is the documented deadlock shape this codebase
    // has hit before. Results are computed here and sent after the guard is
    // dropped.
    let mut shared = shared_config.write().await;
    let config = &mut *shared;

    let reply = match payload {
        ClientPayload::MessengerConfigRequest => Reply::None,

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
            // Out of the redacting wire type and into plain pairs; from here
            // the values only travel toward the vault.
            let secrets = secrets
                .into_iter()
                .map(|(field, value)| (field, value.into_inner()))
                .collect();
            let outcome = save_account(
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
            Reply::Account { name, outcome }
        }

        ClientPayload::MessengerAccountDelete { name } => {
            let outcome = delete_account(config, vault, &name).await;
            Reply::Account { name, outcome }
        }

        ClientPayload::MessengerSecretsMigrate { name } => {
            let outcome = migrate_secrets(config, vault, &name).await;
            Reply::Account { name, outcome }
        }

        ClientPayload::MessengerRouteSave {
            messenger,
            channel,
            thread_id,
            agent_id,
            enabled,
        } => Reply::Route(save_route(
            config, messenger, channel, thread_id, agent_id, enabled,
        )),

        ClientPayload::MessengerRouteDelete { messenger, channel } => {
            Reply::Route(delete_route(config, &messenger, channel.as_deref()))
        }

        // Deliberately not logging the payload itself: `ClientPayload`'s
        // derived `Debug` prints credential-bearing variants (`SecretsStore`,
        // `MessengerAccountSave.secrets`, …) verbatim, and while today's one
        // caller only routes messenger frames here, a catch-all that logs
        // secrets is one refactor away from doing so in production.
        _ => {
            warn!("Non-messenger payload reached the messenger handler");
            return Ok(());
        }
    };

    // The connection's own snapshot drives everything else this connection
    // does (prompts, messenger loops started from it); leaving it behind the
    // shared copy would undo the fix above from this connection's viewpoint.
    conn_config.messengers = config.messengers.clone();
    conn_config.messenger_routes = config.messenger_routes.clone();

    let view = build_view(config, vault).await;
    drop(shared);

    match reply {
        Reply::Account { name, outcome } => send_account_result(writer, &name, outcome).await?,
        Reply::Route(result) => send_route_result(writer, result).await?,
        Reply::None => {}
    }
    send_frame(writer, &view).await
}

// ── View ────────────────────────────────────────────────────────────────────

/// Whether this build can run a messenger type.
///
/// The schema lists every type the project supports; whether *this* binary can
/// run one depends on the features it was compiled with. Reporting the gap
/// explicitly lets a client say "rebuild with `--features matrix`" instead of
/// hiding Matrix and leaving the user to wonder where it went.
/// A kind's schema from the whole vocabulary this process knows: the
/// registry first (in-tree kinds this build can run, plus anything plugins
/// registered), falling back to the static table so a feature-off builtin
/// still validates and renders instead of becoming "unknown".
fn known_spec(kind: &str) -> Option<rustyclaw_core::messengers::setup::KindSpec> {
    rustyclaw_core::messengers::messenger_registry()
        .spec(kind)
        .or_else(|| kind_spec(kind).cloned())
}

/// Whether this gateway can actually construct the kind — i.e. it is in the
/// registry. The old cfg!-based feature filter lives inside the registry now:
/// a feature-gated builtin simply never registers in a build without it.
fn kind_available(kind: &str) -> bool {
    rustyclaw_core::messengers::messenger_registry()
        .spec(kind)
        .is_some()
}

/// One field of a known kind, plugin kinds included.
fn known_field(kind: &str, field: &str) -> Option<rustyclaw_core::messengers::setup::FieldSpec> {
    known_spec(kind).and_then(|s| s.fields.iter().find(|f| f.name == field).cloned())
}

/// Messenger types this gateway can actually connect.
fn available_kinds() -> Vec<String> {
    rustyclaw_core::messengers::messenger_registry().kind_ids()
}

/// Every thread a route may point at, across all agents.
///
/// Read straight off disk rather than from a connection's in-memory manager:
/// routes are gateway-wide and may name a thread belonging to an agent this
/// connection has not switched to.
fn routable_threads(config: &Config) -> Vec<RoutableThreadDto> {
    let mut out = Vec::new();
    for agent in config.agent_registry().list() {
        // A read-only peek, not a load: enumerating for the picker must not
        // materialise a "Main" thread (and a store on disk) for every agent
        // nobody has opened, nor raise the global thread-id floor to the
        // installation-wide maximum. An agent with no store simply has no
        // routable threads.
        let path = config.sessions_dir_for(&agent.id).join("threads.json");
        let Some(threads) = rustyclaw_core::threads::ThreadStore::peek(&path) else {
            continue;
        };
        for thread in threads {
            out.push(RoutableThreadDto {
                thread_id: thread.id,
                label: thread.label,
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
    vault_locked: bool,
) -> MessengerAccountDto {
    let spec = known_spec(messenger.messenger_type.as_str());

    // Non-secret fields only. A secret's value is in the vault (or, for a
    // not-yet-migrated account, in plaintext config) and either way it is not
    // this frame's business.
    let fields: BTreeMap<String, String> = spec
        .as_ref()
        .map(|s| {
            s.fields
                .iter()
                .filter(|f| !f.is_secret())
                .filter_map(|f| {
                    messenger
                        .field_value(&f.name)
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
    //
    // When the vault is locked there is nothing to check against, so the
    // references are taken at face value: "we cannot see it from here" is much
    // closer to the truth than "it is gone".
    let vaulted: BTreeMap<String, String> = messenger
        .secret_refs
        .iter()
        .filter(|(_, cred)| vault_locked || stored.contains(&format!("val:{cred}")))
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
        Some(s) if !kind_available(messenger.messenger_type.as_str()) => (
            false,
            s.feature.as_deref().map(|f| {
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
        messenger_type: messenger.messenger_type.to_string(),
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
    // A locked vault lists nothing, which is indistinguishable from an empty
    // one unless we ask. Reporting "no credentials" for a locked vault would
    // flag every working account as broken and push the user into retyping
    // tokens that are already stored — over the top of the good ones.
    let (vault_locked, stored) = {
        let mut mgr = vault.lock().await;
        match mgr.is_locked() {
            true => (true, Vec::new()),
            false => (false, mgr.list_secrets()),
        }
    };

    let mut accounts = Vec::with_capacity(config.messengers.len());
    for messenger in &config.messengers {
        accounts.push(account_dto(messenger, config, vault, &stored, vault_locked).await);
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
            vault_locked,
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

    let Some(spec) = known_spec(&messenger_type) else {
        return Err(vec![format!("Unknown messenger type: '{messenger_type}'")]);
    };
    // Only an *enabled* account needs a runnable backend. Both clients
    // implement the Disable toggle as a save with `enabled` flipped, so
    // refusing here made the one account the panel flags as unsupported —
    // the one failing at every startup — the one account that could not be
    // switched off without deleting it.
    if enabled && !kind_available(&messenger_type) {
        return Err(vec![format!(
            "This gateway was built without support for {}",
            spec.label
        )]);
    }

    // Renaming to a name already in use would leave two accounts fighting over
    // one set of vault keys, so it is refused rather than merged.
    //
    // Compared on the *slug*, not the display name: vault keys are derived by
    // lowercasing and folding punctuation, so "Telegram Main" and
    // "telegram-main" are different accounts sharing one set of credentials.
    // Saving the second would overwrite the first's token, and deleting either
    // would strand the other.
    let clash = config
        .messengers
        .iter()
        .filter(|m| Some(&m.name) != original_name.as_ref())
        .find(|m| setup::slugs_collide(&m.name, &name));
    if let Some(existing) = clash {
        return Err(vec![match existing.name == name {
            true => format!("An account named '{name}' already exists"),
            false => format!(
                "'{name}' would share stored credentials with the existing                  account '{}' — the two names differ only in case or                  punctuation. Pick a more distinct name.",
                existing.name
            ),
        }]);
    }

    // An edit must name an account that still exists. Falling back to
    // "create" here looked forgiving, but the case it actually served was a
    // stale panel: another connection deletes or renames the account, this
    // one submits its old form, and the fallback resurrected the deleted
    // account as new — with whatever half-stale settings the form held.
    if let Some(orig) = original_name.as_deref() {
        if !config.messengers.iter().any(|m| m.name == orig) {
            return Err(vec![format!(
                "'{orig}' no longer exists — it was renamed or deleted since this \
                 form was opened. Refresh the panel and start again."
            )]);
        }
    }

    // Start from the existing entry when editing so untouched fields survive.
    let mut entry = original_name
        .as_deref()
        .and_then(|orig| config.messengers.iter().find(|m| m.name == orig))
        .cloned()
        .unwrap_or_default();

    // A type change invalidates the old credentials: they belong to a backend
    // that is no longer in play. Drop the references — and remember the vault
    // entries they pointed at, because a reference-less credential is
    // invisible to `delete_account` and would otherwise live in the vault
    // forever. The entries themselves are deleted only after this save
    // persists, so a failure further down does not cost a working login.
    let mut obsolete: Vec<String> = Vec::new();
    if !entry.messenger_type.is_unset() && entry.messenger_type != messenger_type.as_str() {
        obsolete.extend(entry.secret_refs.values().cloned());
        entry.secret_refs.clear();
        // The plaintext twins go too. A never-migrated credential otherwise
        // survives the switch invisibly: the new type's schema no longer
        // lists it, so `plaintext_credentials` stops reporting it and the
        // "move to vault" affordance never sees it — while the value sits
        // re-serialised in config.toml and can even satisfy a same-named
        // required field on the new backend during validation.
        if let Some(old_spec) = known_spec(entry.messenger_type.as_str()) {
            for field in old_spec.fields.iter().filter(|f| f.is_secret()) {
                entry.set_field(&field.name, "").ignore();
            }
        }
    }

    entry.name = name.clone();
    entry.messenger_type = messenger_type.as_str().into();
    entry.enabled = enabled;

    let mut errors = Vec::new();
    for (field, value) in &fields {
        match known_field(&messenger_type, field) {
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
        let Some(f) = known_field(&messenger_type, &field) else {
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
    // in the vault from a previous save. A *disabled* account skips the
    // required-field check: it is not going to connect, and demanding a bot
    // token before an unsupported or half-configured account may be switched
    // off inverts the point of disabling it. Enabling validates in full.
    if enabled {
        let arriving: Vec<&str> = staged.iter().map(|(f, _, _)| f.as_str()).collect();
        validate_fields(&messenger_type, |field| {
            arriving.contains(&field)
                || entry.field_is_set(field)
                || fields
                    .iter()
                    .any(|(name, value)| name == field && !value.trim().is_empty())
        })?;
    }

    // Validate everything that still can be before the vault is touched. The
    // avatar check in particular used to run after the writes, and any
    // rejection there stranded a freshly stored credential under an account
    // that never came into being — unreferenced by `secret_refs`, invisible
    // to `delete_account`, cleaned up by nothing.
    let validated_avatar = match avatar_path {
        None => None,
        Some(path) if path.as_os_str().is_empty() => Some(None),
        Some(path) => {
            let canonical = validate_avatar(&path, config)?;
            // Fingerprinted now, while the user is looking at the file they
            // picked. The upload happens at connect time, and the hash is
            // what stops a file swapped in between from being published.
            let fingerprint = setup::file_fingerprint(&canonical).map_err(|e| {
                vec![format!(
                    "Could not read avatar '{}': {e}",
                    canonical.display()
                )]
            })?;
            Some(Some((canonical, fingerprint)))
        }
    };

    // Steps after the writes can still fail (a vault rename, persisting the
    // config), so remember which credentials this call brought into being.
    // Only those are safe to delete on the way out — rolling back an
    // *overwrite* by deletion would destroy the previous value too.
    let mut created: Vec<String> = Vec::new();
    {
        let mut mgr = vault.lock().await;
        let existing = mgr.list_secrets();
        for (field, label, value) in &staged {
            let cred = secret_name(&name, field);
            if !existing.contains(&format!("val:{cred}")) {
                created.push(cred.clone());
            }
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
            if let Err(e) = mgr.store_service_credential(&cred, &secret_entry, value) {
                for cred in &created {
                    mgr.delete_credential(cred).ignore();
                }
                return Err(vec![format!("Could not store {label} in the vault: {e}")]);
            }
            // A rename arriving together with a fresh credential overwrites
            // the field's reference before `rename_credentials` ever sees it,
            // so the key being displaced must be retired here — it is the
            // only record of the old entry, and without it the previous
            // credential lives in the vault unreferenced forever.
            if let Some(displaced) = entry.secret_refs.insert(field.clone(), cred.clone()) {
                if displaced != cred {
                    obsolete.push(displaced);
                }
            }
            // The vault now owns it; leaving the plaintext twin behind would
            // defeat the entire point of moving it.
            entry.set_field(field, "").ignore();
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
    if let Some(avatar) = validated_avatar {
        match avatar {
            Some((path, fingerprint)) => {
                profile.avatar_path = Some(path);
                profile.avatar_sha256 = Some(fingerprint);
            }
            None => {
                profile.avatar_path = None;
                profile.avatar_sha256 = None;
            }
        }
    }
    if let Some(id) = agent_id {
        profile.agent_id = override_of(Some(id));
    }
    entry.profile = (!profile.is_empty()).then_some(profile);

    // Renaming copies the vault entries too, so credentials follow the
    // account rather than being orphaned under the old name. The old keys go
    // on the deferred-delete list: they must outlive `persist`, which is the
    // moment the config stops referring to them.
    if let Some(orig) = original_name.as_deref().filter(|o| *o != name) {
        match rename_credentials(&mut entry, vault, orig, &name).await {
            Ok((old_keys, rename_created)) => {
                obsolete.extend(old_keys);
                created.extend(rename_created);
            }
            Err(e) => {
                discard_created_credentials(vault, &created).await;
                return Err(e);
            }
        }
    }

    // Everything from here to `persist` mutates the connection's live config,
    // and `build_view` renders that config to every client right after this
    // function returns. A failed persist must therefore put it back exactly
    // as it was — otherwise the user is told the save failed while the panel
    // shows it applied, and the phantom state gets written out by whichever
    // unrelated save next succeeds.
    let prior_messengers = config.messengers.clone();
    let prior_routes = config.messenger_routes.clone();

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

    if let Err(e) = persist(config) {
        config.messengers = prior_messengers;
        config.messenger_routes = prior_routes;
        discard_created_credentials(vault, &created).await;
        return Err(e);
    }

    // The save is durable; now the keys nothing references any more can go.
    // Best-effort, and never anything still referenced by any account — a
    // stuck entry is untidy, a deleted live credential is a broken login.
    if !obsolete.is_empty() {
        let referenced: std::collections::BTreeSet<&String> = config
            .messengers
            .iter()
            .flat_map(|m| m.secret_refs.values())
            .collect();
        let mut mgr = vault.lock().await;
        for cred in obsolete.iter().filter(|c| !referenced.contains(c)) {
            if let Err(e) = mgr.delete_credential(cred) {
                warn!(credential = %cred, error = %e, "Could not remove a superseded messenger credential");
            }
        }
    }
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
/// Best-effort removal of credentials a failed save brought into being.
///
/// Callers pass only names that did not exist before the save — deleting an
/// overwritten credential would destroy the previous value along with the new
/// one. Failures here are swallowed: the save is already being rejected for
/// the real reason, and that is the error the user needs to see.
async fn discard_created_credentials(vault: &SharedVault, created: &[String]) {
    if created.is_empty() {
        return;
    }
    let mut mgr = vault.lock().await;
    for cred in created {
        mgr.delete_credential(cred).ignore();
    }
}

/// Copy an account's credentials to the names a rename gives them.
///
/// The old keys are deliberately *not* deleted here. Until the config that
/// references the new names has been persisted, the old key is the only copy
/// the on-disk config can find — deleting it first meant a failed settings
/// write silently destroyed the credential. The caller deletes the returned
/// old keys only after `persist` has succeeded.
///
/// Returns `(old_keys, created_keys)`: the keys to retire once the rename is
/// durable, and the copies this call brought into being (for rollback). On
/// error the copies already made are removed again before returning.
async fn rename_credentials(
    entry: &mut MessengerConfig,
    vault: &SharedVault,
    from: &str,
    to: &str,
) -> Result<(Vec<String>, Vec<String>), Vec<String>> {
    debug!(%from, %to, "Copying messenger credentials to follow a rename");
    let mut mgr = vault.lock().await;

    let moves: Vec<(String, String, String)> = entry
        .secret_refs
        .iter()
        .map(|(field, current)| (field.clone(), current.clone(), secret_name(to, field)))
        .filter(|(_, current, new)| current != new)
        .collect();

    let existing = mgr.list_secrets();
    let mut old_keys: Vec<String> = Vec::new();
    let mut created: Vec<String> = Vec::new();
    for (field, old, new) in moves {
        let value = match mgr.read_service_credential(&old) {
            Ok(value) => value,
            Err(e) => {
                for cred in &created {
                    mgr.delete_credential(cred).ignore();
                }
                return Err(vec![format!("Could not read '{old}' while renaming: {e}")]);
            }
        };
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
        if !existing.contains(&format!("val:{new}")) {
            created.push(new.clone());
        }
        if let Err(e) = mgr.store_service_credential(&new, &secret_entry, value.as_str()) {
            for cred in &created {
                mgr.delete_credential(cred).ignore();
            }
            return Err(vec![format!("Could not write '{new}' while renaming: {e}")]);
        }
        old_keys.push(old);
        entry.secret_refs.insert(field, new);
    }
    Ok((old_keys, created))
}

async fn delete_account(config: &mut Config, vault: &SharedVault, name: &str) -> Outcome {
    let Some(index) = config.messengers.iter().position(|m| m.name == name) else {
        return Err(vec![format!("No account named '{name}'")]);
    };

    // Config first, vault second. Destroying the vault entries before the
    // deletion was durable meant a failed settings write left the on-disk
    // config naming credentials that no longer existed — an account that
    // could never log in again. The reverse failure order merely leaves an
    // untidy vault entry, which is why the vault cleanup below is
    // best-effort.
    let prior_messengers = config.messengers.clone();
    let prior_routes = config.messenger_routes.clone();

    let removed = config.messengers.remove(index);

    // A route pointing at a deleted account matches nothing and would sit in
    // the table as a permanent puzzle.
    let before = config.messenger_routes.len();
    config.messenger_routes.retain(|r| r.messenger != name);
    let dropped = before - config.messenger_routes.len();

    if let Err(e) = persist(config) {
        config.messengers = prior_messengers;
        config.messenger_routes = prior_routes;
        return Err(e);
    }

    {
        let mut mgr = vault.lock().await;
        // Never delete a key some other account still references (possible in
        // a legacy config whose names collide once slugged).
        let referenced: std::collections::BTreeSet<&String> = config
            .messengers
            .iter()
            .flat_map(|m| m.secret_refs.values())
            .collect();
        for cred in removed
            .secret_refs
            .values()
            .filter(|c| !referenced.contains(c))
        {
            if let Err(e) = mgr.delete_credential(cred) {
                // The account is already gone from config; a stuck vault entry
                // is untidy, not dangerous, and worth a log rather than a
                // failed delete the user cannot retry.
                warn!(credential = %cred, error = %e, "Could not remove messenger credential");
            }
        }
    }

    info!(account = %name, routes_removed = dropped, "Messenger account deleted");
    Ok(Some(match dropped {
        0 => format!("Deleted '{name}'"),
        n => format!("Deleted '{name}' and {n} route(s)"),
    }))
}

/// Move an account's plaintext credentials into the vault.
async fn migrate_secrets(config: &mut Config, vault: &SharedVault, name: &str) -> Outcome {
    // For restoring the live config if anything below fails: the client is
    // told the migration failed, so the config it is then shown must still
    // carry the plaintext credentials, and the on-disk copy must match.
    let prior_messengers = config.messengers.clone();

    let Some(entry) = config.messengers.iter_mut().find(|m| m.name == name) else {
        return Err(vec![format!("No account named '{name}'")]);
    };
    let pending = entry.plaintext_credentials();
    if pending.is_empty() {
        return Ok(Some(format!("'{name}' has no plaintext credentials")));
    }

    let label = known_spec(entry.messenger_type.as_str())
        .map_or_else(|| "Messenger".to_string(), |s| s.label.to_string());
    let mut moved = Vec::new();
    let mut created: Vec<String> = Vec::new();
    let mut failure: Option<Vec<String>> = None;
    {
        let mut mgr = vault.lock().await;
        let existing = mgr.list_secrets();
        for field in &pending {
            let Some(value) = entry.field_value(&field.field) else {
                continue;
            };
            let cred = secret_name(name, &field.field);
            let secret_entry = SecretEntry {
                label: format!("{name} — {}", field.label),
                kind: SecretKind::Token,
                policy: AccessPolicy::WithAuth,
                description: Some(format!("{label} credential for messenger '{name}'")),
                disabled: false,
            };
            if !existing.contains(&format!("val:{cred}")) {
                created.push(cred.clone());
            }
            if let Err(e) = mgr.store_service_credential(&cred, &secret_entry, &value) {
                for cred in &created {
                    mgr.delete_credential(cred).ignore();
                }
                failure = Some(vec![format!("Could not store {}: {e}", field.label)]);
                break;
            }
            entry.secret_refs.insert(field.field.to_string(), cred);
            // Clear only after the vault write succeeded — the ordering is the
            // difference between migrating a credential and losing one.
            entry.set_field(&field.field, "").ignore();
            moved.push(field.label.clone());
        }
    }
    if let Some(errors) = failure {
        config.messengers = prior_messengers;
        return Err(errors);
    }

    if let Err(e) = persist(config) {
        config.messengers = prior_messengers;
        discard_created_credentials(vault, &created).await;
        return Err(e);
    }
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
    // the user would have no way to tell that from a working one — so an
    // *enabled* save requires the thread. Disabling skips the check, exactly
    // as `save_account` does for unrunnable backends: both clients toggle by
    // re-sending the save with `enabled` flipped, and the one route the
    // panel flags as dangling must not be the one that can't be turned off.
    let threads = routable_threads(config);
    let target = threads
        .iter()
        .find(|t| t.thread_id == thread_id && t.agent_id == agent);
    let label = match (target, enabled) {
        (Some(target), _) => target.label.clone(),
        (None, true) => return Err(format!("Agent '{agent}' has no thread #{thread_id}")),
        (None, false) => format!("#{thread_id}"),
    };

    let channel = channel.filter(|c| !c.trim().is_empty());
    let route = ThreadRoute {
        messenger: messenger.clone(),
        channel: channel.clone(),
        thread_id,
        agent_id: Some(agent),
        enabled,
    };

    let prior_routes = config.messenger_routes.clone();
    // Identity via `same_key`, which folds case exactly like route matching
    // does — exact comparison here let `#Rust` and `#rust` coexist as two
    // routes for one channel, the second permanently shadowed.
    match config
        .messenger_routes
        .iter_mut()
        .find(|r| r.same_key(&messenger, channel.as_deref()))
    {
        Some(existing) => *existing = route,
        None => config.messenger_routes.push(route),
    }

    if let Err(e) = persist(config) {
        // The client is told the save failed; the live config (and the view
        // rebuilt from it) must not show the route as applied.
        config.messenger_routes = prior_routes;
        return Err(e.join("; "));
    }
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
    let prior_routes = config.messenger_routes.clone();
    // Same identity rule as `save_route`: fold case, as matching does.
    config
        .messenger_routes
        .retain(|r| !r.same_key(messenger, channel));
    if config.messenger_routes.len() == prior_routes.len() {
        return Err("No such route".to_string());
    }
    if let Err(e) = persist(config) {
        config.messenger_routes = prior_routes;
        return Err(e.join("; "));
    }
    Ok(Some("Route removed".to_string()))
}

/// Image extensions a profile picture may have.
const AVATAR_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Check an avatar path before it can be handed to a chat platform.
///
/// `announce_profile` uploads this file's *contents* to a third-party service,
/// so an unchecked path is an exfiltration primitive: a connected client could
/// name the credentials vault and have the gateway publish it. The workspace
/// file frames already refuse anything outside their root; the same
/// confinement applies here — an avatar must live inside an agent workspace,
/// not merely outside the credentials store, or any readable image anywhere
/// on the gateway host could be published to a platform the requester picks.
pub(crate) fn validate_avatar(
    path: &std::path::Path,
    config: &Config,
) -> Result<std::path::PathBuf, Vec<String>> {
    let canonical = path
        .canonicalize()
        .map_err(|e| vec![format!("Avatar '{}' cannot be read: {e}", path.display())])?;

    // Two checks, deliberately. `is_protected_path` consults a process-global
    // credentials directory that is only set once the gateway has initialised
    // its sandbox, so on its own it silently permits everything before that
    // point. The explicit comparison does not depend on global state. Both
    // are kept even though the workspace confinement below would also refuse
    // these paths: naming the vault deserves its own message and its own log
    // line, not a generic "outside the workspace".
    let protected = rustyclaw_core::tools::is_protected_path(&canonical)
        || config
            .credentials_dir()
            .canonicalize()
            .is_ok_and(|dir| canonical.starts_with(dir));
    if protected {
        warn!(path = %canonical.display(), "Refused an avatar inside a protected path");
        return Err(vec![
            "That path is inside the credentials store and cannot be published".to_string(),
        ]);
    }

    if !canonical.is_file() {
        return Err(vec![format!("'{}' is not a file", canonical.display())]);
    }

    // An extension check is not proof of file type, but it stops the whole
    // class of "point it at a config file and see what happens" by accident,
    // and a mislabelled image simply fails on the platform's side.
    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !AVATAR_EXTENSIONS.contains(&ext.as_str()) {
        return Err(vec![format!(
            "An avatar must be one of: {}",
            AVATAR_EXTENSIONS.join(", ")
        )]);
    }

    // Confinement proper: the file must be inside an agent workspace. That is
    // where the user's own files live, and it is the boundary every other
    // client-named path in the protocol already honours.
    let mut roots = vec![config.workspace_dir()];
    for agent in config.agent_registry().list() {
        roots.push(config.workspace_dir_for(&agent.id));
    }
    let allowed = roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|root| canonical.starts_with(root))
    });
    if !allowed {
        warn!(path = %canonical.display(), "Refused an avatar outside the agent workspaces");
        return Err(vec![format!(
            "An avatar must live inside an agent workspace (for instance {}); \
             the gateway does not publish files from elsewhere on its host",
            config.workspace_dir().display()
        )]);
    }

    Ok(canonical)
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
    use rustyclaw_core::threads::ThreadManager;
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
    async fn a_plugin_kind_account_saves_like_a_builtin() {
        use rustyclaw_core::messengers::setup::{FieldKind, FieldSpec, KindSpec, Requirement};
        use std::borrow::Cow;

        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        // A kind this gateway was never compiled with: schema from the
        // registry, plain field with no MessengerConfig slot, one secret.
        let spec = KindSpec {
            id: Cow::Borrowed("cfg_plugin_chat"),
            label: Cow::Borrowed("Acme Chat"),
            icon: Cow::Borrowed("🔌"),
            summary: Cow::Borrowed("Plugin-registered test kind"),
            feature: None,
            fields: Cow::Owned(vec![
                FieldSpec {
                    name: Cow::Borrowed("workspace"),
                    label: Cow::Borrowed("Workspace"),
                    kind: FieldKind::Text,
                    requirement: Requirement::Required,
                    help: Cow::Borrowed("Which workspace to join"),
                },
                FieldSpec {
                    name: Cow::Borrowed("api_key"),
                    label: Cow::Borrowed("API key"),
                    kind: FieldKind::Secret,
                    requirement: Requirement::Required,
                    help: Cow::Borrowed("Acme API key"),
                },
            ]),
        };
        rustyclaw_core::messengers::messenger_registry()
            .register_plugin_kind(
                "cfg-test-plugin",
                spec,
                Arc::new(|config: &MessengerConfig| {
                    Ok(Box::new(rustyclaw_core::messengers::ConsoleMessenger::new(
                        config.name.clone(),
                    ))
                        as Box<dyn rustyclaw_core::messengers::Messenger>)
                }),
            )
            .expect("plugin kind registers");

        save_account(
            &mut config,
            &vault,
            None,
            "acme-1".to_string(),
            "cfg_plugin_chat".to_string(),
            true,
            vec![("workspace".to_string(), "myteam".to_string())],
            vec![("api_key".to_string(), "sekrit".to_string())],
            None,
            None,
            None,
            None,
        )
        .await
        .expect("plugin-kind account saves");

        let account = &config.messengers[0];
        assert_eq!(account.messenger_type.as_str(), "cfg_plugin_chat");
        assert_eq!(
            account.extra.get("workspace").map(String::as_str),
            Some("myteam"),
            "plain plugin field persists in `extra`"
        );
        assert_eq!(
            account.secret_ref("api_key"),
            Some("messenger/acme-1/api_key"),
            "plugin secret is vault-referenced, not in config"
        );
        assert!(!account.extra.contains_key("api_key"));
        assert_eq!(
            vault
                .lock()
                .await
                .read_service_credential("messenger/acme-1/api_key")
                .unwrap()
                .map(String::from)
                .as_deref(),
            Some("sekrit")
        );
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
            .unwrap()
            .map(String::from);
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
                .map(String::from)
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
                .map(String::from)
                .as_deref(),
            Some("123:secret"),
            "the credential must follow the account"
        );
        assert_eq!(
            mgr.read_service_credential("messenger/tg/token")
                .unwrap()
                .map(String::from),
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
                .map(String::from)
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
                .unwrap()
                .map(String::from),
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
        // The vault entry goes too. With the reference cleared nothing can
        // reach it again — not even delete_account — so leaving it stored
        // means a live token nothing will ever clean up.
        assert_eq!(
            vault
                .lock()
                .await
                .read_service_credential("messenger/acct/token")
                .unwrap()
                .map(String::from),
            None,
            "a type change must not orphan the old credential in the vault"
        );
    }

    #[tokio::test]
    async fn an_edit_naming_a_deleted_account_is_refused_not_resurrected() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();
        delete_account(&mut config, &vault, "tg").await.unwrap();

        // A second panel that never saw the delete submits its stale form.
        // The old fallback pushed this as a brand-new account.
        let errors = save_account(
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
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(errors[0].contains("no longer exists"), "{errors:?}");
        assert!(
            config.messengers.is_empty(),
            "a stale edit must not resurrect a deleted account"
        );
    }

    #[tokio::test]
    async fn an_unsupported_account_can_still_be_disabled() {
        if kind_available("matrix") {
            return; // This build can run it, so the scenario doesn't exist.
        }
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        // An account this build can't run — from a config written by a build
        // that could. The panel flags it unsupported and offers Disable.
        config.messengers.push(MessengerConfig {
            name: "mx".to_string(),
            messenger_type: "matrix".into(),
            enabled: true,
            ..Default::default()
        });

        // Disabling must work: it is the one action that stops the account
        // failing at every startup, and it cannot make anything worse.
        save_account(
            &mut config,
            &vault,
            Some("mx".to_string()),
            "mx".to_string(),
            "matrix".to_string(),
            false,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("disabling an unsupported account must be allowed");
        assert!(!config.messengers[0].enabled);

        // Re-enabling still validates: the backend genuinely cannot run.
        let errors = save_account(
            &mut config,
            &vault,
            Some("mx".to_string()),
            "mx".to_string(),
            "matrix".to_string(),
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
        assert!(errors[0].contains("without support"), "{errors:?}");
    }

    #[tokio::test]
    async fn an_avatar_is_fingerprinted_at_save_and_cleared_with_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        std::fs::create_dir_all(config.workspace_dir()).unwrap();
        let face = config.workspace_dir().join("face.png");
        std::fs::write(&face, b"\x89PNG-bytes").unwrap();
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
            Some(face.clone()),
            None,
        )
        .await
        .unwrap();

        let profile = config.messengers[0].profile.clone().unwrap();
        assert_eq!(
            profile.avatar_sha256.as_deref(),
            Some(setup::file_fingerprint(&face).unwrap().as_str()),
            "the saved bytes are what connect-time uploads are pinned to"
        );

        // Clearing the avatar clears the pin with it.
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
            Some(std::path::PathBuf::new()),
            None,
        )
        .await
        .unwrap();
        let profile = config.messengers[0].profile.clone();
        assert!(profile.is_none_or(|p| p.avatar_sha256.is_none()));
    }

    #[tokio::test]
    async fn a_rename_with_a_fresh_credential_retires_the_old_vault_entry() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("old-token"))
            .await
            .unwrap();

        // Rename and retype in one save: the staging loop overwrites the
        // field's reference before rename_credentials can see the old key,
        // so the displaced key has to be retired by the staging loop itself.
        save_account(
            &mut config,
            &vault,
            Some("tg".to_string()),
            "tg2".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            vec![("token".to_string(), "new-token".to_string())],
            None,
            None,
            None,
            None,
        )
        .await
        .expect("rename with a new credential should succeed");

        let mut mgr = vault.lock().await;
        assert_eq!(
            mgr.read_service_credential("messenger/tg2/token")
                .unwrap()
                .map(String::from)
                .as_deref(),
            Some("new-token")
        );
        assert_eq!(
            mgr.read_service_credential("messenger/tg/token")
                .unwrap()
                .map(String::from),
            None,
            "the displaced credential must not live on unreferenced"
        );
    }

    #[tokio::test]
    async fn a_type_change_clears_the_old_backends_plaintext_credential() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        // A pre-vault account: the token was hand-written into config.toml
        // and never migrated.
        let mut legacy = MessengerConfig {
            name: "acct".to_string(),
            messenger_type: "telegram".into(),
            enabled: true,
            ..Default::default()
        };
        legacy.set_field("token", "123:plaintext").unwrap();
        config.messengers.push(legacy);

        // Telegram → IRC: the new schema has no `token`, so nothing would
        // ever report or migrate the leftover — it would just sit in
        // config.toml under the new type.
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

        assert!(
            config.messengers[0]
                .field_value("token")
                .unwrap_or_default()
                .is_empty(),
            "the old backend's plaintext credential must not survive the switch"
        );
    }

    #[tokio::test]
    async fn a_leftover_plaintext_credential_does_not_satisfy_the_new_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        let mut legacy = MessengerConfig {
            name: "acct".to_string(),
            messenger_type: "telegram".into(),
            enabled: true,
            ..Default::default()
        };
        legacy.set_field("token", "123:plaintext").unwrap();
        config.messengers.push(legacy);

        // Discord also requires a secret named `token`. The leftover Telegram
        // value used to satisfy that requirement, letting the save succeed
        // with the previous backend's credential.
        let errors = save_account(
            &mut config,
            &vault,
            Some("acct".to_string()),
            "acct".to_string(),
            "discord".to_string(),
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
        assert!(
            errors
                .iter()
                .any(|e| e.contains("token") || e.contains("Bot")),
            "a new backend must demand its own credential: {errors:?}"
        );
    }

    #[tokio::test]
    async fn a_rename_that_cannot_persist_keeps_the_credential_reachable() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap();

        // Make the *next* persist fail: the rename's vault copy has already
        // happened by then, which is exactly the window that used to lose
        // the credential (old key deleted, new key rolled back, on-disk
        // config still pointing at the old key).
        let config_path = dir.path().join("config.toml");
        std::fs::remove_file(&config_path).unwrap();
        std::fs::create_dir_all(&config_path).unwrap();

        let errors = save_account(
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
        .unwrap_err();
        assert!(!errors.is_empty());

        // The in-memory config must still describe the un-renamed account —
        // the client was told the save failed, and the view built right after
        // this reply must not show it applied.
        assert_eq!(config.messengers[0].name, "tg");
        assert_eq!(
            config.messengers[0].secret_ref("token"),
            Some("messenger/tg/token")
        );
        // And the key that config points at must still hold the value.
        assert_eq!(
            vault
                .lock()
                .await
                .read_service_credential("messenger/tg/token")
                .unwrap()
                .map(String::from)
                .as_deref(),
            Some("123:secret"),
            "a failed rename must never cost the stored credential"
        );
    }

    #[tokio::test]
    async fn a_route_whose_thread_was_deleted_can_still_be_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        let sessions = config.sessions_dir_for(MAIN_AGENT_ID);
        std::fs::create_dir_all(&sessions).unwrap();
        let mut mgr = ThreadManager::new();
        let thread = mgr.create_chat("Inbox");
        rustyclaw_core::threads::ThreadStore::at_legacy_path(&sessions.join("threads.json"))
            .persist(&mut mgr)
            .unwrap();
        save_route(&mut config, "tg".into(), None, thread.0, None, true).unwrap();

        // The thread goes away; the panel now flags the route as dangling.
        let mut empty = ThreadManager::new();
        rustyclaw_core::threads::ThreadStore::at_legacy_path(&sessions.join("threads.json"))
            .persist(&mut empty)
            .unwrap();

        // Disabling the broken route must work — it is the one action that
        // stops it, and deleting it outright was the only alternative.
        save_route(&mut config, "tg".into(), None, thread.0, None, false)
            .expect("disabling a dangling route must be allowed");
        assert!(!config.messenger_routes[0].enabled);

        // Re-enabling still validates: the thread genuinely is not there.
        let error = save_route(&mut config, "tg".into(), None, thread.0, None, true).unwrap_err();
        assert!(error.contains("no thread"), "{error}");
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
        // Through the store, as the gateway persists: seeding the legacy
        // `threads.json` masked the production loader reading a file that
        // no longer exists.
        rustyclaw_core::threads::ThreadStore::at_legacy_path(&sessions.join("threads.json"))
            .persist(&mut mgr)
            .unwrap();

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

    /// A writer that keeps every frame, for handler-level tests.
    struct SinkWriter(Vec<ServerFrame>);

    #[async_trait::async_trait]
    impl rustyclaw_core::gateway::TransportWriter for SinkWriter {
        async fn send_on_stream(
            &mut self,
            _stream_id: u64,
            frame: &ServerFrame,
        ) -> anyhow::Result<()> {
            self.0.push(frame.clone());
            Ok(())
        }

        async fn close(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_mutation_lands_in_the_shared_config_not_just_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let (config, vault) = fixture(dir.path());
        let shared: crate::SharedConfig = Arc::new(tokio::sync::RwLock::new(config.clone()));
        let mut conn_config = config;
        let mut writer = SinkWriter(Vec::new());

        handle_messenger_config(
            &mut writer,
            ClientPayload::MessengerAccountSave {
                original_name: None,
                name: "tg".to_string(),
                messenger_type: "telegram".into(),
                enabled: true,
                fields: Vec::new(),
                secrets: vec![("token".to_string(), "123:secret".to_string().into())],
                display_name: None,
                bio: None,
                avatar_path: None,
                agent_id: None,
            },
            &mut conn_config,
            &shared,
            &vault,
        )
        .await
        .unwrap();

        // The shared copy is what every other settings handler persists from.
        // An account living only in the connection snapshot survives exactly
        // until the next agent rename or model switch rewrites config.toml
        // without it.
        assert_eq!(shared.read().await.messengers.len(), 1);
        assert_eq!(
            conn_config.messengers.len(),
            1,
            "the connection snapshot must follow the shared copy"
        );

        // The unrelated-settings-change scenario end to end: persisting the
        // shared copy must keep the account on disk.
        shared.read().await.save(None).unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(
            on_disk.contains("[[messengers]]"),
            "a shared-config persist must not erase messenger accounts"
        );
    }

    #[tokio::test]
    async fn a_delete_that_cannot_persist_keeps_the_credential_and_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap();

        // Make persist fail; the vault deletion must not have happened yet,
        // or the on-disk config keeps naming a credential that is gone and
        // the account can never log in again.
        let config_path = dir.path().join("config.toml");
        std::fs::remove_file(&config_path).unwrap();
        std::fs::create_dir_all(&config_path).unwrap();

        let errors = delete_account(&mut config, &vault, "tg").await.unwrap_err();
        assert!(!errors.is_empty());

        assert_eq!(
            config.messengers.len(),
            1,
            "a failed delete must leave the live config untouched"
        );
        assert_eq!(
            vault
                .lock()
                .await
                .read_service_credential("messenger/tg/token")
                .unwrap()
                .map(String::from)
                .as_deref(),
            Some("123:secret"),
            "a failed delete must not have destroyed the credential"
        );
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
        std::fs::create_dir_all(config.workspace_dir()).unwrap();
        let avatar = config.workspace_dir().join("face.png");
        std::fs::write(&avatar, b"png").unwrap();
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
            Some(avatar.canonicalize().unwrap())
        );
    }

    #[tokio::test]
    async fn an_account_whose_slug_collides_with_another_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "telegram-main", Some("first"))
            .await
            .unwrap();

        // Different display name, same vault keys: accepting this would
        // overwrite the first account's token.
        let errors = save_telegram(&mut config, &vault, "Telegram Main", Some("second"))
            .await
            .unwrap_err();
        assert!(errors[0].contains("share stored credentials"), "{errors:?}");
        assert_eq!(config.messengers.len(), 1);
        assert_eq!(
            vault
                .lock()
                .await
                .read_service_credential("messenger/telegram-main/token")
                .unwrap()
                .map(String::from)
                .as_deref(),
            Some("first"),
            "the existing credential must survive the refused save"
        );
    }

    #[tokio::test]
    async fn renaming_an_account_to_its_own_slug_variant_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "telegram-main", Some("t"))
            .await
            .unwrap();

        // Colliding with *itself* is not a collision.
        save_account(
            &mut config,
            &vault,
            Some("telegram-main".to_string()),
            "Telegram Main".to_string(),
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
        .expect("an account may be recased");
        assert_eq!(config.messengers[0].name, "Telegram Main");
    }

    #[tokio::test]
    async fn an_avatar_inside_the_credentials_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        // The gateway uploads an avatar's *contents* to a chat platform, so
        // naming the vault would publish it.
        let secret = config.credentials_dir().join("secrets.json");
        std::fs::write(&secret, "{}").unwrap();
        let errors = save_account(
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
            Some(secret),
            None,
        )
        .await
        .unwrap_err();
        assert!(errors[0].contains("credentials store"), "{errors:?}");
    }

    #[tokio::test]
    async fn an_avatar_that_is_not_an_image_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, "private").unwrap();
        let errors = save_account(
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
            Some(notes),
            None,
        )
        .await
        .unwrap_err();
        assert!(errors[0].contains("must be one of"), "{errors:?}");
    }

    #[tokio::test]
    async fn an_ordinary_image_is_accepted_as_an_avatar() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        std::fs::create_dir_all(config.workspace_dir()).unwrap();
        let face = config.workspace_dir().join("face.png");
        std::fs::write(&face, b"\x89PNG").unwrap();
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
            Some(face.clone()),
            None,
        )
        .await
        .expect("a png in the workspace is fine");
        assert_eq!(
            config.messengers[0]
                .profile
                .as_ref()
                .and_then(|p| p.avatar_path.clone()),
            Some(face.canonicalize().unwrap())
        );
    }

    #[tokio::test]
    async fn an_image_outside_every_workspace_is_refused_as_an_avatar() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("t"))
            .await
            .unwrap();

        // A perfectly ordinary image — but not in any agent workspace, so
        // publishing it would let a client exfiltrate arbitrary readable
        // images from the gateway host.
        let face = dir.path().join("face.png");
        std::fs::write(&face, b"\x89PNG").unwrap();
        let errors = save_account(
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
            Some(face),
            None,
        )
        .await
        .unwrap_err();
        assert!(errors[0].contains("workspace"), "{errors:?}");
    }

    #[tokio::test]
    async fn a_save_rejected_after_storing_a_credential_removes_it_again() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        // A bad avatar path fails the save *after* credentials used to be
        // written, which stranded the token in the vault under an account
        // that never came into being.
        let errors = save_account(
            &mut config,
            &vault,
            None,
            "tg".to_string(),
            "telegram".to_string(),
            true,
            Vec::new(),
            vec![("token".to_string(), "123:secret".to_string())],
            None,
            None,
            Some(dir.path().join("nonexistent.png")),
            None,
        )
        .await
        .unwrap_err();
        assert!(!errors.is_empty());
        assert!(config.messengers.is_empty(), "the account must not exist");

        let stored = vault.lock().await.list_secrets();
        assert!(
            stored.iter().all(|k| !k.contains("messenger/tg/")),
            "a rejected save must not leave its credential behind: {stored:?}"
        );
    }

    #[tokio::test]
    async fn a_save_that_cannot_persist_removes_the_credential_it_created() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());

        // Occupying config.toml's path with a directory makes `persist` fail
        // — the one rejection that can only happen after the vault write, so
        // it exercises the rollback rather than the reordered validation.
        std::fs::create_dir_all(dir.path().join("config.toml")).unwrap();

        let errors = save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap_err();
        assert!(errors[0].contains("save"), "{errors:?}");

        let stored = vault.lock().await.list_secrets();
        assert!(
            stored.iter().all(|k| !k.contains("messenger/tg/")),
            "an unpersisted account must not keep a vault credential: {stored:?}"
        );
        assert!(
            config.messengers.is_empty(),
            "an unpersisted account must not appear in the live config either"
        );
    }

    #[tokio::test]
    async fn a_locked_vault_does_not_report_every_account_as_uncredentialed() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, vault) = fixture(dir.path());
        save_telegram(&mut config, &vault, "tg", Some("123:secret"))
            .await
            .unwrap();

        // A password-protected vault the daemon has not been given a password
        // for lists nothing — which must not be read as "the credential is
        // gone", or the user gets told to retype a token that is already there.
        // A password-protected vault is one with a secrets file and no key
        // file beside it — that is what `is_locked` looks for.
        std::fs::remove_file(config.credentials_dir().join("secrets.key")).ignore();
        let locked = Arc::new(Mutex::new(rustyclaw_core::secrets::SecretsManager::locked(
            config.credentials_dir(),
        )));
        assert!(locked.lock().await.is_locked(), "fixture should be locked");
        let frame = build_view(&config, &locked).await;
        match frame.payload {
            ServerPayload::MessengerConfigResult {
                accounts,
                vault_locked,
                ..
            } => {
                assert!(vault_locked, "the view must say the vault is locked");
                assert_eq!(
                    accounts[0].vaulted.get("token").map(String::as_str),
                    Some("messenger/tg/token"),
                    "config's reference is the best available answer here"
                );
            }
            other => panic!("expected a config result, got {other:?}"),
        }
    }

    #[test]
    fn availability_is_registry_membership() {
        // Ungated builtins always register; a kind nobody registered is
        // unavailable no matter what the static table thinks of it.
        assert!(kind_available("console"));
        assert!(!kind_available("time-travel"));
    }
}

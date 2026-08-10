//! Messenger integration for the gateway.
//!
//! This module provides the messenger polling loop that receives messages
//! from configured messengers (Telegram, Discord, Matrix, etc.) and routes
//! them through the model for processing with full tool loop support.

use anyhow::Result;
use rustyclaw_core::config::Config;
use rustyclaw_core::ignore::Ignore;
use rustyclaw_core::messengers::{Message, Messenger, MessengerManager, SendOptions};
use rustyclaw_core::tools;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use crate::providers;
use crate::secrets_handler;
use crate::skills_handler;
use crate::{SharedSkillManager, SharedVault};
use rustyclaw_core::gateway::{
    ChatMessage, MediaRef, ModelContext, ProviderRequest, ToolCallResult,
};

mod builders;
mod media;
mod prompt;
mod relevance;

use builders::create_messenger;
use media::process_attachments;
use prompt::build_messenger_system_prompt;
use relevance::{SentMessageTracker, channel_key, is_message_relevant, mention_tokens};

/// Shared messenger manager for the gateway.
pub type SharedMessengerManager = Arc<Mutex<MessengerManager>>;

/// Conversation history storage per chat.
/// Key: "messenger_type:chat_id" or "messenger_type:sender_id"
type ConversationStore = Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>;

/// Maximum messages to keep in conversation history per chat.
const MAX_HISTORY_MESSAGES: usize = 50;

/// Maximum tool loop rounds.
const MAX_TOOL_ROUNDS: usize = 25;

/// Fill an account's credential fields from the vault.
///
/// Config carries a *reference* — `secret_refs.token = "messenger/x/token"` —
/// and the value lives encrypted. This produces the config the builders
/// actually want: the same entry with those fields populated.
///
/// A reference that resolves to nothing is left as it is rather than blanked,
/// so the builder's own "Telegram requires 'token'" error is what the user
/// sees. Losing the credential and the explanation at once would be worse.
async fn resolve_credentials(
    messenger_config: &rustyclaw_core::config::MessengerConfig,
    vault: &SharedVault,
) -> rustyclaw_core::config::MessengerConfig {
    if messenger_config.secret_refs.is_empty() {
        return messenger_config.clone();
    }

    let mut resolved = messenger_config.clone();
    let mut mgr = vault.lock().await;
    for (field, credential) in &messenger_config.secret_refs {
        match mgr.read_service_credential(credential) {
            Ok(Some(value)) => {
                // The error is deliberately not logged: `set_field`'s parse
                // messages quote the offending value, and the value here is
                // a decrypted credential. "Never log secrets" outranks a
                // better diagnostic.
                if resolved.set_field(field, value.as_str()).is_err() {
                    warn!(
                        account = %messenger_config.name,
                        field,
                        "Vaulted credential does not fit this field; leaving it unset"
                    );
                }
            }
            Ok(None) => warn!(
                account = %messenger_config.name,
                field,
                credential,
                "Credential is referenced by config but missing from the vault"
            ),
            Err(e) => warn!(
                account = %messenger_config.name,
                field,
                error = %e,
                "Could not read credential from the vault"
            ),
        }
    }
    resolved
}

/// Resolve an account's presented identity against its agent.
fn resolved_profile(
    messenger_config: &rustyclaw_core::config::MessengerConfig,
    config: &Config,
) -> rustyclaw_core::messengers::setup::ResolvedProfile {
    let profile = messenger_config.profile.clone().unwrap_or_default();
    let registry = config.agent_registry();
    let agent = registry.get(
        profile
            .agent_id
            .as_deref()
            .unwrap_or(rustyclaw_core::agents::MAIN_AGENT_ID),
    );
    let (name, description) = match agent {
        Some(info) => (info.name, info.description),
        None => (config.agent_name.clone(), None),
    };
    profile.resolve(&name, description.as_deref())
}

/// Apply the parts of a profile that are config-level rather than API calls.
///
/// Only IRC has a name that is chosen at connect time. Everywhere else the
/// display name belongs to the account itself and is set on the platform, not
/// by us — see [`announce_profile`] for what can be pushed after connecting.
fn apply_profile(
    mut messenger_config: rustyclaw_core::config::MessengerConfig,
    config: &Config,
) -> rustyclaw_core::config::MessengerConfig {
    if messenger_config.messenger_type == "irc" && messenger_config.nick.is_none() {
        let profile = resolved_profile(&messenger_config, config);
        // IRC nicks are far more restricted than display names, so the
        // agent's name is reduced to something a server will accept rather
        // than rejected outright at connect time.
        let nick: String = profile
            .display_name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .take(16)
            .collect();
        if !nick.is_empty() {
            messenger_config.nick = Some(nick);
        }
    }
    messenger_config
}

/// Push the profile fields the platform will accept.
///
/// Both calls are no-ops on backends that do not support them — that is the
/// trait's contract — so a failure here is worth a log and nothing more. It
/// must not stop a messenger that is otherwise connected and working.
async fn announce_profile(
    messenger: &dyn Messenger,
    profile: &rustyclaw_core::messengers::setup::ResolvedProfile,
    config: &Config,
) {
    if let Some(bio) = profile.bio.as_deref() {
        if let Err(e) = messenger.set_text_status(bio).await {
            debug!(messenger = %messenger.name(), error = %e, "Could not set status text");
        }
    }
    if let Some(avatar) = profile.avatar_path.as_deref() {
        // Validated at save time AND here. The path was checked when the user
        // configured it, but this upload happens on every connect, arbitrarily
        // later — the file (or a symlink in its place) can have changed in
        // between, and it is this read that leaves the host.
        match crate::messenger_config_handler::validate_avatar(avatar, config) {
            Ok(path) => {
                // Location checks can't vouch for *contents*: anything with
                // workspace write access (the agent's file tools included)
                // could have replaced the image. Only bytes the user has
                // blessed get published — a fingerprint recorded at save
                // time that still matches. No fingerprint means no upload,
                // even for a hand-written config: leaving that case open
                // made "an unpinned profile" the standing loophole, and a
                // hand-editor can bless their image either by saving the
                // profile once in the setup panel or by writing the
                // `avatar_sha256` field beside `avatar_path`.
                //
                // The bytes are read ONCE, verified, and re-staged in a
                // gateway-private file the backend uploads from. Handing the
                // backend the workspace path would have it re-open a file
                // anyone with workspace access can swap between our hash and
                // its read — the exact race the pin exists to close.
                let verified = match (profile.avatar_sha256.as_deref(), std::fs::read(&path)) {
                    (Some(saved), Ok(bytes))
                        if saved
                            == rustyclaw_core::messengers::setup::bytes_fingerprint(&bytes) =>
                    {
                        Some(bytes)
                    }
                    _ => None,
                };
                match verified {
                    Some(bytes) => {
                        if let Err(e) = upload_staged_avatar(messenger, config, &path, &bytes).await
                        {
                            debug!(messenger = %messenger.name(), error = %e, "Could not set avatar");
                        }
                    }
                    None => warn!(
                        path = %path.display(),
                        "Avatar has no recorded fingerprint or its contents changed since \
                         the profile was saved; not publishing it — save the profile in \
                         the messenger setup panel to bless this image"
                    ),
                }
            }
            Err(reasons) => warn!(
                path = %avatar.display(),
                reason = %reasons.join("; "),
                "Configured avatar no longer passes validation; leaving the current picture alone"
            ),
        }
    }
}

/// Hand verified avatar bytes to the backend via a gateway-private file.
///
/// The staging directory lives under the settings dir, which nothing but the
/// gateway writes; the file is removed again once the upload returns. The
/// name carries the original extension so backends that infer content type
/// from it keep working.
async fn upload_staged_avatar(
    messenger: &dyn Messenger,
    config: &Config,
    original: &std::path::Path,
    bytes: &[u8],
) -> Result<()> {
    let staging = config.settings_dir.join("runtime");
    std::fs::create_dir_all(&staging)?;
    let ext = original
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png");
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let staged = staging.join(format!("avatar-{}-{unique}.{ext}", std::process::id()));
    std::fs::write(&staged, bytes)?;

    let url = format!("file://{}", staged.display());
    let result = messenger.set_profile_picture(&url).await;
    // Best-effort cleanup either way; a leftover file in the gateway's own
    // runtime dir is untidy, not dangerous.
    std::fs::remove_file(&staged).ignore();
    result
}

/// Create a messenger manager from config.
///
/// Credentials come from the vault where the account has been migrated to it,
/// and from plaintext config where it has not — see [`resolve_credentials`].
pub async fn create_messenger_manager(
    config: &Config,
    vault: &SharedVault,
) -> Result<MessengerManager> {
    let mut manager = MessengerManager::new();

    for messenger_config in &config.messengers {
        if !messenger_config.enabled {
            continue;
        }
        let messenger_config =
            &apply_profile(resolve_credentials(messenger_config, vault).await, config);
        match create_messenger(messenger_config).await {
            Ok(messenger) => {
                info!(
                    name = %messenger.name(),
                    messenger_type = %messenger.messenger_type(),
                    "Messenger initialized"
                );
                announce_profile(
                    messenger.as_ref(),
                    &resolved_profile(messenger_config, config),
                    config,
                )
                .await;
                manager = manager.add_boxed(messenger);
            }
            Err(e) => {
                error!(
                    messenger_type = %messenger_config.messenger_type,
                    error = %e,
                    "Failed to initialize messenger"
                );
            }
        }
    }

    Ok(manager)
}

/// Where a message's conversation lives, and where its tools run.
///
/// Without a route these are the historical defaults: a conversation keyed by
/// `"<type>:<channel>"` and the installation-wide workspace. A route replaces
/// both — that is what binding a channel to a thread *means*.
struct Routing {
    /// Key into the conversation store.
    conv_key: String,
    /// Directory tools run in.
    workspace_dir: std::path::PathBuf,
    /// Thread the route named, for logging and the system prompt.
    thread: Option<(u64, String)>,
    /// Agent whose tools these are, and so who owns anything they start.
    /// A transfer needs an owner to be listable and stoppable at all.
    agent_id: String,
}

/// Resolve where a message belongs.
///
/// Matched on the account name the message actually arrived on, so a second
/// account of the same type gets its own routes rather than the first one's.
fn resolve_routing(
    config: &Config,
    account_name: &str,
    messenger_type: &str,
    msg: &Message,
) -> Routing {
    let channel = msg.channel.as_deref();
    let fallback = || Routing {
        conv_key: format!("{messenger_type}:{}", channel.unwrap_or(&msg.sender)),
        workspace_dir: config.workspace_dir(),
        thread: None,
        // Paired with the workspace above: an unrouted message runs in the
        // installation-wide directory, which is the main agent's.
        agent_id: rustyclaw_core::agents::MAIN_AGENT_ID.to_string(),
    };

    // By name; falling back to the first enabled account of this type only
    // when the manager's name matches nothing in config.
    let account = config
        .messengers
        .iter()
        .find(|m| m.name == account_name && m.enabled)
        .or_else(|| {
            config
                .messengers
                .iter()
                .find(|m| m.messenger_type == messenger_type && m.enabled)
        });
    let Some(account) = account else {
        return fallback();
    };

    let Some(route) = rustyclaw_core::messengers::setup::route_for(
        &config.messenger_routes,
        &account.name,
        channel,
    ) else {
        return fallback();
    };

    // The thread is read from disk each time rather than cached: a client may
    // have renamed it, moved it, or deleted it since the loop started, and a
    // stale working directory is a tool call in the wrong repository. A
    // read-only peek: resolving a route must not create stores or threads
    // (this runs per inbound message), and the legacy loader it replaced
    // read a `threads.json` migration has renamed away, so it found nothing
    // and every route fell back to the default workspace.
    let threads_path = config.sessions_dir_for(route.agent()).join("threads.json");
    let threads = rustyclaw_core::threads::ThreadStore::peek(&threads_path);
    let Some(thread) = threads
        .as_deref()
        .and_then(|ts| ts.iter().find(|t| t.id == route.thread_id))
    else {
        warn!(
            account = %account.name,
            thread_id = route.thread_id,
            agent = route.agent(),
            "Route points at a thread that no longer exists; using the default workspace"
        );
        return fallback();
    };

    Routing {
        // Keyed by thread, not by channel: two channels routed to one thread
        // share a conversation, which is the point of routing them together.
        //
        // Direct messages are the exception: keying purely by thread would
        // put every correspondent into one history, replaying one person's
        // private messages as context for answering another. They keep a
        // per-sender key and take only the thread's working directory.
        //
        // "Direct" is the backend's `is_direct` flag OR a missing channel —
        // not the channel alone. Telegram and friends give private chats a
        // chat id, so a DM there arrives with `channel: Some(..)`, and
        // testing only the channel merged exactly the conversations this
        // split exists to keep apart.
        conv_key: if msg.is_direct || channel.is_none() {
            format!(
                "thread:{}:{}:dm:{}",
                route.agent(),
                route.thread_id,
                msg.sender
            )
        } else {
            format!("thread:{}:{}", route.agent(), route.thread_id)
        },
        workspace_dir: thread
            .working_dir
            .clone()
            .unwrap_or_else(|| config.workspace_dir_for(route.agent())),
        thread: Some((route.thread_id, thread.label.clone())),
        agent_id: route.agent().to_string(),
    }
}

/// The connection a reply (or typing indicator) should leave through.
///
/// Messages are tagged with the account they arrived on, and the answer must
/// go back out the same way: with two accounts on one platform, resolving by
/// type alone sends the second account's replies through the first account's
/// bot. The type-based fallback covers a messenger built from something other
/// than this config (tests, hand-rolled setups), where no name matches.
fn get_messenger_for_account<'a>(
    mgr: &'a MessengerManager,
    account_name: &str,
    messenger_type: &str,
) -> Option<&'a dyn Messenger> {
    let messengers = mgr.messengers();
    messengers
        .iter()
        .find(|messenger| messenger.name() == account_name)
        .or_else(|| {
            messengers
                .iter()
                .find(|messenger| messenger.messenger_type() == messenger_type)
        })
        .map(|messenger| messenger.as_ref())
}

/// Run the messenger polling loop.
///
/// This polls all configured messengers for incoming messages and routes
/// them through the model for processing with full tool support.
///
/// When `messenger_max_concurrent` > 1, messages are processed concurrently.
/// The loop continues polling for new messages while processing tasks run.
///
/// Reads the **shared** config, not a startup snapshot: the setup panel
/// saves accounts and routes into it, and a loop pinned to a boot-time copy
/// reported those saves as done while never connecting or routing anything
/// until the gateway restarted. Each tick picks up the current config;
/// a change to the account list rebuilds the connections.
pub async fn run_messenger_loop(
    shared_config: crate::SharedConfig,
    messenger_mgr: SharedMessengerManager,
    model_ctx: Option<Arc<ModelContext>>,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    task_mgr: super::SharedTaskManager,
    model_registry: super::SharedModelRegistry,
    copilot_session: Option<Arc<super::CopilotSession>>,
    cancel: CancellationToken,
) -> Result<()> {
    // If no model context, we can't process messages
    let model_ctx = match model_ctx {
        Some(ctx) => ctx,
        None => {
            warn!("No model context — messenger loop disabled");
            return Ok(());
        }
    };

    let mut config = shared_config.read().await.clone();

    let poll_interval =
        Duration::from_millis(config.messenger_poll_interval_ms.unwrap_or(2000).max(500) as u64);

    // Concurrent processing setup
    let max_concurrent = config.messenger_max_concurrent.unwrap_or(1);
    let concurrent_mode = max_concurrent > 1;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

    if concurrent_mode {
        info!(max_concurrent, "Concurrent message processing enabled");
    }

    // Per-chat conversation history
    let conversations: ConversationStore = Arc::new(Mutex::new(HashMap::new()));

    // IDs of messages the agent sent, for reply-to-agent relevance.
    let sent_tracker: Arc<std::sync::Mutex<SentMessageTracker>> =
        Arc::new(std::sync::Mutex::new(SentMessageTracker::new()));

    let http = Arc::new(
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()?,
    );

    info!(
        poll_interval_ms = poll_interval.as_millis(),
        "Starting messenger loop"
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Shutting down messenger loop");
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {
                // Pick up setup-panel edits without a restart. The account
                // list decides which connections exist, so a change there
                // rebuilds them; routes and profiles are read from the fresh
                // snapshot on every message either way.
                let fresh = shared_config.read().await.clone();
                if fresh.messengers != config.messengers {
                    match create_messenger_manager(&fresh, &vault).await {
                        Ok(new_mgr) => {
                            *messenger_mgr.lock().await = new_mgr;
                            info!(
                                accounts = fresh.messengers.len(),
                                "Messenger connections rebuilt after a config change"
                            );
                        }
                        Err(e) => {
                            error!(error = %e, "Could not rebuild messengers after a config change");
                        }
                    }
                }
                config = fresh;

                // Mention names per account for the relevance pre-filter.
                // Computed once per tick; the profile resolution is cheap
                // and accounts are few.
                let tokens_by_account: HashMap<String, Vec<String>> = config
                    .messengers
                    .iter()
                    .map(|mc| (mc.name.clone(), mention_tokens(&config, mc)))
                    .collect();

                // The loop runs on every gateway so the setup panel's first
                // account is noticed — but with nothing configured, a tick
                // is just the config check above.
                if config.messengers.is_empty() {
                    continue;
                }

                // Poll all messengers for incoming messages
                let messages = {
                    let mgr = messenger_mgr.lock().await;
                    poll_all_messengers(&mgr).await
                };
                trace!(count = messages.len(), "Polled messengers");

                // Process each message
                for (account_name, messenger_type, msg) in messages {
                    // Relevance pre-filter (rule tier of #165): under
                    // `relevance_filter = "mentions"`, group-chat messages
                    // that neither mention the agent nor reply to one of its
                    // messages are dropped before any token spend, history
                    // lookup, or typing indicator.
                    //
                    // An account absent from `tokens_by_account` means we
                    // cannot determine its mention names: do not drop its
                    // group messages on an empty token list, pass them
                    // through instead (legacy behavior for unknown accounts).
                    let relevant = match tokens_by_account.get(&account_name) {
                        Some(tokens) => is_message_relevant(
                            &config,
                            &account_name,
                            &msg,
                            tokens,
                            // A poisoned lock (a panicked task) must not
                            // take the whole messenger poll down with it:
                            // recover the guard and continue.
                            &sent_tracker.lock().unwrap_or_else(|e| e.into_inner()),
                        ),
                        None => true,
                    };
                    if !relevant {
                        debug!(
                            account_name,
                            sender = %msg.sender,
                            channel = ?msg.channel,
                            "Skipping irrelevant group-chat message (relevance_filter = \"mentions\")"
                        );
                        continue;
                    }

                    if concurrent_mode {
                        // Spawn message processing as a background task
                        let http = Arc::clone(&http);
                        let config = config.clone();
                        let messenger_mgr = Arc::clone(&messenger_mgr);
                        let model_ctx = Arc::clone(&model_ctx);
                        let vault = Arc::clone(&vault);
                        let skill_mgr = Arc::clone(&skill_mgr);
                        let task_mgr = Arc::clone(&task_mgr);
                        let model_registry = Arc::clone(&model_registry);
                        let copilot_session = copilot_session.clone();
                        let conversations = Arc::clone(&conversations);
                        let semaphore = Arc::clone(&semaphore);
                        let messenger_type = messenger_type.clone();
                        let account_name = account_name.clone();
                        let sent_tracker = Arc::clone(&sent_tracker);

                        tokio::spawn(async move {
                            // Acquire permit (blocks if at capacity)
                            let _permit = semaphore.acquire().await;

                            // Set typing indicator
                            if let Some(channel) = &msg.channel {
                                let mgr = messenger_mgr.lock().await;
                                if let Some(messenger) = get_messenger_for_account(&mgr, &account_name, &messenger_type) {
                                    messenger.set_typing(channel, true).await.ignore();
                                }
                            }

                            let channel_for_typing = msg.channel.clone();
                            let result = process_incoming_message(
                                &http,
                                &config,
                                &messenger_mgr,
                                &model_ctx,
                                &vault,
                                &skill_mgr,
                                &task_mgr,
                                &model_registry,
                                copilot_session.as_deref(),
                                &conversations,
                                &account_name,
                                &messenger_type,
                                sent_tracker,
                                msg,
                            )
                            .await;

                            // Clear typing indicator
                            if let Some(channel) = channel_for_typing {
                                let mgr = messenger_mgr.lock().await;
                                if let Some(messenger) = get_messenger_for_account(&mgr, &account_name, &messenger_type) {
                                    messenger.set_typing(&channel, false).await.ignore();
                                }
                            }

                            if let Err(e) = result {
                                error!(error = %e, "Error processing message");
                            }
                        });
                    } else {
                        // Sequential mode (original behavior)
                        // Set typing indicator before processing
                        if let Some(channel) = &msg.channel {
                            let mgr = messenger_mgr.lock().await;
                            if let Some(messenger) = get_messenger_for_account(&mgr, &account_name, &messenger_type) {
                                messenger.set_typing(channel, true).await.ignore();
                            }
                        }

                        let channel_for_typing = msg.channel.clone();
                        let result = process_incoming_message(
                            &http,
                            &config,
                            &messenger_mgr,
                            &model_ctx,
                            &vault,
                            &skill_mgr,
                            &task_mgr,
                            &model_registry,
                            copilot_session.as_deref(),
                            &conversations,
                            &account_name,
                            &messenger_type,
                            Arc::clone(&sent_tracker),
                            msg,
                        )
                        .await;

                        // Clear typing indicator after processing
                        if let Some(channel) = channel_for_typing {
                            let mgr = messenger_mgr.lock().await;
                            if let Some(messenger) = get_messenger_for_account(&mgr, &account_name, &messenger_type) {
                                messenger.set_typing(&channel, false).await.ignore();
                            }
                        }

                        if let Err(e) = result {
                            error!(error = %e, "Error processing message");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Poll all messengers and collect incoming messages.
/// Poll every messenger, tagging each message with the account it came from.
///
/// The account *name* matters as much as the type: routes and profiles are
/// keyed by name, and deriving the name back from the type picks whichever
/// account happens to be listed first — so with two Telegram accounts the
/// second one's routes and presented identity would never take effect.
async fn poll_all_messengers(mgr: &MessengerManager) -> Vec<(String, String, Message)> {
    let mut all_messages = Vec::new();

    for messenger in mgr.messengers() {
        match messenger.receive_messages().await {
            Ok(messages) => {
                for msg in messages {
                    all_messages.push((
                        messenger.name().to_string(),
                        messenger.messenger_type().to_string(),
                        msg,
                    ));
                }
            }
            Err(e) => {
                debug!(
                    messenger_type = %messenger.messenger_type(),
                    error = %e,
                    "Error polling messenger"
                );
            }
        }
    }

    all_messages
}

/// Process an incoming message through the model with full tool loop.
async fn process_incoming_message(
    http: &reqwest::Client,
    config: &Config,
    messenger_mgr: &SharedMessengerManager,
    model_ctx: &Arc<ModelContext>,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    task_mgr: &super::SharedTaskManager,
    model_registry: &super::SharedModelRegistry,
    copilot_session: Option<&super::CopilotSession>,
    conversations: &ConversationStore,
    account_name: &str,
    messenger_type: &str,
    sent_tracker: Arc<std::sync::Mutex<SentMessageTracker>>,
    msg: Message,
) -> Result<()> {
    debug!(
        sender = %msg.sender,
        messenger_type = %messenger_type,
        content_preview = %if msg.content.len() > 50 {
            format!("{}...", &msg.content[..50])
        } else {
            msg.content.clone()
        },
        "Received message"
    );

    // A route binds this channel to a gateway thread, which decides both which
    // conversation the message joins and where its tools run.
    let routing = resolve_routing(config, account_name, messenger_type, &msg);
    let workspace_dir = routing.workspace_dir.clone();
    // Owns anything the tools below start: a transfer with no owner is listed
    // by no panel, refused by cancel, and therefore unstoppable.
    let agent_id = routing.agent_id.clone();
    let conv_key = routing.conv_key.clone();
    if let Some((thread_id, label)) = &routing.thread {
        debug!(
            thread_id,
            thread_label = %label,
            workspace = %workspace_dir.display(),
            "Message routed to a gateway thread"
        );
    }

    // Get or create conversation history
    let mut messages = {
        let mut store = conversations.lock().await;
        store
            .entry(conv_key.clone())
            .or_insert_with(Vec::new)
            .clone()
    };

    // Build system prompt (async to include task and model context)
    let account = config
        .messengers
        .iter()
        .find(|m| m.name == account_name)
        .or_else(|| {
            config
                .messengers
                .iter()
                .find(|m| m.messenger_type == messenger_type && m.enabled)
        });
    let profile = match account {
        Some(a) => resolved_profile(a, config),
        // No matching account means the messenger was built from something
        // other than this config; fall back to the agent's own identity.
        None => rustyclaw_core::messengers::setup::MessengerProfile::default()
            .resolve(&config.agent_name, None),
    };
    let identity = prompt::MessengerIdentity {
        profile: &profile,
        thread: routing.thread.as_ref().map(|(id, l)| (*id, l.as_str())),
        workspace_dir: &workspace_dir,
    };
    let system_prompt = build_messenger_system_prompt(
        config,
        messenger_type,
        &msg,
        task_mgr,
        model_registry,
        skill_mgr,
        &conv_key,
        &identity,
    )
    .await;

    // Add system message if not present
    if messages.is_empty() || messages[0].role != "system" {
        messages.insert(0, ChatMessage::text("system", &system_prompt));
    } else {
        // Update system prompt
        messages[0].content = system_prompt.clone();
    }

    // Media cache directory
    let cache_dir = config.credentials_dir().join("media_cache");

    // Process any image attachments
    let images = if let Some(attachments) = &msg.media {
        process_attachments(http, attachments, &cache_dir).await
    } else {
        Vec::new()
    };

    if !images.is_empty() {
        debug!(
            image_count = images.len(),
            "Processing images (vision not yet supported in messenger handler)"
        );
    }

    // Build media refs for history storage
    let media_refs: Vec<MediaRef> = images.iter().map(|img| img.media_ref.clone()).collect();

    // Add user message to history (with media refs, not raw data)
    messages.push(ChatMessage::user_with_media(
        &msg.content,
        media_refs.clone(),
    ));

    // Resolve effective bearer token (handles Copilot session exchange)
    let effective_key = crate::auth::resolve_bearer_token(
        http,
        &model_ctx.provider,
        model_ctx.api_key.as_deref(),
        copilot_session,
    )
    .await
    .ok()
    .flatten();

    // Build request - ProviderRequest expects Vec<ChatMessage>
    let mut resolved = ProviderRequest {
        provider: model_ctx.provider.clone(),
        model: model_ctx.model.clone(),
        base_url: model_ctx.base_url.clone(),
        api_key: effective_key,
        messages: messages.clone(),
        allowed_tools: None,
        reasoning_effort: model_ctx.reasoning_effort.clone(),
        max_tokens: model_ctx.max_tokens,
        temperature: model_ctx.temperature,
    };

    // Run the agentic tool loop
    let mut final_response = String::new();

    for _round in 0..MAX_TOOL_ROUNDS {
        let result = providers::call_with_tools(http, &resolved, None).await;

        let model_resp = match result {
            Ok(r) => r,
            Err(err) => {
                error!(error = %err, "Model error");
                return Err(err);
            }
        };

        // Collect text response
        if !model_resp.text.is_empty() {
            final_response.push_str(&model_resp.text);
        }

        if model_resp.tool_calls.is_empty() {
            // No tool calls — done
            break;
        }

        // Execute each requested tool
        let mut tool_results: Vec<ToolCallResult> = Vec::new();

        for tc in &model_resp.tool_calls {
            debug!(tool_name = %tc.name, tool_id = %tc.id, "Executing tool call");

            let (output, is_error) = if tools::is_secrets_tool(&tc.name) {
                match secrets_handler::execute_secrets_tool(&tc.name, &tc.arguments, vault).await {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if tools::is_skill_tool(&tc.name) {
                match skills_handler::execute_skill_tool(&tc.name, &tc.arguments, skill_mgr).await {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if crate::mcp_handler::is_mcp_tool(&tc.name) {
                #[cfg(feature = "mcp")]
                {
                    // MCP tools require the MCP manager - for now, return an error
                    // TODO: Pass mcp_mgr to this function
                    (
                        format!(
                            "MCP tool '{}' called but MCP manager not available in this context",
                            tc.name
                        ),
                        true,
                    )
                }
                #[cfg(not(feature = "mcp"))]
                {
                    (
                        format!("MCP tool '{}' requires the 'mcp' feature", tc.name),
                        true,
                    )
                }
            } else if super::canvas_handler::is_canvas_tool(&tc.name) {
                // Canvas tools require the canvas host - for now, return an error
                // TODO: Pass canvas_host to this function
                (
                    format!(
                        "Canvas tool '{}' called but canvas host not available in this context",
                        tc.name
                    ),
                    true,
                )
            } else if crate::task_handler::is_task_tool(&tc.name) {
                // Execute task tool with task manager
                match crate::task_handler::execute_task_tool(
                    &tc.name,
                    &tc.arguments,
                    task_mgr,
                    Some(&conv_key),
                )
                .await
                {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else if crate::command_wrapper::should_wrap_in_task(&tc.name) {
                // Wrap execute_command in a Task
                let task_id =
                    crate::command_wrapper::start_command_task(task_mgr, &tc.arguments, &conv_key)
                        .await;

                // Scoped to the conversation, so a process backgrounded for
                // one chat is not reachable from another chat or from the
                // TUI's threads, and its tool budget is its own.
                let caller = format!("messenger:{conv_key}");
                let result = match rustyclaw_core::tool_limits::check_rate(Some(&caller), &tc.name)
                {
                    Err(err) => Err(err.to_string().into()),
                    Ok(()) => {
                        rustyclaw_core::tool_caller::with_caller(
                            caller,
                            rustyclaw_core::downloads::with_origin(
                                rustyclaw_core::downloads::headless_origin(&agent_id),
                                tools::execute_tool(&tc.name, &tc.arguments, &workspace_dir),
                            ),
                        )
                        .await
                    }
                };

                match result {
                    Ok(output) => {
                        // Check if it was backgrounded
                        if let Some(session_id) = crate::command_wrapper::parse_session_id(&output)
                        {
                            crate::command_wrapper::update_command_task_session(
                                task_mgr,
                                task_id,
                                &session_id,
                            )
                            .await;
                        } else {
                            crate::command_wrapper::complete_command_task(
                                task_mgr, task_id, &output,
                            )
                            .await;
                        }
                        (output, false)
                    }
                    Err(err) => {
                        let err = err.to_string();
                        crate::command_wrapper::fail_command_task(task_mgr, task_id, &err).await;
                        (err, true)
                    }
                }
            } else if crate::model_handler::is_model_tool(&tc.name) {
                // Model management tools
                match crate::model_handler::execute_model_tool(
                    &tc.name,
                    &tc.arguments,
                    model_registry,
                )
                .await
                {
                    Ok(text) => (text, false),
                    Err(err) => (err.to_string(), true),
                }
            } else {
                {
                    let caller = format!("messenger:{conv_key}");
                    match rustyclaw_core::tool_limits::check_rate(Some(&caller), &tc.name) {
                        Err(err) => (err.to_string(), true),
                        Ok(()) => match rustyclaw_core::tool_caller::with_caller(
                            caller,
                            rustyclaw_core::downloads::with_origin(
                                rustyclaw_core::downloads::headless_origin(&agent_id),
                                tools::execute_tool(&tc.name, &tc.arguments, &workspace_dir),
                            ),
                        )
                        .await
                        {
                            Ok(text) => (text, false),
                            Err(err) => (err.to_string(), true),
                        },
                    }
                }
            };

            trace!(
                tool_name = %tc.name,
                is_error = is_error,
                output_preview = %rustyclaw_core::text::truncate_chars(&output, 100),
                "Tool result"
            );

            tool_results.push(ToolCallResult {
                id: tc.id.clone(),
                name: tc.name.clone(),
                output,
                is_error,
            });
        }

        // Append tool round to conversation
        providers::append_tool_round(
            &resolved.provider,
            &mut resolved.messages,
            &model_resp,
            &tool_results,
        );
    }

    // Update conversation history
    {
        let mut store = conversations.lock().await;
        let history = store.entry(conv_key).or_insert_with(Vec::new);

        // Add user message (with media refs)
        history.push(ChatMessage::user_with_media(
            &msg.content,
            media_refs.clone(),
        ));

        // Add assistant response
        if !final_response.is_empty() {
            history.push(ChatMessage::text("assistant", &final_response));
        }

        // Trim history if too long (keep system message)
        while history.len() > MAX_HISTORY_MESSAGES {
            if history.len() > 1 && history[1].role != "system" {
                history.remove(1);
            } else {
                break;
            }
        }
    }

    // Send response back via messenger
    if !final_response.is_empty()
        && final_response.trim() != "NO_REPLY"
        && final_response.trim() != "HEARTBEAT_OK"
    {
        let mgr = messenger_mgr.lock().await;
        if let Some(messenger) = get_messenger_for_account(&mgr, account_name, messenger_type) {
            let recipient = msg.channel.as_deref().unwrap_or(&msg.sender);

            let opts = SendOptions {
                recipient,
                content: &final_response,
                reply_to: Some(&msg.id),
                thread_id: None,
                silent: false,
                media: None,
            };

            match messenger.send_message_with_options(opts).await {
                Ok(msg_id) => {
                    // Remember the sent ID so a reply to it counts as
                    // relevant under `relevance_filter = "mentions"`.
                    // Poisoned-lock recovery: one panicked task must not
                    // stop reply tracking (and with it, all messenger
                    // handling).
                    sent_tracker
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .record(&channel_key(account_name, msg.channel.as_deref()), &msg_id);
                    debug!(
                        message_id = %msg_id,
                        response_preview = %rustyclaw_core::text::truncate_chars(&final_response, 50),
                        "Sent response"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Failed to send response");
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::messengers::setup::{MessengerProfile, ThreadRoute};
    use rustyclaw_core::threads::ThreadManager;

    /// A config rooted in a temp dir, with one enabled account.
    fn test_config(dir: &std::path::Path, messenger_type: &str) -> Config {
        let mut config = Config {
            settings_dir: dir.to_path_buf(),
            agent_name: "Ada".to_string(),
            ..Config::default()
        };
        config.settings_dir = dir.to_path_buf();
        config.workspace_dir = Some(dir.join("workspace"));
        config.messengers = vec![rustyclaw_core::config::MessengerConfig {
            name: "acct".to_string(),
            messenger_type: messenger_type.to_string(),
            enabled: true,
            ..Default::default()
        }];
        config
    }

    /// Persist a thread for the main agent and return its id.
    fn write_thread(config: &Config, label: &str, working_dir: Option<&std::path::Path>) -> u64 {
        let sessions = config.sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID);
        std::fs::create_dir_all(&sessions).unwrap();
        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat(label);
        if let Some(dir) = working_dir {
            mgr.set_working_dir(id, Some(dir.to_path_buf()));
        }
        // Through the store, as the gateway persists: seeding the legacy
        // `threads.json` masked the production loader reading a file that
        // no longer exists.
        rustyclaw_core::threads::ThreadStore::at_legacy_path(&sessions.join("threads.json"))
            .persist(&mut mgr)
            .unwrap();
        id.0
    }

    fn message(channel: Option<&str>) -> Message {
        Message {
            id: "1".into(),
            sender: "someone".into(),
            content: "hi".into(),
            timestamp: 0,
            channel: channel.map(str::to_string),
            reply_to: None,
            thread_id: None,
            media: None,
            is_direct: channel.is_none(),
            message_type: Default::default(),
            edited_timestamp: None,
            reactions: None,
        }
    }

    #[test]
    fn an_unrouted_channel_keeps_its_own_conversation_and_the_default_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path(), "telegram");
        let routing = resolve_routing(&config, "acct", "telegram", &message(Some("-100")));
        assert_eq!(routing.conv_key, "telegram:-100");
        assert_eq!(routing.workspace_dir, config.workspace_dir());
        assert!(routing.thread.is_none());
    }

    #[test]
    fn a_direct_message_without_a_channel_is_keyed_by_sender() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path(), "telegram");
        let routing = resolve_routing(&config, "acct", "telegram", &message(None));
        assert_eq!(routing.conv_key, "telegram:someone");
    }

    #[test]
    fn a_routed_channel_adopts_the_threads_key_and_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path(), "telegram");
        let project = dir.path().join("project");
        let thread_id = write_thread(&config, "Support", Some(&project));
        config.messenger_routes = vec![ThreadRoute {
            messenger: "acct".into(),
            channel: Some("-100".into()),
            thread_id,
            agent_id: None,
            enabled: true,
        }];

        let routing = resolve_routing(&config, "acct", "telegram", &message(Some("-100")));
        assert_eq!(routing.conv_key, format!("thread:main:{thread_id}"));
        assert_eq!(routing.workspace_dir, project);
        assert_eq!(routing.thread, Some((thread_id, "Support".to_string())));
    }

    #[test]
    fn two_channels_on_one_thread_share_a_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path(), "telegram");
        let thread_id = write_thread(&config, "Shared", None);
        config.messenger_routes = vec![
            ThreadRoute {
                messenger: "acct".into(),
                channel: Some("a".into()),
                thread_id,
                agent_id: None,
                enabled: true,
            },
            ThreadRoute {
                messenger: "acct".into(),
                channel: Some("b".into()),
                thread_id,
                agent_id: None,
                enabled: true,
            },
        ];

        let a = resolve_routing(&config, "acct", "telegram", &message(Some("a")));
        let b = resolve_routing(&config, "acct", "telegram", &message(Some("b")));
        assert_eq!(
            a.conv_key, b.conv_key,
            "bridged channels must share history"
        );
    }

    #[test]
    fn a_thread_without_an_override_inherits_the_agent_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path(), "telegram");
        let thread_id = write_thread(&config, "Main", None);
        config.messenger_routes = vec![ThreadRoute {
            messenger: "acct".into(),
            channel: None,
            thread_id,
            agent_id: None,
            enabled: true,
        }];
        let routing = resolve_routing(&config, "acct", "telegram", &message(Some("anything")));
        assert_eq!(routing.workspace_dir, config.workspace_dir());
    }

    #[test]
    fn a_route_to_a_deleted_thread_falls_back_rather_than_running_in_the_wrong_place() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path(), "telegram");
        // No thread is ever written, so the id names nothing.
        config.messenger_routes = vec![ThreadRoute {
            messenger: "acct".into(),
            channel: Some("-100".into()),
            thread_id: 9999,
            agent_id: None,
            enabled: true,
        }];
        let routing = resolve_routing(&config, "acct", "telegram", &message(Some("-100")));
        assert_eq!(routing.conv_key, "telegram:-100");
        assert_eq!(routing.workspace_dir, config.workspace_dir());
        assert!(routing.thread.is_none());
    }

    #[test]
    fn a_disabled_account_is_not_matched_by_its_routes() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path(), "telegram");
        let thread_id = write_thread(&config, "Support", None);
        config.messengers[0].enabled = false;
        config.messenger_routes = vec![ThreadRoute {
            messenger: "acct".into(),
            channel: None,
            thread_id,
            agent_id: None,
            enabled: true,
        }];
        let routing = resolve_routing(&config, "acct", "telegram", &message(Some("c")));
        assert!(routing.thread.is_none());
    }

    #[test]
    fn a_second_account_of_the_same_type_gets_its_own_routes() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path(), "telegram");
        let thread_id = write_thread(&config, "Support", None);
        // Two Telegram accounts. Routing is keyed by name, and a message
        // carries only a type, so resolving by type would hand every message
        // to whichever account is listed first.
        config
            .messengers
            .push(rustyclaw_core::config::MessengerConfig {
                name: "second".to_string(),
                messenger_type: "telegram".to_string(),
                enabled: true,
                ..Default::default()
            });
        config.messenger_routes = vec![ThreadRoute {
            messenger: "second".into(),
            channel: Some("-100".into()),
            thread_id,
            agent_id: None,
            enabled: true,
        }];

        let routed = resolve_routing(&config, "second", "telegram", &message(Some("-100")));
        assert_eq!(
            routed.thread,
            Some((thread_id, "Support".to_string())),
            "the second account's own route must apply"
        );

        // And the first account, which has no route, is unaffected.
        let unrouted = resolve_routing(&config, "acct", "telegram", &message(Some("-100")));
        assert!(unrouted.thread.is_none());
    }

    #[test]
    fn a_catch_all_route_does_not_merge_separate_direct_messages() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path(), "telegram");
        let project = dir.path().join("project");
        let thread_id = write_thread(&config, "Inbox", Some(&project));
        config.messenger_routes = vec![ThreadRoute {
            messenger: "acct".into(),
            channel: None,
            thread_id,
            agent_id: None,
            enabled: true,
        }];

        let dm_from = |sender: &str| {
            let mut msg = message(None);
            msg.sender = sender.to_string();
            resolve_routing(&config, "acct", "telegram", &msg)
        };
        let alice = dm_from("alice");
        let bob = dm_from("bob");

        assert_ne!(
            alice.conv_key, bob.conv_key,
            "two people's private messages must not share a history"
        );

        // Telegram-style DMs carry a chat id, so `channel` is Some — the
        // backend's `is_direct` flag is what marks them private. They must
        // split per sender exactly like channel-less DMs.
        let flagged_dm_from = |sender: &str, chat_id: &str| {
            let mut msg = message(Some(chat_id));
            msg.sender = sender.to_string();
            msg.is_direct = true;
            resolve_routing(&config, "acct", "telegram", &msg)
        };
        let carol = flagged_dm_from("carol", "1111");
        let dave = flagged_dm_from("dave", "2222");
        assert_ne!(
            carol.conv_key, dave.conv_key,
            "a DM with a chat id is still a DM — is_direct decides, not the channel"
        );
        // The route still applies: both run in the thread's directory.
        assert_eq!(alice.workspace_dir, project);
        assert_eq!(bob.workspace_dir, project);
        assert_eq!(alice.thread, Some((thread_id, "Inbox".to_string())));
    }

    #[test]
    fn a_catch_all_route_still_shares_one_history_across_channels() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path(), "telegram");
        let thread_id = write_thread(&config, "Shared", None);
        config.messenger_routes = vec![ThreadRoute {
            messenger: "acct".into(),
            channel: None,
            thread_id,
            agent_id: None,
            enabled: true,
        }];

        // Group channels are what a catch-all is for; only DMs are split out.
        let a = resolve_routing(&config, "acct", "telegram", &message(Some("chan-a")));
        let b = resolve_routing(&config, "acct", "telegram", &message(Some("chan-b")));
        assert_eq!(a.conv_key, b.conv_key);
    }

    #[test]
    fn an_irc_nick_is_derived_from_the_profile_when_none_is_configured() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path(), "irc");
        let account = rustyclaw_core::config::MessengerConfig {
            name: "libera".into(),
            messenger_type: "irc".into(),
            enabled: true,
            profile: Some(MessengerProfile {
                display_name: Some("Ada Lovelace!".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let applied = apply_profile(account, &config);
        // Nick charset is far narrower than a display name's, so it is reduced
        // rather than passed through and rejected at connect time.
        assert_eq!(applied.nick.as_deref(), Some("Ada_Lovelace_"));
    }

    #[test]
    fn a_configured_irc_nick_is_not_overwritten_by_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path(), "irc");
        let account = rustyclaw_core::config::MessengerConfig {
            name: "libera".into(),
            messenger_type: "irc".into(),
            enabled: true,
            nick: Some("rustybot".into()),
            profile: Some(MessengerProfile {
                display_name: Some("Ada".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            apply_profile(account, &config).nick.as_deref(),
            Some("rustybot")
        );
    }

    #[test]
    fn an_account_with_no_profile_presents_the_agents_own_name() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path(), "telegram");
        let resolved = resolved_profile(&config.messengers[0], &config);
        assert_eq!(resolved.display_name, "Ada");
    }
}

//! Describing, validating, and provisioning messenger accounts.
//!
//! Setting up a messenger means three separate things, and this module holds
//! the shared vocabulary for all three so the gateway and every client agree:
//!
//! * **Credentials** — what each backend needs to log in. [`kind_spec`]
//!   describes the fields; [`secret_name`] says where the secret ones live in
//!   the vault. Secrets never belong in `config.toml`; see [`plaintext_fields`]
//!   for the migration path off the older layout.
//! * **Profile** — the identity the agent presents on that messenger.
//!   [`MessengerProfile`] holds the overrides; [`ResolvedProfile`] is what you
//!   get after falling back to the agent's own name and description.
//! * **Routing** — which gateway thread a channel's conversation belongs to.
//!   See [`ThreadRoute`].
//!
//! # Why a field schema rather than a form per client
//!
//! There are three clients and a dozen backends. Hard-coding "Telegram needs a
//! token, Matrix needs a homeserver and either a password or an access token"
//! into each client means twelve forms times three, drifting apart the moment
//! a backend changes. Instead the shape of each backend is data here, and the
//! clients render whatever they are given. Adding a messenger type is one
//! entry in [`KINDS`].

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;

// ── Field schema ────────────────────────────────────────────────────────────

/// How a configuration field should be entered and stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// Free text, safe to keep in `config.toml`.
    Text,
    /// A credential. Stored in the vault, never written to config, never sent
    /// back to a client.
    Secret,
    /// Unsigned integer (a port, for instance).
    Number,
    /// Boolean toggle.
    Bool,
    /// Comma- or newline-separated list of strings.
    List,
    /// Filesystem path to something the gateway reads.
    Path,
}

impl FieldKind {
    /// Whether values of this kind belong in the vault rather than in config.
    pub fn is_secret(&self) -> bool {
        matches!(self, Self::Secret)
    }
}

/// Whether a field must be filled in, and if not, what it is grouped with.
/// Not `Copy`: the `OneOf` group name may be owned by a plugin-registered kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    /// Must be present for the account to work.
    Required,
    /// May be left empty.
    Optional,
    /// Part of a named alternatives group: at least one field in the group
    /// must be set. Matrix, for example, accepts a password *or* an access
    /// token, and demanding both would be wrong.
    OneOf(Cow<'static, str>),
}

/// One configurable field on a messenger account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSpec {
    /// Key in [`crate::config::MessengerConfig`] and in the values map.
    pub name: Cow<'static, str>,
    /// Short label for a form.
    pub label: Cow<'static, str>,
    /// How to enter and store it.
    pub kind: FieldKind,
    /// Whether it has to be filled in.
    pub requirement: Requirement,
    /// One line explaining where the user gets this value.
    pub help: Cow<'static, str>,
}

impl FieldSpec {
    const fn required(
        name: &'static str,
        label: &'static str,
        kind: FieldKind,
        help: &'static str,
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            label: Cow::Borrowed(label),
            kind,
            requirement: Requirement::Required,
            help: Cow::Borrowed(help),
        }
    }

    const fn optional(
        name: &'static str,
        label: &'static str,
        kind: FieldKind,
        help: &'static str,
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            label: Cow::Borrowed(label),
            kind,
            requirement: Requirement::Optional,
            help: Cow::Borrowed(help),
        }
    }

    const fn one_of(
        name: &'static str,
        label: &'static str,
        kind: FieldKind,
        group: &'static str,
        help: &'static str,
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            label: Cow::Borrowed(label),
            kind,
            requirement: Requirement::OneOf(Cow::Borrowed(group)),
            help: Cow::Borrowed(help),
        }
    }

    /// Whether this field's value belongs in the vault.
    pub fn is_secret(&self) -> bool {
        self.kind.is_secret()
    }
}

/// Everything the UI needs to configure one kind of messenger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindSpec {
    /// Type id as written to `messenger_type` in config.
    pub id: Cow<'static, str>,
    /// Human-readable name.
    pub label: Cow<'static, str>,
    /// Emoji shown next to the account in listings.
    pub icon: Cow<'static, str>,
    /// One line on what this backend is and what it needs.
    pub summary: Cow<'static, str>,
    /// Cargo feature the gateway must be built with, if any.
    pub feature: Option<Cow<'static, str>>,
    /// Fields the account exposes, in the order a form should show them.
    pub fields: Cow<'static, [FieldSpec]>,
}

impl KindSpec {
    /// Fields whose values go to the vault.
    pub fn secret_fields(&self) -> impl Iterator<Item = &FieldSpec> {
        self.fields.iter().filter(|f| f.is_secret())
    }
}

/// Every messenger type the gateway knows how to build.
///
/// Whether a given entry is *usable* depends on the gateway's compiled
/// features — see [`KindSpec::feature`]. The list is deliberately complete
/// regardless, so a client can explain "this build cannot do Matrix" instead
/// of silently omitting it.
pub static KINDS: &[KindSpec] = &[
    KindSpec {
        id: Cow::Borrowed("telegram"),
        label: Cow::Borrowed("Telegram"),
        icon: Cow::Borrowed("✈️"),
        summary: Cow::Borrowed("Bot account. Create one with @BotFather and paste its token."),
        feature: None,
        fields: Cow::Borrowed(&[FieldSpec::required(
            "token",
            "Bot token",
            FieldKind::Secret,
            "From @BotFather, of the form 123456:ABC-DEF...",
        )]),
    },
    KindSpec {
        id: Cow::Borrowed("discord"),
        label: Cow::Borrowed("Discord"),
        icon: Cow::Borrowed("🎮"),
        summary: Cow::Borrowed("Bot account from the Discord developer portal."),
        feature: None,
        fields: Cow::Borrowed(&[FieldSpec::required(
            "token",
            "Bot token",
            FieldKind::Secret,
            "Developer portal → your application → Bot → Reset Token",
        )]),
    },
    KindSpec {
        id: Cow::Borrowed("slack"),
        label: Cow::Borrowed("Slack"),
        icon: Cow::Borrowed("💼"),
        summary: Cow::Borrowed("Bot user token, optionally with Socket Mode for events."),
        feature: None,
        fields: Cow::Borrowed(&[
            FieldSpec::required(
                "token",
                "Bot token",
                FieldKind::Secret,
                "Starts with xoxb-, from OAuth & Permissions",
            ),
            FieldSpec::optional(
                "app_token",
                "App token",
                FieldKind::Secret,
                "Starts with xapp-; enables Socket Mode",
            ),
            FieldSpec::optional(
                "default_channel",
                "Default channel",
                FieldKind::Text,
                "Channel to post to when a message names none",
            ),
        ]),
    },
    KindSpec {
        id: Cow::Borrowed("matrix"),
        label: Cow::Borrowed("Matrix"),
        icon: Cow::Borrowed("🔗"),
        summary: Cow::Borrowed(
            "Any homeserver. Log in with a password or an existing access token.",
        ),
        feature: Some(Cow::Borrowed("matrix")),
        fields: Cow::Borrowed(&[
            FieldSpec::required(
                "homeserver",
                "Homeserver",
                FieldKind::Text,
                "For example https://matrix.org",
            ),
            FieldSpec::required(
                "user_id",
                "User ID",
                FieldKind::Text,
                "Fully qualified, e.g. @agent:matrix.org",
            ),
            FieldSpec::one_of(
                "password",
                "Password",
                FieldKind::Secret,
                "matrix-auth",
                "Account password; exchanged for an access token on login",
            ),
            FieldSpec::one_of(
                "access_token",
                "Access token",
                FieldKind::Secret,
                "matrix-auth",
                "Use instead of a password if you already have one",
            ),
        ]),
    },
    KindSpec {
        id: Cow::Borrowed("irc"),
        label: Cow::Borrowed("IRC"),
        icon: Cow::Borrowed("💬"),
        summary: Cow::Borrowed("Any IRC network. TLS on port 6697 is the usual choice."),
        feature: None,
        fields: Cow::Borrowed(&[
            FieldSpec::required(
                "server",
                "Server",
                FieldKind::Text,
                "Hostname, e.g. irc.libera.chat",
            ),
            FieldSpec::optional("port", "Port", FieldKind::Number, "Defaults to 6697"),
            FieldSpec::optional(
                "nick",
                "Nickname",
                FieldKind::Text,
                "Defaults to the agent's profile name",
            ),
            FieldSpec::optional(
                "irc_channels",
                "Channels",
                FieldKind::List,
                "Channels to join on connect, e.g. #rust, #rustyclaw",
            ),
            FieldSpec::optional("use_tls", "Use TLS", FieldKind::Bool, "Recommended"),
            FieldSpec::optional(
                "password",
                "Server password",
                FieldKind::Secret,
                "NickServ or server password, if the network needs one",
            ),
        ]),
    },
    KindSpec {
        id: Cow::Borrowed("signal"),
        label: Cow::Borrowed("Signal"),
        icon: Cow::Borrowed("📱"),
        summary: Cow::Borrowed("Talks to a local signal-cli that you have already registered."),
        feature: Some(Cow::Borrowed("signal-cli")),
        fields: Cow::Borrowed(&[FieldSpec::required(
            "phone",
            "Phone number",
            FieldKind::Text,
            "In E.164 form, e.g. +15551234567",
        )]),
    },
    KindSpec {
        id: Cow::Borrowed("whatsapp"),
        label: Cow::Borrowed("WhatsApp"),
        icon: Cow::Borrowed("📞"),
        summary: Cow::Borrowed("Pairs as a linked device; scan the QR code on first start."),
        feature: Some(Cow::Borrowed("whatsapp")),
        fields: Cow::Borrowed(&[]),
    },
    KindSpec {
        id: Cow::Borrowed("google_chat"),
        label: Cow::Borrowed("Google Chat"),
        icon: Cow::Borrowed("🅖"),
        summary: Cow::Borrowed("Incoming webhook, service account, or bot token."),
        feature: None,
        fields: Cow::Borrowed(&[
            FieldSpec::one_of(
                "webhook_url",
                "Webhook URL",
                FieldKind::Secret,
                "gchat-auth",
                "Space → Apps & integrations → Webhooks",
            ),
            FieldSpec::one_of(
                "credentials_path",
                "Service account JSON",
                FieldKind::Path,
                "gchat-auth",
                "Path to the service account key file",
            ),
            FieldSpec::one_of(
                "token",
                "Bot token",
                FieldKind::Secret,
                "gchat-auth",
                "Chat API bot token",
            ),
            FieldSpec::optional(
                "spaces",
                "Spaces",
                FieldKind::List,
                "Space ids to listen on; required for service account mode",
            ),
        ]),
    },
    KindSpec {
        id: Cow::Borrowed("teams"),
        label: Cow::Borrowed("Microsoft Teams"),
        icon: Cow::Borrowed("🟣"),
        summary: Cow::Borrowed("Incoming webhook, or a Bot Framework registration."),
        feature: None,
        fields: Cow::Borrowed(&[
            FieldSpec::one_of(
                "webhook_url",
                "Webhook URL",
                FieldKind::Secret,
                "teams-auth",
                "Channel → Connectors → Incoming Webhook",
            ),
            FieldSpec::one_of(
                "app_id",
                "Bot app ID",
                FieldKind::Text,
                "teams-auth",
                "Azure app registration id",
            ),
            FieldSpec::optional(
                "app_password",
                "Bot app password",
                FieldKind::Secret,
                "Client secret for the app registration",
            ),
        ]),
    },
    KindSpec {
        id: Cow::Borrowed("imessage"),
        label: Cow::Borrowed("iMessage"),
        icon: Cow::Borrowed("🍎"),
        summary: Cow::Borrowed("Reads the local macOS Messages database. macOS only."),
        feature: None,
        fields: Cow::Borrowed(&[FieldSpec::optional(
            "server",
            "chat.db path",
            FieldKind::Path,
            "Defaults to ~/Library/Messages/chat.db",
        )]),
    },
    KindSpec {
        id: Cow::Borrowed("webhook"),
        label: Cow::Borrowed("Webhook"),
        icon: Cow::Borrowed("🪝"),
        summary: Cow::Borrowed("Posts to an arbitrary HTTP endpoint. Send-only."),
        feature: None,
        fields: Cow::Borrowed(&[FieldSpec::required(
            "webhook_url",
            "Endpoint URL",
            FieldKind::Secret,
            "Anything that accepts a JSON POST",
        )]),
    },
    KindSpec {
        id: Cow::Borrowed("console"),
        label: Cow::Borrowed("Console"),
        icon: Cow::Borrowed("🖥️"),
        summary: Cow::Borrowed("Reads and writes the gateway's own stdio. Useful for testing."),
        feature: None,
        fields: Cow::Borrowed(&[]),
    },
];

/// Look up a messenger type's schema.
pub fn kind_spec(id: &str) -> Option<&'static KindSpec> {
    KINDS.iter().find(|k| k.id == id)
}

/// Look up one field's schema on a messenger type.
pub fn field_spec(kind: &str, field: &str) -> Option<&'static FieldSpec> {
    kind_spec(kind).and_then(|k| k.fields.iter().find(|f| f.name == field))
}

// ── Account naming ──────────────────────────────────────────────────────────

/// Longest account name accepted.
pub const MAX_ACCOUNT_NAME: usize = 64;

/// Check that an account name is usable as a display name and as part of a
/// vault key.
///
/// Names reach here from a client, and they end up in vault key strings, so
/// they are validated rather than sanitized silently: a name that quietly
/// becomes something else is a name the user cannot find again.
pub fn validate_account_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Account name cannot be empty".to_string());
    }
    if trimmed.len() > MAX_ACCOUNT_NAME {
        return Err(format!(
            "Account name is too long ({} characters; the limit is {MAX_ACCOUNT_NAME})",
            trimmed.len()
        ));
    }
    if let Some(bad) = trimmed
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.')))
    {
        return Err(format!(
            "Account name cannot contain '{bad}' — use letters, digits, spaces, '_', '-', or '.'"
        ));
    }
    if account_slug(trimmed).is_empty() {
        // A name of only punctuation slugs to nothing, which would file its
        // credentials under `messenger//<field>` — a key shared with every
        // other such name.
        return Err("Account name must contain at least one letter or digit".to_string());
    }
    Ok(())
}

/// Turn an account name into the form used inside vault keys.
///
/// Public because uniqueness has to be checked on the *slug*, not the display
/// name: the mapping is lossy — case, spaces, `_`, `-` and `.` all collapse —
/// so two names that look distinct can address one set of vault keys. See
/// [`slugs_collide`].
pub fn account_slug(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Vault credential name holding one field of one messenger account.
///
/// The `messenger/` prefix keeps these grouped in the vault listing and makes
/// it obvious what a stray entry belongs to.
pub fn secret_name(account: &str, field: &str) -> String {
    format!("messenger/{}/{}", account_slug(account), field)
}

/// Hex SHA-256 of a byte buffer — see [`MessengerProfile::avatar_sha256`].
///
/// Callers that go on to *use* the content should hash the bytes they hold
/// rather than calling [`file_fingerprint`] and reading the file again:
/// re-opening the path re-introduces the swap window the pin exists to
/// close.
pub fn bytes_fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex SHA-256 of a file's contents.
///
/// Used to pin a profile avatar to the bytes present when the user saved it
/// — see [`MessengerProfile::avatar_sha256`].
pub fn file_fingerprint(path: &std::path::Path) -> std::io::Result<String> {
    Ok(bytes_fingerprint(&std::fs::read(path)?))
}

/// Whether two account names would share one set of vault keys.
///
/// `"Telegram Main"`, `"telegram-main"` and `"telegram_main"` all slug to
/// `telegram-main`. Left unchecked, saving the second would overwrite the
/// first's token and deleting either would strand the other with a reference
/// to a credential that no longer exists.
pub fn slugs_collide(a: &str, b: &str) -> bool {
    account_slug(a) == account_slug(b)
}

// ── Profile ─────────────────────────────────────────────────────────────────

/// The identity the agent presents on a messenger.
///
/// Every field is optional: unset means "use the agent's own", which is
/// resolved by [`MessengerProfile::resolve`]. Storing the overrides rather
/// than a flattened copy means renaming the agent updates every messenger
/// that had not deliberately diverged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessengerProfile {
    /// Name shown to other people in the chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Short "about" or status text, where the backend has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    /// Avatar image to upload, where the backend supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_path: Option<std::path::PathBuf>,
    /// SHA-256 of the avatar's contents at the moment it was saved.
    ///
    /// The upload happens at connect time, arbitrarily later, and anything
    /// able to write the workspace (the agent's own file tools included)
    /// could swap the file in between. Pinning the bytes the user actually
    /// saved means a swap makes the upload refuse, not exfiltrate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_sha256: Option<String>,
    /// Which agent's identity this account speaks as. Defaults to `main`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// A profile with every fallback applied, ready to present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProfile {
    /// Name presented on the platform.
    pub display_name: String,
    /// Status/about text, when set anywhere.
    pub bio: Option<String>,
    /// Avatar image to upload, where the backend supports it.
    pub avatar_path: Option<std::path::PathBuf>,
    /// See [`MessengerProfile::avatar_sha256`].
    pub avatar_sha256: Option<String>,
    /// Agent whose identity this account speaks as.
    pub agent_id: String,
}

impl MessengerProfile {
    /// Fill in unset fields from the agent this account speaks as.
    ///
    /// `agent_name` and `agent_description` come from the agent's manifest —
    /// see [`crate::agents::AgentRegistry`].
    pub fn resolve(&self, agent_name: &str, agent_description: Option<&str>) -> ResolvedProfile {
        let non_empty = |s: &Option<String>| {
            s.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        ResolvedProfile {
            display_name: non_empty(&self.display_name).unwrap_or_else(|| agent_name.to_string()),
            bio: non_empty(&self.bio).or_else(|| {
                agent_description
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            }),
            avatar_path: self.avatar_path.clone(),
            avatar_sha256: self.avatar_sha256.clone(),
            agent_id: self
                .agent_id
                .clone()
                .unwrap_or_else(|| crate::agents::MAIN_AGENT_ID.to_string()),
        }
    }

    /// Whether anything is overridden at all.
    pub fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.bio.is_none()
            && self.avatar_path.is_none()
            && self.agent_id.is_none()
    }
}

// ── Thread routing ──────────────────────────────────────────────────────────

/// Binds a messenger channel to a gateway thread.
///
/// Without a route, each channel gets its own ad-hoc conversation keyed by
/// `"<type>:<channel>"` and shares nothing with the threads visible in a
/// client. A route says "this channel *is* thread N": the conversation is
/// keyed by the thread, and tools run in the thread's working directory. Two
/// channels pointed at the same thread share one conversation, which is how
/// you bridge a Matrix room and an IRC channel into a single discussion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRoute {
    /// Account name this route applies to, matching
    /// [`crate::config::MessengerConfig::name`].
    pub messenger: String,
    /// Channel, room, or chat id. `None` matches every channel on the
    /// account that no more specific route claims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Gateway thread that carries this conversation.
    pub thread_id: u64,
    /// Agent that owns the thread. Defaults to `main`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Whether the route is in effect.
    #[serde(default = "crate::config::default_true")]
    pub enabled: bool,
}

impl ThreadRoute {
    /// Whether this route applies to a message on `messenger` in `channel`.
    fn matches(&self, messenger: &str, channel: Option<&str>) -> bool {
        if !self.enabled || !self.messenger.eq_ignore_ascii_case(messenger) {
            return false;
        }
        match (&self.channel, channel) {
            (None, _) => true,
            (Some(want), Some(have)) => want.eq_ignore_ascii_case(have),
            (Some(_), None) => false,
        }
    }

    /// The agent whose thread list `thread_id` refers to.
    pub fn agent(&self) -> &str {
        self.agent_id
            .as_deref()
            .unwrap_or(crate::agents::MAIN_AGENT_ID)
    }

    /// Whether this route *is* the `(messenger, channel)` binding — the
    /// identity used when saving or deleting one.
    ///
    /// Case-insensitive on both parts because [`matches`](Self::matches) is:
    /// if identity compared exactly while matching folded case, `#Rust` and
    /// `#rust` could coexist as two routes for one channel, the first
    /// shadowing the second, and deleting one would leave the other.
    pub fn same_key(&self, messenger: &str, channel: Option<&str>) -> bool {
        self.messenger.eq_ignore_ascii_case(messenger)
            && match (&self.channel, channel) {
                (None, None) => true,
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                _ => false,
            }
    }
}

/// Find the route governing a message, preferring the most specific one.
///
/// A channel-specific route always wins over an account-wide catch-all, so
/// "everything from Telegram goes to thread 1, except this one group which
/// goes to thread 5" says what it looks like it says. Among equally specific
/// routes the first wins, which makes the config order meaningful and the
/// outcome stable rather than dependent on iteration order.
pub fn route_for<'a>(
    routes: &'a [ThreadRoute],
    messenger: &str,
    channel: Option<&str>,
) -> Option<&'a ThreadRoute> {
    let matching = || routes.iter().filter(|r| r.matches(messenger, channel));
    matching()
        .find(|r| r.channel.is_some())
        .or_else(|| matching().next())
}

// ── Plaintext credential migration ──────────────────────────────────────────

/// A credential sitting in `config.toml` that ought to be in the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaintextField {
    /// Field name, e.g. `token`.
    pub field: Cow<'static, str>,
    /// Label from the field schema.
    pub label: Cow<'static, str>,
}

/// Secret-valued fields this account still keeps in plaintext config.
///
/// Config predates the vault, so existing installs have live tokens sitting in
/// `config.toml`. Those keep working — silently breaking someone's running
/// messenger to make a point about hygiene would be worse — but they are
/// reported here so a client can offer to move them.
pub fn plaintext_fields(
    kind: &str,
    value_of: impl Fn(&str) -> Option<String>,
    has_secret_ref: impl Fn(&str) -> bool,
) -> Vec<PlaintextField> {
    let Some(spec) = kind_spec(kind) else {
        return Vec::new();
    };
    spec.secret_fields()
        .filter(|f| !has_secret_ref(&f.name))
        .filter(|f| value_of(&f.name).is_some_and(|v| !v.trim().is_empty()))
        .map(|f| PlaintextField {
            field: f.name.clone(),
            label: f.label.clone(),
        })
        .collect()
}

// ── Schema-driven field access ──────────────────────────────────────────────

/// Read and write [`crate::config::MessengerConfig`] fields by their schema
/// name, so a client can drive a form without knowing the struct.
///
/// The alternative is a match on field names at every call site — one in the
/// gateway handler, one per client — each free to forget a field. Here there
/// is one match, and [`tests::every_field_name_exists_on_the_config_struct`]
/// checks the schema against the struct.
impl crate::config::MessengerConfig {
    /// Current value of a schema field, rendered as text. Lists come back
    /// comma-separated; absent and empty are both `None`.
    pub fn field_value(&self, field: &str) -> Option<String> {
        let list = |v: &Vec<String>| (!v.is_empty()).then(|| v.join(", "));
        let text = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());
        match field {
            "token" => text(&self.token),
            "webhook_url" => text(&self.webhook_url),
            "homeserver" => text(&self.homeserver),
            "user_id" => text(&self.user_id),
            "password" => text(&self.password),
            "access_token" => text(&self.access_token),
            "phone" => text(&self.phone),
            "app_token" => text(&self.app_token),
            "default_channel" => text(&self.default_channel),
            "server" => text(&self.server),
            "port" => self.port.map(|p| p.to_string()),
            "nick" => text(&self.nick),
            "irc_channels" => list(&self.irc_channels),
            "use_tls" => self.use_tls.map(|b| b.to_string()),
            "credentials_path" => text(&self.credentials_path),
            "spaces" => list(&self.spaces),
            "app_id" => text(&self.app_id),
            "app_password" => text(&self.app_password),
            _ => None,
        }
    }

    /// Set a schema field from text. An empty value clears the field.
    ///
    /// Returns the parse failure rather than silently dropping a bad value: a
    /// port that quietly failed to apply is a connection failure nobody can
    /// explain later.
    pub fn set_field(&mut self, field: &str, value: &str) -> Result<(), String> {
        let trimmed = value.trim();
        let text = |v: &str| (!v.is_empty()).then(|| v.to_string());
        let list = |v: &str| {
            v.split([',', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        match field {
            "token" => self.token = text(trimmed),
            "webhook_url" => self.webhook_url = text(trimmed),
            "homeserver" => self.homeserver = text(trimmed),
            "user_id" => self.user_id = text(trimmed),
            "password" => self.password = text(trimmed),
            "access_token" => self.access_token = text(trimmed),
            "phone" => self.phone = text(trimmed),
            "app_token" => self.app_token = text(trimmed),
            "default_channel" => self.default_channel = text(trimmed),
            "server" => self.server = text(trimmed),
            "nick" => self.nick = text(trimmed),
            "credentials_path" => self.credentials_path = text(trimmed),
            "app_id" => self.app_id = text(trimmed),
            "app_password" => self.app_password = text(trimmed),
            "irc_channels" => self.irc_channels = list(trimmed),
            "spaces" => self.spaces = list(trimmed),
            "port" => {
                self.port = match trimmed.is_empty() {
                    true => None,
                    false => Some(
                        trimmed
                            .parse::<u16>()
                            .map_err(|_| format!("'{trimmed}' is not a port number (0-65535)"))?,
                    ),
                }
            }
            "use_tls" => {
                self.use_tls = match trimmed.to_ascii_lowercase().as_str() {
                    "" => None,
                    "true" | "yes" | "on" | "1" => Some(true),
                    "false" | "no" | "off" | "0" => Some(false),
                    other => return Err(format!("'{other}' is not a yes/no value")),
                }
            }
            other => return Err(format!("Unknown field: '{other}'")),
        }
        Ok(())
    }

    /// Vault credential name for a secret field, if one has been provisioned.
    pub fn secret_ref(&self, field: &str) -> Option<&str> {
        self.secret_refs.get(field).map(String::as_str)
    }

    /// Whether a field is filled, counting a vault reference as filled.
    ///
    /// This is what [`validate_fields`] should be handed when checking an
    /// account already on disk: the token is set, it is simply not here.
    pub fn field_is_set(&self, field: &str) -> bool {
        self.secret_refs.contains_key(field) || self.field_value(field).is_some()
    }

    /// Secret fields still stored in plaintext in `config.toml`.
    pub fn plaintext_credentials(&self) -> Vec<PlaintextField> {
        plaintext_fields(
            self.messenger_type.as_str(),
            |f| self.field_value(f),
            |f| self.secret_refs.contains_key(f),
        )
    }
}

// ── Validation ──────────────────────────────────────────────────────────────

/// Check a set of field values against a messenger type's schema.
///
/// `has_value` reports whether a field is filled, counting both a value in the
/// submitted form and a credential already in the vault — re-editing an
/// account without retyping its token must not read as "token missing".
pub fn validate_fields(kind: &str, has_value: impl Fn(&str) -> bool) -> Result<(), Vec<String>> {
    let Some(spec) = kind_spec(kind) else {
        return Err(vec![format!("Unknown messenger type: '{kind}'")]);
    };

    let mut errors = Vec::new();
    let mut groups: BTreeMap<&str, (bool, Vec<&str>)> = BTreeMap::new();

    for field in spec.fields.iter() {
        match &field.requirement {
            Requirement::Required if !has_value(&field.name) => {
                errors.push(format!("{} is required", field.label));
            }
            Requirement::OneOf(group) => {
                let entry = groups.entry(group.as_ref()).or_insert((false, Vec::new()));
                entry.0 |= has_value(&field.name);
                entry.1.push(field.label.as_ref());
            }
            _ => {}
        }
    }

    for (_, (satisfied, labels)) in groups {
        if !satisfied {
            errors.push(format!("One of {} is required", labels.join(", ")));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vault_key_is_derived_from_the_account_name() {
        assert_eq!(
            secret_name("Telegram Main", "token"),
            "messenger/telegram-main/token"
        );
        assert_eq!(
            secret_name("irc.libera", "password"),
            "messenger/irc-libera/password"
        );
    }

    #[test]
    fn names_that_differ_only_in_punctuation_are_recognised_as_colliding() {
        // All of these address messenger/telegram-main/<field>.
        for name in [
            "Telegram Main",
            "telegram-main",
            "telegram_main",
            "telegram.main",
        ] {
            assert!(
                slugs_collide("telegram-main", name),
                "{name} shares a vault key with telegram-main"
            );
        }
        assert!(!slugs_collide("telegram-main", "telegram-alt"));
    }

    #[test]
    fn a_name_with_no_alphanumerics_is_rejected() {
        // Would slug to "" and file credentials under messenger//<field>.
        assert!(validate_account_name("...").is_err());
        assert!(validate_account_name("---").is_err());
        assert!(validate_account_name("a").is_ok());
    }

    #[test]
    fn account_names_reject_what_would_mangle_a_vault_key() {
        assert!(validate_account_name("telegram-main").is_ok());
        assert!(validate_account_name("Work Slack 2").is_ok());
        assert!(validate_account_name("").is_err());
        assert!(validate_account_name("../etc/passwd").is_err());
        assert!(validate_account_name(&"x".repeat(MAX_ACCOUNT_NAME + 1)).is_err());
    }

    #[test]
    fn a_required_field_is_named_when_it_is_missing() {
        let err = validate_fields("telegram", |_| false).unwrap_err();
        assert_eq!(err, vec!["Bot token is required"]);
        assert!(validate_fields("telegram", |f| f == "token").is_ok());
    }

    #[test]
    fn either_arm_of_an_alternatives_group_satisfies_it() {
        let base = |f: &str| matches!(f, "homeserver" | "user_id");
        // Neither credential: the group is reported once, naming both.
        let err = validate_fields("matrix", base).unwrap_err();
        assert_eq!(err, vec!["One of Password, Access token is required"]);
        // Either one on its own is enough.
        assert!(validate_fields("matrix", |f| base(f) || f == "password").is_ok());
        assert!(validate_fields("matrix", |f| base(f) || f == "access_token").is_ok());
    }

    #[test]
    fn an_unknown_messenger_type_is_rejected_rather_than_accepted_blankly() {
        assert!(validate_fields("carrier-pigeon", |_| true).is_err());
    }

    #[test]
    fn a_channel_route_outranks_an_account_wide_one() {
        let routes = vec![
            ThreadRoute {
                messenger: "tg".into(),
                channel: None,
                thread_id: 1,
                agent_id: None,
                enabled: true,
            },
            ThreadRoute {
                messenger: "tg".into(),
                channel: Some("-100".into()),
                thread_id: 5,
                agent_id: None,
                enabled: true,
            },
        ];
        assert_eq!(route_for(&routes, "tg", Some("-100")).unwrap().thread_id, 5);
        assert_eq!(route_for(&routes, "tg", Some("-999")).unwrap().thread_id, 1);
        // A different account matches nothing, even for a catch-all route.
        assert!(route_for(&routes, "irc", Some("-100")).is_none());
    }

    #[test]
    fn a_disabled_route_does_not_match() {
        let routes = vec![ThreadRoute {
            messenger: "tg".into(),
            channel: None,
            thread_id: 1,
            agent_id: None,
            enabled: false,
        }];
        assert!(route_for(&routes, "tg", Some("c")).is_none());
    }

    #[test]
    fn an_unset_profile_falls_back_to_the_agent() {
        let resolved = MessengerProfile::default().resolve("Ada", Some("A helpful agent"));
        assert_eq!(resolved.display_name, "Ada");
        assert_eq!(resolved.bio.as_deref(), Some("A helpful agent"));
        assert_eq!(resolved.agent_id, crate::agents::MAIN_AGENT_ID);
    }

    #[test]
    fn a_blank_override_counts_as_unset_rather_than_as_an_empty_name() {
        let profile = MessengerProfile {
            display_name: Some("   ".into()),
            ..Default::default()
        };
        assert_eq!(profile.resolve("Ada", None).display_name, "Ada");
    }

    #[test]
    fn an_override_wins_over_the_agent_name() {
        let profile = MessengerProfile {
            display_name: Some("SupportBot".into()),
            bio: Some("Ask me about billing".into()),
            ..Default::default()
        };
        let resolved = profile.resolve("Ada", Some("A helpful agent"));
        assert_eq!(resolved.display_name, "SupportBot");
        assert_eq!(resolved.bio.as_deref(), Some("Ask me about billing"));
    }

    #[test]
    fn only_secret_fields_without_a_vault_ref_are_reported_as_plaintext() {
        let found = plaintext_fields(
            "slack",
            |f| match f {
                "token" => Some("xoxb-live".into()),
                "app_token" => Some("xapp-live".into()),
                _ => None,
            },
            |f| f == "app_token",
        );
        assert_eq!(found.len(), 1, "app_token is already vaulted: {found:?}");
        assert_eq!(found[0].field, "token");
    }

    #[test]
    fn an_empty_plaintext_value_is_not_a_credential_to_migrate() {
        let found = plaintext_fields("telegram", |_| Some("  ".into()), |_| false);
        assert!(found.is_empty());
    }

    #[test]
    fn every_kind_the_builders_accept_has_a_schema() {
        // The builder matches on these ids; a type it can build but that has
        // no schema is one the UI cannot configure.
        for id in [
            "telegram",
            "discord",
            "slack",
            "matrix",
            "irc",
            "signal",
            "whatsapp",
            "google_chat",
            "teams",
            "imessage",
            "webhook",
            "console",
        ] {
            assert!(kind_spec(id).is_some(), "no schema for '{id}'");
        }
    }

    #[test]
    fn every_field_name_exists_on_the_config_struct() {
        // A typo here is invisible until a user's credential silently fails to
        // reach the backend, so the schema is checked against the serialized
        // shape of MessengerConfig itself.
        let json = serde_json::to_value(crate::config::MessengerConfig::default()).unwrap();
        let object = json
            .as_object()
            .expect("MessengerConfig serializes as a map");
        for kind in KINDS {
            for field in kind.fields.iter() {
                assert!(
                    object.contains_key(field.name.as_ref()),
                    "{}.{} is not a MessengerConfig field",
                    kind.id,
                    field.name
                );
            }
        }
    }
}

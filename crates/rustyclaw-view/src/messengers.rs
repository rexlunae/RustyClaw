//! Component data for the messenger setup panel.
//!
//! The panel has three tabs — accounts, the account editor, and the routing
//! table — and this module owns all of the state and all of the derivations
//! that go with them. The clients render; they do not decide.
//!
//! # Why the logic lives here
//!
//! There are two clients rendering these forms and a dozen messenger backends
//! behind them. Anything a client works out for itself — which fields to show,
//! whether a credential is already stored, what the placeholder should say — is
//! something the other client can work out differently. So the form is derived
//! here, once, from [`rustyclaw_core::messengers::setup`], and a client's only
//! job is to draw the rows it is handed and report keystrokes back.

use rustyclaw_core::gateway::client_types::{
    MessengerAccountDto, RoutableThreadDto, ThreadRouteDto,
};
use rustyclaw_core::messengers::setup::{self, FieldKind, KindSpec, Requirement};

use crate::tone::Tone;

/// Which part of the panel is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MessengerTab {
    /// The configured accounts.
    #[default]
    Accounts,
    /// The channel-to-thread routing table.
    Routes,
}

impl MessengerTab {
    /// Tab label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Accounts => "Accounts",
            Self::Routes => "Routes",
        }
    }

    /// The other tab, for a toggle key.
    pub fn toggled(&self) -> Self {
        match self {
            Self::Accounts => Self::Routes,
            Self::Routes => Self::Accounts,
        }
    }
}

// ── Accounts ────────────────────────────────────────────────────────────────

/// One configured account, as the list shows it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessengerAccountData {
    /// Account name, unique within the config; what routes match on.
    pub name: String,
    /// Backend id, matching a `KindSpec` in the setup schema.
    pub messenger_type: String,
    /// Whether the gateway should connect this account.
    pub enabled: bool,
    /// Non-secret field values, in schema order.
    pub fields: Vec<(String, String)>,
    /// Secret fields with a credential in the vault.
    pub vaulted: Vec<String>,
    /// Secret fields still in plaintext config, as `(field, label)`.
    pub plaintext: Vec<(String, String)>,
    /// Name presented on the platform.
    pub display_name: String,
    /// Published description, if any.
    pub bio: Option<String>,
    /// Whether `display_name` is an override rather than the agent's name.
    pub display_name_overridden: bool,
    /// Whether `bio` is an override rather than the agent's description.
    pub bio_overridden: bool,
    /// Agent this account speaks as.
    pub agent_id: String,
    /// Whether this gateway build can run the backend.
    pub available: bool,
    /// Why not, when it cannot.
    pub unavailable_reason: Option<String>,
}

impl From<&MessengerAccountDto> for MessengerAccountData {
    fn from(dto: &MessengerAccountDto) -> Self {
        // Schema order, not map order: a form whose fields shuffle between
        // renders is a form nobody can fill in by muscle memory.
        let fields = match setup::kind_spec(&dto.messenger_type) {
            Some(spec) => spec
                .fields
                .iter()
                .filter(|f| !f.is_secret())
                .filter_map(|f| {
                    dto.fields
                        .get(f.name)
                        .map(|v| (f.name.to_string(), v.clone()))
                })
                .collect(),
            None => dto
                .fields
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        Self {
            name: dto.name.clone(),
            messenger_type: dto.messenger_type.clone(),
            enabled: dto.enabled,
            fields,
            vaulted: dto.vaulted.keys().cloned().collect(),
            plaintext: dto.plaintext.clone(),
            display_name: dto.profile.display_name.clone(),
            bio: dto.profile.bio.clone(),
            display_name_overridden: dto.profile.display_name_overridden,
            bio_overridden: dto.profile.bio_overridden,
            agent_id: dto.profile.agent_id.clone(),
            available: dto.available,
            unavailable_reason: dto.unavailable_reason.clone(),
        }
    }
}

impl MessengerAccountData {
    /// Schema for this account's backend.
    pub fn spec(&self) -> Option<&'static KindSpec> {
        setup::kind_spec(&self.messenger_type)
    }

    /// Icon for the backend.
    pub fn icon(&self) -> &'static str {
        self.spec().map_or("💬", |s| s.icon)
    }

    /// Human-readable backend name.
    pub fn type_label(&self) -> String {
        self.spec()
            .map(|s| s.label.to_string())
            .unwrap_or_else(|| self.messenger_type.clone())
    }

    /// Short status word for the list.
    pub fn status_label(&self) -> &'static str {
        if !self.available {
            "Unsupported"
        } else if !self.enabled {
            "Disabled"
        } else if !self.plaintext.is_empty() {
            "Plaintext"
        } else if self.vaulted.is_empty() && self.needs_credentials() {
            "No credentials"
        } else {
            "Ready"
        }
    }

    /// Tone matching [`status_label`](Self::status_label).
    pub fn status_tone(&self) -> Tone {
        match self.status_label() {
            "Ready" => Tone::Success,
            // Plaintext is a warning rather than an error: it works, it is
            // simply not where the credential should be.
            "Plaintext" => Tone::Warning,
            "Unsupported" | "No credentials" => Tone::Danger,
            _ => Tone::Neutral,
        }
    }

    /// Whether the backend takes any credential at all. Console and WhatsApp
    /// do not, so "no credentials" is not a fault for them.
    pub fn needs_credentials(&self) -> bool {
        self.spec()
            .is_some_and(|s| s.secret_fields().next().is_some())
    }

    /// Whether this account has credentials worth offering to migrate.
    pub fn has_plaintext(&self) -> bool {
        !self.plaintext.is_empty()
    }

    /// One-line summary of the plaintext warning.
    pub fn plaintext_warning(&self) -> Option<String> {
        (!self.plaintext.is_empty()).then(|| {
            let labels: Vec<&str> = self.plaintext.iter().map(|(_, l)| l.as_str()).collect();
            format!("{} stored in plaintext config", labels.join(", "))
        })
    }
}

// ── Account editor ──────────────────────────────────────────────────────────

/// One row in the account editor.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorField {
    /// Schema field name, or one of the synthetic names below.
    pub name: String,
    /// Label shown beside the input.
    pub label: String,
    /// How it should be entered.
    pub kind: FieldKind,
    /// Current buffer contents. Always empty for a secret with a stored value
    /// — see [`EditorField::placeholder`].
    pub value: String,
    /// Whether the field must be filled.
    pub required: bool,
    /// Alternatives group, when the field is one of several accepted.
    pub one_of: Option<String>,
    /// Help text from the schema.
    pub help: String,
    /// Whether the row currently counts as already having a credential.
    ///
    /// Derived, not owned: it is [`EditorField::had_stored`] AND the buffer
    /// being empty. Typing replaces the stored credential; emptying the box
    /// again goes back to keeping it.
    pub stored: bool,
    /// Whether a credential existed when the form was built.
    pub had_stored: bool,
}

impl EditorField {
    /// Recompute [`stored`](Self::stored) after the buffer changed.
    ///
    /// Clearing `stored` on the first keystroke and never restoring it meant
    /// that typing one character into a credential box and deleting it left
    /// the row empty *and* marked unset, so validation demanded a token the
    /// user had never intended to change.
    fn refresh_stored(&mut self) {
        self.stored = self.had_stored && self.value.trim().is_empty();
    }
}

impl EditorField {
    /// What to show when the buffer is empty.
    ///
    /// For a secret that is already stored this is the whole mechanism by
    /// which a user edits an account without retyping its token: the field
    /// reads as set, and leaving it blank keeps what the vault has.
    pub fn placeholder(&self) -> String {
        if self.stored {
            return "•••••••• (stored — leave blank to keep)".to_string();
        }
        match (&self.kind, self.required) {
            (FieldKind::Secret, _) => "required".to_string(),
            (FieldKind::Bool, _) => "yes / no".to_string(),
            (FieldKind::List, _) => "comma-separated".to_string(),
            (_, true) => "required".to_string(),
            (_, false) => "optional".to_string(),
        }
    }

    /// Whether typed characters should be masked.
    pub fn masked(&self) -> bool {
        self.kind.is_secret()
    }

    /// Whether this row carries a credential.
    pub fn is_secret(&self) -> bool {
        self.kind.is_secret()
    }
}

/// Synthetic editor field holding the account name.
pub const FIELD_ACCOUNT_NAME: &str = "@name";
/// Synthetic editor field holding the profile display-name override.
pub const FIELD_DISPLAY_NAME: &str = "@display_name";
/// Synthetic editor field holding the profile description override.
pub const FIELD_BIO: &str = "@bio";
/// Synthetic editor field holding the avatar path.
pub const FIELD_AVATAR: &str = "@avatar";

/// Whether a field name is one of the synthetic editor rows rather than a
/// schema field.
pub fn is_synthetic(field: &str) -> bool {
    field.starts_with('@')
}

/// State of the add/edit account form.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessengerEditorData {
    /// Account being edited, or `None` when creating a new one.
    pub editing: Option<String>,
    /// Chosen backend.
    pub messenger_type: String,
    /// Whether the account should be connected.
    pub enabled: bool,
    /// Rows, in display order.
    pub fields: Vec<EditorField>,
    /// Focused row.
    pub focused: usize,
    /// Validation problems from the last save attempt.
    pub errors: Vec<String>,
}

impl MessengerEditorData {
    /// Build the form for a new account of the given type.
    pub fn new(messenger_type: &str) -> Self {
        Self {
            editing: None,
            messenger_type: messenger_type.to_string(),
            enabled: true,
            fields: build_fields(messenger_type, None),
            focused: 0,
            errors: Vec::new(),
        }
    }

    /// Build the form for an existing account.
    pub fn edit(account: &MessengerAccountData) -> Self {
        Self {
            editing: Some(account.name.clone()),
            messenger_type: account.messenger_type.clone(),
            enabled: account.enabled,
            fields: build_fields(&account.messenger_type, Some(account)),
            focused: 0,
            errors: Vec::new(),
        }
    }

    /// Title for the form.
    pub fn title(&self) -> String {
        let label = setup::kind_spec(&self.messenger_type)
            .map(|s| s.label.to_string())
            .unwrap_or_else(|| self.messenger_type.clone());
        match &self.editing {
            Some(name) => format!("Edit {label} account “{name}”"),
            None => format!("New {label} account"),
        }
    }

    /// The focused row, if any.
    pub fn focused_field(&self) -> Option<&EditorField> {
        self.fields.get(self.focused)
    }

    /// Move focus down, wrapping.
    pub fn focus_next(&mut self) {
        if !self.fields.is_empty() {
            self.focused = (self.focused + 1) % self.fields.len();
        }
    }

    /// Move focus up, wrapping.
    pub fn focus_prev(&mut self) {
        if !self.fields.is_empty() {
            self.focused = match self.focused {
                0 => self.fields.len() - 1,
                n => n - 1,
            };
        }
    }

    /// Type a character into the focused row.
    pub fn insert(&mut self, c: char) {
        if let Some(field) = self.fields.get_mut(self.focused) {
            field.value.push(c);
            field.refresh_stored();
        }
    }

    /// Delete the character before the cursor in the focused row.
    pub fn backspace(&mut self) {
        if let Some(field) = self.fields.get_mut(self.focused) {
            field.value.pop();
            // Deleting back to empty returns the row to "keep what is stored".
            field.refresh_stored();
        }
    }

    /// Set a row's value outright (for clients with real text inputs).
    pub fn set_value(&mut self, name: &str, value: &str) {
        if let Some(field) = self.fields.iter_mut().find(|f| f.name == name) {
            field.value = value.to_string();
            field.refresh_stored();
        }
    }

    /// A row's current value.
    pub fn value_of(&self, name: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .map_or("", |f| f.value.as_str())
    }

    /// The account name as typed.
    pub fn account_name(&self) -> &str {
        self.value_of(FIELD_ACCOUNT_NAME).trim()
    }

    /// Check the form without contacting the gateway.
    ///
    /// The gateway validates again — it must, since a client is not a trust
    /// boundary — but catching it here means the user sees "Bot token is
    /// required" while still in the field rather than after a round trip.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if let Err(e) = setup::validate_account_name(self.account_name()) {
            errors.push(e);
        }
        let filled = |name: &str| {
            self.fields
                .iter()
                .find(|f| f.name == name)
                .is_some_and(|f| f.stored || !f.value.trim().is_empty())
        };
        if let Err(mut schema_errors) = setup::validate_fields(&self.messenger_type, filled) {
            errors.append(&mut schema_errors);
        }
        match errors.is_empty() {
            true => Ok(()),
            false => Err(errors),
        }
    }

    /// Non-secret field values to send, omitting untouched empties.
    pub fn field_values(&self) -> Vec<(String, String)> {
        self.fields
            .iter()
            .filter(|f| !f.is_secret() && !is_synthetic(&f.name))
            .map(|f| (f.name.clone(), f.value.trim().to_string()))
            .collect()
    }

    /// Credential values to send.
    ///
    /// A blank secret row is omitted entirely rather than sent as an empty
    /// string, which is what makes "leave blank to keep the stored one" work.
    pub fn secret_values(&self) -> Vec<(String, String)> {
        self.fields
            .iter()
            .filter(|f| f.is_secret() && !f.value.trim().is_empty())
            .map(|f| (f.name.clone(), f.value.clone()))
            .collect()
    }

    /// Profile override as `(display_name, bio, avatar)`, blank meaning
    /// "inherit from the agent".
    /// Profile override as `(display_name, bio, avatar)`.
    ///
    /// Name and description are always `Some`, blank included: the editor
    /// renders both rows, so a blank one is the user saying "inherit from the
    /// agent" and has to reach the gateway as an explicit clear. `None` is
    /// reserved for callers that are not editing the profile at all — an
    /// enable/disable toggle — where it means "leave it alone".
    ///
    /// The avatar is the exception. Its stored path is not sent to clients, so
    /// the row cannot be pre-filled and a blank one means "unchanged" rather
    /// than "remove". Removing an avatar is therefore not yet expressible in
    /// the editor, which is the lesser of the two wrongs.
    pub fn profile_values(&self) -> (Option<String>, Option<String>, Option<std::path::PathBuf>) {
        let avatar = self.value_of(FIELD_AVATAR).trim().to_string();
        (
            Some(self.value_of(FIELD_DISPLAY_NAME).trim().to_string()),
            Some(self.value_of(FIELD_BIO).trim().to_string()),
            (!avatar.is_empty()).then(|| std::path::PathBuf::from(avatar)),
        )
    }
}

/// Assemble the editor rows for a backend, pre-filled from an account.
fn build_fields(messenger_type: &str, account: Option<&MessengerAccountData>) -> Vec<EditorField> {
    let mut fields = vec![EditorField {
        name: FIELD_ACCOUNT_NAME.to_string(),
        label: "Account name".to_string(),
        kind: FieldKind::Text,
        value: account.map(|a| a.name.clone()).unwrap_or_default(),
        required: true,
        one_of: None,
        help: "How this account is listed, and how routes refer to it".to_string(),
        stored: false,
        had_stored: false,
    }];

    if let Some(spec) = setup::kind_spec(messenger_type) {
        for field in spec.fields {
            // A credential still sitting in plaintext config counts as stored
            // for the purpose of this form. It is not where it should be —
            // that is what the migrate action is for — but demanding the user
            // retype a working token before they can edit a channel list
            // would make the un-migrated account the harder one to look after.
            let stored = account.is_some_and(|a| {
                a.vaulted.iter().any(|v| v == field.name)
                    || a.plaintext.iter().any(|(f, _)| f == field.name)
            });
            let value = match field.is_secret() {
                // Never pre-fill a secret: the client was not sent the value,
                // and inventing something to show would be a lie about what
                // pressing Enter will save.
                true => String::new(),
                false => account
                    .and_then(|a| {
                        a.fields
                            .iter()
                            .find(|(name, _)| name == field.name)
                            .map(|(_, v)| v.clone())
                    })
                    .unwrap_or_default(),
            };
            fields.push(EditorField {
                name: field.name.to_string(),
                label: field.label.to_string(),
                kind: field.kind,
                value,
                required: matches!(field.requirement, Requirement::Required),
                one_of: match field.requirement {
                    Requirement::OneOf(group) => Some(group.to_string()),
                    _ => None,
                },
                help: field.help.to_string(),
                stored,
                had_stored: stored,
            });
        }
    }

    // Profile rows last: they are the same for every backend, and they are
    // optional, so they belong after the fields that decide whether the
    // account works at all.
    let overridden = |value: &Option<String>, is_override: bool| match is_override {
        true => value.clone().unwrap_or_default(),
        false => String::new(),
    };
    fields.push(EditorField {
        name: FIELD_DISPLAY_NAME.to_string(),
        label: "Display name".to_string(),
        kind: FieldKind::Text,
        value: account
            .map(|a| overridden(&Some(a.display_name.clone()), a.display_name_overridden))
            .unwrap_or_default(),
        required: false,
        one_of: None,
        help: "Name shown to others; blank inherits the agent's name".to_string(),
        stored: false,
        had_stored: false,
    });
    fields.push(EditorField {
        name: FIELD_BIO.to_string(),
        label: "Description".to_string(),
        kind: FieldKind::Text,
        value: account
            .map(|a| overridden(&a.bio, a.bio_overridden))
            .unwrap_or_default(),
        required: false,
        one_of: None,
        help: "Status/about text; blank inherits the agent's description".to_string(),
        stored: false,
        had_stored: false,
    });
    fields.push(EditorField {
        name: FIELD_AVATAR.to_string(),
        label: "Avatar path".to_string(),
        kind: FieldKind::Path,
        value: String::new(),
        required: false,
        one_of: None,
        help: "Image file to upload, where the platform supports one".to_string(),
        stored: false,
        had_stored: false,
    });

    fields
}

// ── Routes ──────────────────────────────────────────────────────────────────

/// One row of the routing table.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessengerRouteData {
    /// Account name the route applies to.
    pub messenger: String,
    /// `None` renders as "all channels".
    pub channel: Option<String>,
    /// Gateway thread the conversation is bound to.
    pub thread_id: u64,
    /// Agent whose thread list `thread_id` refers to.
    pub agent_id: String,
    /// Whether the route is in effect.
    pub enabled: bool,
    /// Thread label, or `None` when the thread is gone.
    pub thread_label: Option<String>,
}

impl From<&ThreadRouteDto> for MessengerRouteData {
    fn from(dto: &ThreadRouteDto) -> Self {
        Self {
            messenger: dto.messenger.clone(),
            channel: dto.channel.clone(),
            thread_id: dto.thread_id,
            agent_id: dto.agent_id.clone(),
            enabled: dto.enabled,
            thread_label: dto.thread_label.clone(),
        }
    }
}

impl MessengerRouteData {
    /// Left-hand side of the mapping.
    pub fn source_label(&self) -> String {
        match &self.channel {
            Some(channel) => format!("{} · {channel}", self.messenger),
            None => format!("{} · all channels", self.messenger),
        }
    }

    /// Right-hand side of the mapping.
    pub fn target_label(&self) -> String {
        match &self.thread_label {
            Some(label) => format!("#{} {label}", self.thread_id),
            // A dangling route is the one the user most needs to see, so it
            // says so rather than rendering as a bare number.
            None => format!("#{} (missing thread)", self.thread_id),
        }
    }

    /// Whether the route points at a thread that no longer exists.
    pub fn is_dangling(&self) -> bool {
        self.thread_label.is_none()
    }

    /// Tone for the row.
    pub fn tone(&self) -> Tone {
        if self.is_dangling() {
            Tone::Danger
        } else if !self.enabled {
            Tone::Neutral
        } else {
            Tone::Success
        }
    }
}

/// A thread a route may point at.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RoutableThreadData {
    /// Thread id within its agent's thread list.
    pub thread_id: u64,
    /// Thread label as the sidebar shows it.
    pub label: String,
    /// Agent that owns the thread.
    pub agent_id: String,
}

impl From<&RoutableThreadDto> for RoutableThreadData {
    fn from(dto: &RoutableThreadDto) -> Self {
        Self {
            thread_id: dto.thread_id,
            label: dto.label.clone(),
            agent_id: dto.agent_id.clone(),
        }
    }
}

impl RoutableThreadData {
    /// Label for a picker.
    pub fn label_for_picker(&self) -> String {
        format!("#{} {} ({})", self.thread_id, self.label, self.agent_id)
    }
}

/// The keyboard-driven "new route" form.
///
/// The desktop builds its route form from dropdowns; a terminal needs the
/// same choices as cyclable rows. Accounts and threads are chosen from what
/// the panel already knows — the only thing typed is the channel id.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RouteEditorData {
    /// Account names to choose from.
    pub accounts: Vec<String>,
    /// Chosen index into `accounts`.
    pub account_idx: usize,
    /// Channel id as typed; blank routes the whole account.
    pub channel: String,
    /// Chosen index into the panel's `threads`.
    pub thread_idx: usize,
    /// Focused row.
    pub focused: usize,
    /// Validation problems from the last submit attempt.
    pub errors: Vec<String>,
}

impl RouteEditorData {
    /// The account chooser row.
    pub const ROW_ACCOUNT: usize = 0;
    /// The typed channel-id row.
    pub const ROW_CHANNEL: usize = 1;
    /// The thread chooser row.
    pub const ROW_THREAD: usize = 2;
    const ROWS: usize = 3;

    /// Start a route for `preferred` when it names a known account, or the
    /// first account otherwise.
    pub fn new(accounts: Vec<String>, preferred: Option<&str>) -> Self {
        let account_idx = preferred
            .and_then(|name| accounts.iter().position(|a| a == name))
            .unwrap_or(0);
        Self {
            accounts,
            account_idx,
            ..Default::default()
        }
    }

    /// Move focus to the next row, wrapping.
    pub fn focus_next(&mut self) {
        self.focused = (self.focused + 1) % Self::ROWS;
    }

    /// Move focus to the previous row, wrapping.
    pub fn focus_prev(&mut self) {
        self.focused = (self.focused + Self::ROWS - 1) % Self::ROWS;
    }

    /// Step the focused chooser row by `delta`, wrapping. `thread_count`
    /// bounds the thread row; the channel row has nothing to cycle.
    pub fn cycle(&mut self, delta: isize, thread_count: usize) {
        let step = |idx: usize, len: usize| -> usize {
            match len {
                0 => 0,
                len => (idx as isize + delta).rem_euclid(len as isize) as usize,
            }
        };
        match self.focused {
            Self::ROW_ACCOUNT => self.account_idx = step(self.account_idx, self.accounts.len()),
            Self::ROW_THREAD => self.thread_idx = step(self.thread_idx, thread_count),
            _ => {}
        }
    }

    /// Type into the channel row; the chooser rows take no text.
    pub fn insert(&mut self, c: char) {
        if self.focused == Self::ROW_CHANNEL {
            self.channel.push(c);
        }
    }

    /// Delete the last typed character; as [`insert`](Self::insert), only
    /// the channel row has text to edit.
    pub fn backspace(&mut self) {
        if self.focused == Self::ROW_CHANNEL {
            self.channel.pop();
        }
    }

    /// The chosen account, when there is one to choose.
    pub fn account(&self) -> Option<&str> {
        self.accounts.get(self.account_idx).map(String::as_str)
    }

    /// The channel to send: blank means "the whole account".
    pub fn channel_value(&self) -> Option<String> {
        let trimmed = self.channel.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }
}

// ── Panel ───────────────────────────────────────────────────────────────────

/// Everything the messenger setup panel renders.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessengersPanelData {
    /// Which tab is showing.
    pub tab: MessengerTab,
    /// Configured accounts, in config order.
    pub accounts: Vec<MessengerAccountData>,
    /// Configured routes, in config order.
    pub routes: Vec<MessengerRouteData>,
    /// Threads a route may point at, across all agents.
    pub threads: Vec<RoutableThreadData>,
    /// Messenger types this gateway build can run.
    pub available_kinds: Vec<String>,
    /// Whether the vault is locked, so credential presence is unverifiable.
    pub vault_locked: bool,
    /// Selected row in the active tab.
    pub selected: Option<usize>,
    /// Open account editor, if any.
    pub editor: Option<MessengerEditorData>,
    /// Open "new route" form, if any (keyboard-driven clients).
    pub route_editor: Option<RouteEditorData>,
    /// Open backend picker, if any: the index into
    /// [`selectable_kinds`](Self::selectable_kinds).
    pub kind_picker: Option<usize>,
    /// Last message from the gateway.
    pub status: Option<String>,
    /// Bumped on every mutation the gateway accepted.
    ///
    /// A client that keeps its own editor state needs to know when a save
    /// actually landed, and cannot infer it from a refreshed account list: the
    /// gateway pushes one of those after failures too. Watching this counter
    /// change is how the desktop knows it is safe to discard a form.
    pub commits: u64,
    /// Whether `status` reports a failure.
    pub status_is_error: bool,
}

impl MessengersPanelData {
    /// Rows in the active tab.
    pub fn row_count(&self) -> usize {
        match self.tab {
            MessengerTab::Accounts => self.accounts.len(),
            MessengerTab::Routes => self.routes.len(),
        }
    }

    /// Selected account, when the accounts tab is showing.
    pub fn selected_account(&self) -> Option<&MessengerAccountData> {
        match self.tab {
            MessengerTab::Accounts => self.selected.and_then(|i| self.accounts.get(i)),
            MessengerTab::Routes => None,
        }
    }

    /// Selected route, when the routes tab is showing.
    pub fn selected_route(&self) -> Option<&MessengerRouteData> {
        match self.tab {
            MessengerTab::Routes => self.selected.and_then(|i| self.routes.get(i)),
            MessengerTab::Accounts => None,
        }
    }

    /// Backends offered in the "new account" picker.
    ///
    /// Types this build cannot run are excluded: offering Matrix on a binary
    /// without the feature produces an account that can never connect.
    pub fn selectable_kinds(&self) -> Vec<&'static KindSpec> {
        setup::KINDS
            .iter()
            .filter(|k| self.available_kinds.iter().any(|id| id == k.id))
            .collect()
    }

    /// Move the selection down, wrapping.
    pub fn select_next(&mut self) {
        let count = self.row_count();
        if count == 0 {
            self.selected = None;
            return;
        }
        self.selected = Some(match self.selected {
            Some(i) if i + 1 < count => i + 1,
            _ => 0,
        });
    }

    /// Move the selection up, wrapping.
    pub fn select_prev(&mut self) {
        let count = self.row_count();
        if count == 0 {
            self.selected = None;
            return;
        }
        self.selected = Some(match self.selected {
            Some(0) | None => count - 1,
            Some(i) => i - 1,
        });
    }

    /// Switch tabs, clamping the selection to the new tab's length.
    pub fn toggle_tab(&mut self) {
        self.tab = self.tab.toggled();
        self.selected = match self.row_count() {
            0 => None,
            n => Some(self.selected.unwrap_or(0).min(n - 1)),
        };
    }

    /// Replace the gateway-owned contents, keeping the selection in range.
    ///
    /// Refreshes arrive unsolicited after every mutation, so this must not
    /// reset the cursor: a user who just toggled the third account expects to
    /// still be looking at the third account.
    pub fn apply(
        &mut self,
        accounts: Vec<MessengerAccountData>,
        routes: Vec<MessengerRouteData>,
        threads: Vec<RoutableThreadData>,
        available_kinds: Vec<String>,
        vault_locked: bool,
    ) {
        self.vault_locked = vault_locked;
        self.accounts = accounts;
        self.routes = routes;
        self.threads = threads;
        self.available_kinds = available_kinds;
        self.selected = match self.row_count() {
            0 => None,
            n => Some(self.selected.unwrap_or(0).min(n - 1)),
        };
        // A refresh landing mid-edit keeps the route form open (matching the
        // account editor), but its thread choice must stay in range of the
        // list it indexes into.
        if let Some(editor) = &mut self.route_editor {
            editor.thread_idx = editor.thread_idx.min(self.threads.len().saturating_sub(1));
        }
    }

    /// Record an outcome for display.
    pub fn set_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.status = Some(message.into());
        self.status_is_error = is_error;
    }

    /// Tone for the status line.
    pub fn status_tone(&self) -> Tone {
        match self.status_is_error {
            true => Tone::Danger,
            false => Tone::Success,
        }
    }

    /// Summary line for the accounts tab.
    pub fn summary(&self) -> String {
        let ready = self
            .accounts
            .iter()
            .filter(|a| a.status_label() == "Ready")
            .count();
        let plaintext = self.accounts.iter().filter(|a| a.has_plaintext()).count();
        let mut summary = format!("{ready} ready / {} configured", self.accounts.len());
        if self.vault_locked {
            // Credential presence is taken from config here, not confirmed
            // against the vault. Saying so beats quietly implying otherwise.
            summary.push_str(" · vault locked, credentials unverified");
        }
        if plaintext > 0 {
            summary.push_str(&format!(
                " · {plaintext} with plaintext credential{}",
                if plaintext == 1 { "" } else { "s" }
            ));
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(messenger_type: &str) -> MessengerAccountData {
        MessengerAccountData {
            name: "acct".into(),
            messenger_type: messenger_type.into(),
            enabled: true,
            available: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_stored_secret_is_offered_as_keepable_rather_than_blank() {
        let mut telegram = account("telegram");
        telegram.vaulted = vec!["token".into()];
        let editor = MessengerEditorData::edit(&telegram);
        let token = editor.fields.iter().find(|f| f.name == "token").unwrap();
        assert!(token.stored);
        assert!(token.value.is_empty(), "the value was never sent to us");
        assert!(token.placeholder().contains("leave blank"));
        // A stored credential satisfies the requirement without retyping.
        assert!(editor.validate().is_ok(), "{:?}", editor.validate());
    }

    #[test]
    fn a_plaintext_credential_counts_as_present_when_editing() {
        let mut telegram = account("telegram");
        telegram.plaintext = vec![("token".into(), "Bot token".into())];
        let editor = MessengerEditorData::edit(&telegram);
        let token = editor.fields.iter().find(|f| f.name == "token").unwrap();
        assert!(
            token.stored,
            "an un-migrated account must still be editable without retyping its token"
        );
        // The gateway accepts this save; the form must not be the thing that
        // blocks it.
        assert!(editor.validate().is_ok(), "{:?}", editor.validate());
        assert!(editor.secret_values().is_empty());
    }

    #[test]
    fn a_blank_profile_row_is_sent_as_an_explicit_clear() {
        let mut telegram = account("telegram");
        telegram.display_name = "SupportBot".into();
        telegram.display_name_overridden = true;
        let mut editor = MessengerEditorData::edit(&telegram);
        editor.set_value(FIELD_DISPLAY_NAME, "");

        let (display_name, bio, avatar) = editor.profile_values();
        // Some("") rather than None: None means "not editing the profile",
        // which is what a toggle sends, and would keep the old name.
        assert_eq!(display_name.as_deref(), Some(""));
        assert_eq!(bio.as_deref(), Some(""));
        // The avatar row cannot be pre-filled, so blank means "unchanged".
        assert_eq!(avatar, None);
    }

    #[test]
    fn a_filled_profile_row_is_sent_as_typed() {
        let mut editor = MessengerEditorData::new("console");
        editor.set_value(FIELD_ACCOUNT_NAME, "c");
        editor.set_value(FIELD_DISPLAY_NAME, "Ada");
        editor.set_value(FIELD_AVATAR, "/tmp/face.png");
        let (display_name, _, avatar) = editor.profile_values();
        assert_eq!(display_name.as_deref(), Some("Ada"));
        assert_eq!(avatar, Some(std::path::PathBuf::from("/tmp/face.png")));
    }

    #[test]
    fn emptying_a_credential_box_returns_it_to_keeping_the_stored_one() {
        let mut telegram = account("telegram");
        telegram.vaulted = vec!["token".into()];
        let mut editor = MessengerEditorData::edit(&telegram);
        editor.focused = editor
            .fields
            .iter()
            .position(|f| f.name == "token")
            .unwrap();

        editor.insert('x');
        assert!(!editor.fields[editor.focused].stored, "typing replaces it");
        editor.backspace();
        assert!(
            editor.fields[editor.focused].stored,
            "deleting back to empty must mean 'keep the stored one' again"
        );
        assert!(editor.validate().is_ok(), "{:?}", editor.validate());
        assert!(editor.secret_values().is_empty());

        // Same via the desktop's set_value path.
        editor.set_value("token", "y");
        assert!(!editor.fields[editor.focused].stored);
        editor.set_value("token", "");
        assert!(editor.fields[editor.focused].stored);
    }

    #[test]
    fn a_blank_secret_row_is_not_sent_so_the_stored_one_survives() {
        let mut telegram = account("telegram");
        telegram.vaulted = vec!["token".into()];
        let editor = MessengerEditorData::edit(&telegram);
        assert!(editor.secret_values().is_empty());
    }

    #[test]
    fn typing_over_a_stored_secret_stops_claiming_it_is_kept() {
        let mut telegram = account("telegram");
        telegram.vaulted = vec!["token".into()];
        let mut editor = MessengerEditorData::edit(&telegram);
        editor.focused = editor
            .fields
            .iter()
            .position(|f| f.name == "token")
            .unwrap();
        editor.insert('x');
        let token = editor.fields.iter().find(|f| f.name == "token").unwrap();
        assert!(!token.stored);
        assert_eq!(editor.secret_values(), vec![("token".into(), "x".into())]);
    }

    #[test]
    fn a_new_account_without_its_credential_fails_validation_locally() {
        let mut editor = MessengerEditorData::new("telegram");
        editor.set_value(FIELD_ACCOUNT_NAME, "tg");
        let errors = editor.validate().unwrap_err();
        assert_eq!(errors, vec!["Bot token is required"]);
        editor.set_value("token", "123:abc");
        assert!(editor.validate().is_ok());
    }

    #[test]
    fn a_route_form_cycles_choosers_and_types_only_the_channel() {
        let mut form = RouteEditorData::new(
            vec!["tg-main".into(), "tg-support".into()],
            Some("tg-support"),
        );
        assert_eq!(form.account(), Some("tg-support"), "preferred wins");

        // The account row cycles and wraps; typing into it does nothing.
        form.cycle(1, 3);
        assert_eq!(form.account(), Some("tg-main"));
        form.insert('x');
        assert_eq!(form.channel, "", "chooser rows take no text");

        // The channel row types; cycling it does nothing.
        form.focus_next();
        form.insert('#');
        form.insert('a');
        form.cycle(1, 3);
        assert_eq!(form.channel, "#a");
        form.backspace();
        assert_eq!(form.channel, "#");

        // The thread row cycles within the count it is given.
        form.focus_next();
        form.cycle(-1, 3);
        assert_eq!(form.thread_idx, 2, "cycling backwards wraps");
        form.cycle(1, 0);
        assert_eq!(form.thread_idx, 0, "an empty thread list cannot underflow");
    }

    #[test]
    fn a_blank_channel_routes_the_whole_account() {
        let mut form = RouteEditorData::new(vec!["irc".into()], None);
        assert_eq!(form.channel_value(), None);
        form.focus_next();
        form.insert(' ');
        assert_eq!(form.channel_value(), None, "whitespace is not a channel");
        form.insert('#');
        form.insert('r');
        assert_eq!(form.channel_value().as_deref(), Some("#r"));
    }

    #[test]
    fn an_unnamed_account_is_rejected_before_the_round_trip() {
        let editor = MessengerEditorData::new("console");
        assert!(
            editor
                .validate()
                .unwrap_err()
                .iter()
                .any(|e| e.contains("empty"))
        );
    }

    #[test]
    fn secrets_never_travel_in_the_non_secret_field_list() {
        let mut editor = MessengerEditorData::new("slack");
        editor.set_value(FIELD_ACCOUNT_NAME, "work");
        editor.set_value("token", "xoxb-live");
        editor.set_value("default_channel", "#general");
        let fields = editor.field_values();
        assert!(
            !fields.iter().any(|(name, _)| name == "token"),
            "a credential reached the plaintext field list: {fields:?}"
        );
        assert!(
            fields
                .iter()
                .any(|(n, v)| n == "default_channel" && v == "#general")
        );
        assert_eq!(
            editor.secret_values(),
            vec![("token".into(), "xoxb-live".into())]
        );
    }

    #[test]
    fn synthetic_rows_are_not_sent_as_config_fields() {
        let mut editor = MessengerEditorData::new("console");
        editor.set_value(FIELD_ACCOUNT_NAME, "c");
        editor.set_value(FIELD_DISPLAY_NAME, "Ada");
        let fields = editor.field_values();
        assert!(
            fields.iter().all(|(name, _)| !is_synthetic(name)),
            "{fields:?}"
        );
        assert_eq!(editor.profile_values().0.as_deref(), Some("Ada"));
    }

    #[test]
    fn an_inherited_profile_leaves_the_override_row_blank() {
        let mut telegram = account("telegram");
        telegram.display_name = "Ada".into();
        telegram.display_name_overridden = false;
        let editor = MessengerEditorData::edit(&telegram);
        assert_eq!(
            editor.value_of(FIELD_DISPLAY_NAME),
            "",
            "an inherited name pre-filled as an override would silently pin it"
        );
    }

    #[test]
    fn an_explicit_profile_override_is_shown_for_editing() {
        let mut telegram = account("telegram");
        telegram.display_name = "SupportBot".into();
        telegram.display_name_overridden = true;
        let editor = MessengerEditorData::edit(&telegram);
        assert_eq!(editor.value_of(FIELD_DISPLAY_NAME), "SupportBot");
    }

    #[test]
    fn plaintext_credentials_read_as_a_warning_not_a_failure() {
        let mut telegram = account("telegram");
        telegram.plaintext = vec![("token".into(), "Bot token".into())];
        assert_eq!(telegram.status_label(), "Plaintext");
        assert_eq!(telegram.status_tone(), Tone::Warning);
        assert!(telegram.plaintext_warning().unwrap().contains("Bot token"));
    }

    #[test]
    fn a_backend_that_needs_no_credential_is_not_faulted_for_lacking_one() {
        assert_eq!(account("console").status_label(), "Ready");
        assert_eq!(account("telegram").status_label(), "No credentials");
    }

    #[test]
    fn a_route_to_a_deleted_thread_says_so() {
        let route = MessengerRouteData {
            messenger: "tg".into(),
            channel: Some("-100".into()),
            thread_id: 7,
            enabled: true,
            thread_label: None,
            ..Default::default()
        };
        assert!(route.is_dangling());
        assert!(route.target_label().contains("missing"));
        assert_eq!(route.tone(), Tone::Danger);
        assert_eq!(route.source_label(), "tg · -100");
    }

    #[test]
    fn a_catch_all_route_says_all_channels() {
        let route = MessengerRouteData {
            messenger: "tg".into(),
            channel: None,
            thread_id: 1,
            enabled: true,
            thread_label: Some("Main".into()),
            ..Default::default()
        };
        assert_eq!(route.source_label(), "tg · all channels");
        assert_eq!(route.target_label(), "#1 Main");
    }

    #[test]
    fn the_selection_survives_a_refresh() {
        let mut panel = MessengersPanelData::default();
        panel.apply(
            vec![account("telegram"), account("slack"), account("irc")],
            vec![],
            vec![],
            vec![],
            false,
        );
        panel.selected = Some(2);
        // A refresh that dropped the cursor would move the user's selection
        // out from under them mid-edit.
        panel.apply(
            vec![account("telegram"), account("slack"), account("irc")],
            vec![],
            vec![],
            vec![],
            false,
        );
        assert_eq!(panel.selected, Some(2));
    }

    #[test]
    fn a_shrinking_list_clamps_rather_than_dangles() {
        let mut panel = MessengersPanelData::default();
        panel.apply(
            vec![account("telegram"), account("slack")],
            vec![],
            vec![],
            vec![],
            false,
        );
        panel.selected = Some(1);
        panel.apply(vec![account("telegram")], vec![], vec![], vec![], false);
        assert_eq!(panel.selected, Some(0));
        panel.apply(vec![], vec![], vec![], vec![], false);
        assert_eq!(panel.selected, None);
    }

    #[test]
    fn switching_tabs_clamps_the_selection_to_the_new_list() {
        let mut panel = MessengersPanelData::default();
        panel.apply(
            vec![account("telegram"), account("slack"), account("irc")],
            vec![MessengerRouteData::default()],
            vec![],
            vec![],
            false,
        );
        panel.selected = Some(2);
        panel.toggle_tab();
        assert_eq!(panel.tab, MessengerTab::Routes);
        assert_eq!(panel.selected, Some(0), "routes has only one row");
    }

    #[test]
    fn only_kinds_this_build_supports_are_offered() {
        let panel = MessengersPanelData {
            available_kinds: vec!["telegram".into(), "irc".into()],
            ..Default::default()
        };
        let ids: Vec<&str> = panel.selectable_kinds().iter().map(|k| k.id).collect();
        assert_eq!(ids, vec!["telegram", "irc"]);
        assert!(!ids.contains(&"matrix"), "matrix is not compiled in here");
    }

    #[test]
    fn the_summary_counts_plaintext_accounts_separately() {
        let mut plain = account("telegram");
        plain.plaintext = vec![("token".into(), "Bot token".into())];
        let mut ready = account("telegram");
        ready.vaulted = vec!["token".into()];
        let panel = MessengersPanelData {
            accounts: vec![plain, ready],
            ..Default::default()
        };
        let summary = panel.summary();
        assert!(summary.starts_with("1 ready / 2 configured"), "{summary}");
        assert!(summary.contains("1 with plaintext credential"), "{summary}");
    }

    #[test]
    fn fields_appear_in_schema_order_not_map_order() {
        use rustyclaw_core::gateway::client_types::{MessengerAccountDto, MessengerProfileDto};
        use std::collections::BTreeMap;

        // BTreeMap sorts alphabetically: default_channel would come first.
        let mut fields = BTreeMap::new();
        fields.insert("default_channel".to_string(), "#general".to_string());
        let dto = MessengerAccountDto {
            name: "work".into(),
            messenger_type: "slack".into(),
            enabled: true,
            fields,
            vaulted: BTreeMap::new(),
            plaintext: vec![],
            profile: MessengerProfileDto::default(),
            available: true,
            unavailable_reason: None,
        };
        let data = MessengerAccountData::from(&dto);
        assert_eq!(
            data.fields,
            vec![("default_channel".into(), "#general".into())]
        );

        // And the editor lays out every schema field in the declared order.
        let editor = MessengerEditorData::edit(&data);
        let names: Vec<&str> = editor.fields.iter().map(|f| f.name.as_str()).collect();
        let token_at = names.iter().position(|n| *n == "token").unwrap();
        let channel_at = names.iter().position(|n| *n == "default_channel").unwrap();
        assert!(token_at < channel_at, "{names:?}");
    }
}

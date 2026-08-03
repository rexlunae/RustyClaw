//! Messenger setup dialog — accounts, presented profile, and thread routing.
//!
//! Two tabs plus an editor, all driven by [`MessengersPanelData`] from
//! `rustyclaw-view`. Which fields a backend has, whether a credential is
//! already stored, and what the placeholder should say are all decided there,
//! so this component and the TUI's dialog show the same form for the same
//! account without either one knowing what Telegram needs.
//!
//! Credential inputs are `type="password"` and start empty even when a
//! credential exists — the gateway does not send values back, so there is
//! nothing to pre-fill and nothing to leak.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button, Buttons, Table, Tag};
use rustyclaw_view::messengers::{
    MessengerAccountData, MessengerEditorData, MessengerTab, MessengersPanelData,
};

use super::{RcModal, tone_color, tone_modifier};

/// Commands the messenger dialog sends back to the parent.
#[derive(Clone, Debug, PartialEq)]
pub enum MessengerCommand {
    /// Re-fetch accounts, routes, and threads.
    Refresh,
    /// Create or update an account.
    SaveAccount {
        original_name: Option<String>,
        name: String,
        messenger_type: String,
        enabled: bool,
        fields: Vec<(String, String)>,
        secrets: Vec<(String, String)>,
        display_name: Option<String>,
        bio: Option<String>,
        avatar_path: Option<std::path::PathBuf>,
    },
    /// Delete an account, its credentials, and its routes.
    DeleteAccount { name: String },
    /// Move an account's plaintext credentials into the vault.
    MigrateSecrets { name: String },
    /// Create or update a route.
    SaveRoute {
        messenger: String,
        channel: Option<String>,
        thread_id: u64,
        agent_id: Option<String>,
        enabled: bool,
    },
    /// Delete a route.
    DeleteRoute {
        messenger: String,
        channel: Option<String>,
    },
}

#[derive(Props, Clone, PartialEq)]
pub struct MessengersDialogProps {
    pub visible: bool,
    /// Canonical data from the gateway.
    pub data: MessengersPanelData,
    /// Called when the dialog needs the parent to talk to the gateway.
    pub on_command: EventHandler<MessengerCommand>,
    /// Called when the dialog should be dismissed.
    pub on_close: EventHandler<()>,
}

#[component]
pub fn MessengersDialog(props: MessengersDialogProps) -> Element {
    // Tab and editor are desktop-local: the shared model's `tab`/`editor` are
    // the TUI's keyboard-driven state machine, and a mouse client navigates by
    // clicking rather than by cycling.
    let mut tab = use_signal(|| MessengerTab::Accounts);
    let mut editor: Signal<Option<MessengerEditorData>> = use_signal(|| None);
    let mut picking_kind = use_signal(|| false);
    // Highest commit count this dialog has already reacted to. The gateway
    // pushes a refreshed view after failed mutations too, so the account list
    // changing is not evidence that a save landed — the counter is.
    let mut seen_commits = use_signal(|| 0u64);
    let route_channel = use_signal(String::new);
    let route_thread = use_signal(String::new);
    let route_account = use_signal(String::new);

    if !props.visible {
        return rsx! {};
    }

    let data = props.data.clone();

    // Close the editor once — and only once — a save is confirmed.
    if data.commits > *seen_commits.read() {
        seen_commits.set(data.commits);
        editor.set(None);
        picking_kind.set(false);
    }

    rsx! {
        RcModal {
            active: true,
            title: "💬 Messengers",
            width: 860,
            class: "messengers-dialog",
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| props.on_command.call(MessengerCommand::Refresh),
                        "↻ Refresh"
                    }
                }
            },

            div { class: "tabs is-boxed",
                ul {
                    li {
                        class: if *tab.read() == MessengerTab::Accounts { "is-active" } else { "" },
                        a { onclick: move |_| tab.set(MessengerTab::Accounts), "Accounts" }
                    }
                    li {
                        class: if *tab.read() == MessengerTab::Routes { "is-active" } else { "" },
                        a { onclick: move |_| tab.set(MessengerTab::Routes), "Thread routing" }
                    }
                }
            }

            if let Some(status) = data.status.clone() {
                div {
                    class: "notification is-light {tone_modifier(data.status_tone())}",
                    "{status}"
                }
            }

            // ── Account editor ──
            if let Some(form) = editor.read().clone() {
                AccountEditor {
                    form: form.clone(),
                    server_error: data.status_is_error.then(|| data.status.clone()).flatten(),
                    on_change: move |updated| editor.set(Some(updated)),
                    on_cancel: move |_| editor.set(None),
                    on_save: move |form: MessengerEditorData| {
                        match form.validate() {
                            Ok(()) => {
                                let (display_name, bio, avatar_path) = form.profile_values();
                                props.on_command.call(MessengerCommand::SaveAccount {
                                    original_name: form.editing.clone(),
                                    name: form.account_name().to_string(),
                                    messenger_type: form.messenger_type.clone(),
                                    enabled: form.enabled,
                                    fields: form.field_values(),
                                    secrets: form.secret_values(),
                                    display_name,
                                    bio,
                                    avatar_path,
                                });
                                // The form stays mounted until the gateway
                                // confirms. Several rejections are only
                                // detectable server-side — a colliding name,
                                // an unsupported backend, a vault write that
                                // failed — and closing here would throw away
                                // everything typed, including a credential the
                                // gateway never echoes back and so cannot
                                // restore.
                            }
                            Err(errors) => {
                                let mut rejected = form.clone();
                                rejected.errors = errors;
                                editor.set(Some(rejected));
                            }
                        }
                    },
                }
            } else if *picking_kind.read() {
                // ── Backend picker ──
                div {
                    p { class: "mb-3", "Which kind of messenger?" }
                    if data.selectable_kinds().is_empty() {
                        p { class: "has-text-warning",
                            "This gateway build has no messenger backends compiled in."
                        }
                    }
                    for kind in data.selectable_kinds() {
                        div { class: "box is-clickable mb-2",
                            key: "{kind.id}",
                            onclick: move |_| {
                                editor.set(Some(MessengerEditorData::new(kind.id)));
                                picking_kind.set(false);
                            },
                            strong { "{kind.icon} {kind.label}" }
                            p { class: "is-size-7 has-text-grey", "{kind.summary}" }
                        }
                    }
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| picking_kind.set(false),
                        "Cancel"
                    }
                }
            } else if *tab.read() == MessengerTab::Accounts {
                AccountList {
                    data: data.clone(),
                    on_new: move |_| picking_kind.set(true),
                    on_edit: move |account: MessengerAccountData| {
                        editor.set(Some(MessengerEditorData::edit(&account)));
                    },
                    on_command: props.on_command,
                }
            } else {
                RouteTable {
                    data: data.clone(),
                    account: route_account,
                    channel: route_channel,
                    thread: route_thread,
                    on_command: props.on_command,
                }
            }
        }
    }
}

/// The accounts tab.
#[component]
fn AccountList(
    data: MessengersPanelData,
    on_new: EventHandler<()>,
    on_edit: EventHandler<MessengerAccountData>,
    on_command: EventHandler<MessengerCommand>,
) -> Element {
    rsx! {
        div {
            p { class: "is-size-7 has-text-grey mb-2", "{data.summary()}" }

            Table { striped: true, hoverable: true, fullwidth: true,
                thead {
                    tr {
                        th { "Account" }
                        th { "Type" }
                        th { "Presenting as" }
                        th { "Status" }
                        th { "" }
                    }
                }
                tbody {
                    for account in data.accounts.clone() {
                        tr { key: "{account.name}",
                            td { "{account.icon()} {account.name}" }
                            td { "{account.type_label()}" }
                            td {
                                "{account.display_name}"
                                if !account.display_name_overridden {
                                    span { class: "is-size-7 has-text-grey", " (inherited)" }
                                }
                            }
                            td {
                                Tag {
                                    color: tone_color(account.status_tone()),
                                    "{account.status_label()}"
                                }
                                if let Some(warning) = account.plaintext_warning() {
                                    p { class: "is-size-7 has-text-warning", "{warning}" }
                                }
                                if let Some(reason) = account.unavailable_reason.clone() {
                                    p { class: "is-size-7 has-text-danger", "{reason}" }
                                }
                            }
                            td {
                                Buttons {
                                    Button {
                                        size: BulmaSize::Small,
                                        onclick: {
                                            let account = account.clone();
                                            move |_| on_edit.call(account.clone())
                                        },
                                        "Edit"
                                    }
                                    Button {
                                        size: BulmaSize::Small,
                                        onclick: {
                                            let account = account.clone();
                                            move |_| on_command.call(MessengerCommand::SaveAccount {
                                                original_name: Some(account.name.clone()),
                                                name: account.name.clone(),
                                                messenger_type: account.messenger_type.clone(),
                                                enabled: !account.enabled,
                                                // Nothing else is sent, so the
                                                // gateway edits the stored
                                                // entry rather than replacing
                                                // it with a blank one.
                                                fields: Vec::new(),
                                                secrets: Vec::new(),
                                                display_name: None,
                                                bio: None,
                                                avatar_path: None,
                                            })
                                        },
                                        if account.enabled { "Disable" } else { "Enable" }
                                    }
                                    if account.has_plaintext() {
                                        Button {
                                            size: BulmaSize::Small,
                                            color: BulmaColor::Warning,
                                            onclick: {
                                                let name = account.name.clone();
                                                move |_| on_command.call(
                                                    MessengerCommand::MigrateSecrets { name: name.clone() }
                                                )
                                            },
                                            "Move to vault"
                                        }
                                    }
                                    Button {
                                        size: BulmaSize::Small,
                                        color: BulmaColor::Danger,
                                        onclick: {
                                            let name = account.name.clone();
                                            move |_| on_command.call(
                                                MessengerCommand::DeleteAccount { name: name.clone() }
                                            )
                                        },
                                        "Delete"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Button {
                color: BulmaColor::Primary,
                onclick: move |_| on_new.call(()),
                "Add account"
            }
        }
    }
}

/// The account add/edit form.
#[component]
fn AccountEditor(
    form: MessengerEditorData,
    /// The gateway's complaint about the last save, if it refused one.
    server_error: Option<String>,
    on_change: EventHandler<MessengerEditorData>,
    on_save: EventHandler<MessengerEditorData>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        div {
            h4 { class: "title is-6", "{form.title()}" }

            for field in form.fields.clone() {
                div { class: "field", key: "{field.name}",
                    label { class: "label is-small",
                        "{field.label}"
                        if field.required {
                            span { class: "has-text-danger", " *" }
                        }
                        if field.one_of.is_some() {
                            span { class: "has-text-grey", " (one of these)" }
                        }
                    }
                    div { class: "control",
                        input {
                            class: "input is-small",
                            r#type: if field.masked() { "password" } else { "text" },
                            value: "{field.value}",
                            placeholder: "{field.placeholder()}",
                            oninput: {
                                let form = form.clone();
                                let name = field.name.clone();
                                move |e: FormEvent| {
                                    let mut updated = form.clone();
                                    updated.set_value(&name, &e.value());
                                    on_change.call(updated);
                                }
                            },
                        }
                    }
                    p { class: "help", "{field.help}" }
                }
            }

            label { class: "checkbox mb-3",
                input {
                    r#type: "checkbox",
                    checked: form.enabled,
                    onchange: {
                        let form = form.clone();
                        move |e: FormEvent| {
                            let mut updated = form.clone();
                            updated.enabled = e.value() == "true";
                            on_change.call(updated);
                        }
                    },
                }
                " Connect this account"
            }

            for error in form.errors.clone() {
                p { class: "has-text-danger is-size-7", "✗ {error}" }
            }
            if let Some(rejection) = server_error {
                p { class: "has-text-danger is-size-7", "✗ {rejection}" }
            }

            Buttons {
                Button {
                    color: BulmaColor::Primary,
                    onclick: {
                        let form = form.clone();
                        move |_| on_save.call(form.clone())
                    },
                    "Save"
                }
                Button { onclick: move |_| on_cancel.call(()), "Cancel" }
            }
        }
    }
}

/// The routing tab.
#[component]
fn RouteTable(
    data: MessengersPanelData,
    account: Signal<String>,
    channel: Signal<String>,
    thread: Signal<String>,
    on_command: EventHandler<MessengerCommand>,
) -> Element {
    // A thread id typed as text has to become a number before it can be sent;
    // an unparseable one disables the button rather than sending nonsense.
    let parsed_thread = thread.read().trim().parse::<u64>().ok();
    let selected_thread =
        parsed_thread.and_then(|id| data.threads.iter().find(|t| t.thread_id == id).cloned());
    let can_add = !account.read().trim().is_empty() && selected_thread.is_some();

    rsx! {
        div {
            p { class: "is-size-7 has-text-grey mb-2",
                "A channel bound to a thread shares that thread's conversation and \
                 working directory. Channels with no route get their own \
                 conversation and the default workspace."
            }

            Table { striped: true, hoverable: true, fullwidth: true,
                thead {
                    tr {
                        th { "Channel" }
                        th { "Thread" }
                        th { "State" }
                        th { "" }
                    }
                }
                tbody {
                    for route in data.routes.clone() {
                        tr { key: "{route.messenger}/{route.channel.clone().unwrap_or_default()}",
                            td { "{route.source_label()}" }
                            td { "{route.target_label()}" }
                            td {
                                Tag {
                                    color: tone_color(route.tone()),
                                    if route.is_dangling() {
                                        "Missing thread"
                                    } else if route.enabled {
                                        "Active"
                                    } else {
                                        "Disabled"
                                    }
                                }
                            }
                            td {
                                Buttons {
                                    Button {
                                        size: BulmaSize::Small,
                                        onclick: {
                                            let route = route.clone();
                                            move |_| on_command.call(MessengerCommand::SaveRoute {
                                                messenger: route.messenger.clone(),
                                                channel: route.channel.clone(),
                                                thread_id: route.thread_id,
                                                agent_id: Some(route.agent_id.clone()),
                                                enabled: !route.enabled,
                                            })
                                        },
                                        if route.enabled { "Disable" } else { "Enable" }
                                    }
                                    Button {
                                        size: BulmaSize::Small,
                                        color: BulmaColor::Danger,
                                        onclick: {
                                            let route = route.clone();
                                            move |_| on_command.call(MessengerCommand::DeleteRoute {
                                                messenger: route.messenger.clone(),
                                                channel: route.channel.clone(),
                                            })
                                        },
                                        "Delete"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            h4 { class: "title is-6", "Bind a channel to a thread" }
            div { class: "field is-grouped",
                div { class: "control",
                    div { class: "select is-small",
                        select {
                            onchange: move |e: FormEvent| account.set(e.value()),
                            option { value: "", "Account…" }
                            for a in data.accounts.clone() {
                                option { key: "{a.name}", value: "{a.name}", "{a.name}" }
                            }
                        }
                    }
                }
                div { class: "control",
                    input {
                        class: "input is-small",
                        r#type: "text",
                        placeholder: "Channel id (blank = all channels)",
                        value: "{channel.read()}",
                        oninput: move |e: FormEvent| channel.set(e.value()),
                    }
                }
                div { class: "control",
                    div { class: "select is-small",
                        select {
                            onchange: move |e: FormEvent| thread.set(e.value()),
                            option { value: "", "Thread…" }
                            for t in data.threads.clone() {
                                option {
                                    key: "{t.agent_id}/{t.thread_id}",
                                    value: "{t.thread_id}",
                                    "{t.label_for_picker()}"
                                }
                            }
                        }
                    }
                }
                div { class: "control",
                    Button {
                        size: BulmaSize::Small,
                        color: BulmaColor::Primary,
                        disabled: !can_add,
                        onclick: move |_| {
                            let Some(target) = selected_thread.clone() else { return };
                            let channel_value = channel.read().trim().to_string();
                            on_command.call(MessengerCommand::SaveRoute {
                                messenger: account.read().clone(),
                                channel: (!channel_value.is_empty()).then_some(channel_value),
                                thread_id: target.thread_id,
                                agent_id: Some(target.agent_id.clone()),
                                enabled: true,
                            });
                            channel.set(String::new());
                        },
                        "Add route"
                    }
                }
            }
        }
    }
}

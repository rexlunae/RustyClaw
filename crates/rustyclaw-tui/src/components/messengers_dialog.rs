// ── Messengers dialog — messenger setup overlay ───────────────────────────────
//
// Opened with /messengers. Three views share the overlay: the account list,
// the account editor, and the routing table. Which one is showing is decided
// by `MessengersPanelData` — this component only draws what it is handed.
//
// Every derivation (which fields exist, whether a credential is stored, what a
// placeholder should say) lives in `rustyclaw_view::messengers` so the desktop
// client renders the same forms from the same rules.

use iocraft::prelude::*;
use rustyclaw_view::messengers::{MessengerTab, MessengersPanelData};
use rustyclaw_view::tone::Tone;

use crate::theme;

#[derive(Default, Props)]
pub struct MessengersDialogProps {
    pub data: Option<MessengersPanelData>,
}

/// Map a view-layer tone onto the TUI palette.
fn tone_color(tone: Tone) -> Color {
    match tone {
        Tone::Success => theme::SUCCESS,
        Tone::Warning => theme::WARN,
        Tone::Danger => theme::ERROR,
        Tone::Info => theme::INFO,
        Tone::Primary => theme::ACCENT,
        Tone::Neutral => theme::MUTED,
    }
}

/// One rendered line: text plus its colour.
type Line = (String, Color);

/// Account rows, plus the detail block for the selected one.
fn account_lines(data: &MessengersPanelData) -> Vec<Line> {
    let mut lines = Vec::new();
    if data.accounts.is_empty() {
        lines.push((
            "No messenger accounts yet. Press 'n' to add one.".to_string(),
            theme::MUTED,
        ));
        return lines;
    }

    for (i, account) in data.accounts.iter().enumerate() {
        let pointer = if Some(i) == data.selected {
            "▸ "
        } else {
            "  "
        };
        let toggle = if account.enabled { "[x]" } else { "[ ]" };
        lines.push((
            format!(
                "{pointer}{toggle} {} {:<20} {:<16} {}",
                account.icon(),
                account.name,
                account.type_label(),
                account.status_label(),
            ),
            match Some(i) == data.selected {
                true => theme::ACCENT_BRIGHT,
                false => tone_color(account.status_tone()),
            },
        ));
    }

    if let Some(account) = data.selected_account() {
        lines.push((String::new(), theme::TEXT));
        lines.push((
            format!(
                "  Presenting as: {}{}",
                account.display_name,
                match account.display_name_overridden {
                    true => "",
                    false => "  (inherited from the agent)",
                }
            ),
            theme::TEXT,
        ));
        if let Some(bio) = &account.bio {
            lines.push((format!("  Description:   {bio}"), theme::TEXT_DIM));
        }
        if !account.vaulted.is_empty() {
            lines.push((
                format!("  In the vault:  {}", account.vaulted.join(", ")),
                theme::SUCCESS,
            ));
        }
        for (field, value) in &account.fields {
            lines.push((format!("  {field:<14} {value}"), theme::TEXT_DIM));
        }
        if let Some(warning) = account.plaintext_warning() {
            lines.push((
                format!("  ⚠ {warning} — press 'm' to move it into the vault"),
                theme::WARN,
            ));
        }
        if let Some(reason) = &account.unavailable_reason {
            lines.push((format!("  ⚠ {reason}"), theme::ERROR));
        }
    }
    lines
}

/// Route rows.
fn route_lines(data: &MessengersPanelData) -> Vec<Line> {
    let mut lines = Vec::new();
    if data.routes.is_empty() {
        lines.push((
            "No routes. Unrouted channels get their own conversation and the".to_string(),
            theme::MUTED,
        ));
        lines.push((
            "default workspace. Press 'n' to bind one to a thread.".to_string(),
            theme::MUTED,
        ));
        return lines;
    }

    for (i, route) in data.routes.iter().enumerate() {
        let pointer = if Some(i) == data.selected {
            "▸ "
        } else {
            "  "
        };
        let toggle = if route.enabled { "[x]" } else { "[ ]" };
        lines.push((
            format!(
                "{pointer}{toggle} {:<32} → {}",
                route.source_label(),
                route.target_label(),
            ),
            match Some(i) == data.selected {
                true => theme::ACCENT_BRIGHT,
                false => tone_color(route.tone()),
            },
        ));
    }
    lines
}

/// The account editor form.
fn editor_lines(data: &MessengersPanelData) -> Vec<Line> {
    let Some(editor) = &data.editor else {
        return Vec::new();
    };
    let mut lines = vec![
        (editor.title(), theme::ACCENT_BRIGHT),
        (String::new(), theme::TEXT),
    ];

    for (i, field) in editor.fields.iter().enumerate() {
        let focused = i == editor.focused;
        let pointer = if focused { "▸ " } else { "  " };
        let shown = match (field.value.is_empty(), field.masked()) {
            (true, _) => field.placeholder(),
            (false, true) => "•".repeat(field.value.chars().count()),
            (false, false) => field.value.clone(),
        };
        let marker = match (field.required, &field.one_of) {
            (true, _) => "*",
            (_, Some(_)) => "†",
            _ => " ",
        };
        lines.push((
            format!("{pointer}{}{:<18} {shown}", marker, field.label),
            match (focused, field.value.is_empty()) {
                (true, _) => theme::ACCENT_BRIGHT,
                (false, true) => theme::MUTED,
                (false, false) => theme::TEXT,
            },
        ));
    }

    if let Some(field) = editor.focused_field() {
        lines.push((String::new(), theme::TEXT));
        lines.push((format!("  {}", field.help), theme::TEXT_DIM));
    }
    if editor.fields.iter().any(|f| f.one_of.is_some()) {
        lines.push((
            "  † at least one of these is required".to_string(),
            theme::TEXT_DIM,
        ));
    }
    for error in &editor.errors {
        lines.push((format!("  ✗ {error}"), theme::ERROR));
    }
    lines
}

/// The "which backend?" picker shown before a new account's form.
fn picker_lines(data: &MessengersPanelData) -> Vec<Line> {
    let Some(selected) = data.kind_picker else {
        return Vec::new();
    };
    let mut lines = vec![
        ("Add a messenger account".to_string(), theme::ACCENT_BRIGHT),
        (String::new(), theme::TEXT),
    ];
    let kinds = data.selectable_kinds();
    if kinds.is_empty() {
        lines.push((
            "This gateway build has no messenger backends compiled in.".to_string(),
            theme::WARN,
        ));
        return lines;
    }
    for (i, kind) in kinds.iter().enumerate() {
        let pointer = if i == selected { "▸ " } else { "  " };
        lines.push((
            format!("{pointer}{} {:<18} {}", kind.icon, kind.label, kind.summary),
            match i == selected {
                true => theme::ACCENT_BRIGHT,
                false => theme::TEXT,
            },
        ));
    }
    lines
}

/// Key hints for whatever is currently showing.
fn hints(data: &MessengersPanelData) -> Vec<(&'static str, &'static str)> {
    if data.editor.is_some() {
        return vec![("Tab", "next field"), ("Enter", "save"), ("Esc", "cancel")];
    }
    if data.kind_picker.is_some() {
        return vec![("↑↓", "choose"), ("Enter", "continue"), ("Esc", "cancel")];
    }
    match data.tab {
        MessengerTab::Accounts => vec![
            ("↑↓", "select"),
            ("n", "new"),
            ("e", "edit"),
            ("Space", "enable"),
            ("m", "vault plaintext"),
            ("d", "delete"),
            ("t", "routes"),
            ("Esc", "close"),
        ],
        MessengerTab::Routes => vec![
            ("↑↓", "select"),
            ("n", "new route"),
            ("Space", "enable"),
            ("d", "delete"),
            ("t", "accounts"),
            ("Esc", "close"),
        ],
    }
}

#[component]
pub fn MessengersDialog(props: &MessengersDialogProps) -> impl Into<AnyElement<'static>> {
    let Some(data) = props.data.clone() else {
        return element! {
            View(
                width: 100pct,
                height: 100pct,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
            ) {
                Text(content: "(messenger setup not loaded)", color: theme::MUTED)
            }
        };
    };

    // Exactly one of these is non-empty; the panel state decides which.
    let (title, lines) = if data.editor.is_some() {
        ("Messenger setup".to_string(), editor_lines(&data))
    } else if data.kind_picker.is_some() {
        ("Messenger setup".to_string(), picker_lines(&data))
    } else {
        let title = format!("Messenger setup — {}", data.tab.label());
        let lines = match data.tab {
            MessengerTab::Accounts => account_lines(&data),
            MessengerTab::Routes => route_lines(&data),
        };
        (title, lines)
    };

    let summary = match (
        data.editor.is_some() || data.kind_picker.is_some(),
        data.tab,
    ) {
        (true, _) => String::new(),
        (false, MessengerTab::Accounts) => data.summary(),
        (false, MessengerTab::Routes) => format!("{} route(s)", data.routes.len()),
    };

    let status = data.status.clone();
    let status_color = tone_color(data.status_tone());
    let hint_pairs = hints(&data);

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 82pct,
                max_height: 88pct,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::INFO,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                Text(content: title, color: theme::INFO, weight: Weight::Bold)

                #((!summary.is_empty()).then(|| element! {
                    Text(content: summary.clone(), color: theme::TEXT_DIM)
                }))

                View(height: 1)

                #(lines.into_iter().enumerate().map(|(i, (text, color))| element! {
                    View(key: i as u32) {
                        Text(content: text, color: color, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                #(status.map(|message| element! {
                    Text(content: message, color: status_color, wrap: TextWrap::Wrap)
                }))

                View(flex_direction: FlexDirection::Row) {
                    #(hint_pairs.into_iter().enumerate().map(|(i, (key, label))| element! {
                        View(key: i as u32, flex_direction: FlexDirection::Row) {
                            Text(content: format!("{key} "), color: theme::ACCENT_BRIGHT)
                            Text(content: format!("{label}  "), color: theme::MUTED)
                        }
                    }))
                }
            }
        }
    }
}

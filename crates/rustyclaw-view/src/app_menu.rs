//! Shared menu model for application-level actions.
//!
//! Defines a hierarchy of named menu items that both clients can
//! render in their native way (OS menu bar on desktop, in-app
//! overlay on TUI).

/// A single action that a menu item triggers.
///
/// Clients map these to their own event dispatch; this enum owns
/// only the semantic intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    // ── File ────────────────────────────────────────────────────
    NewThread,
    CloseThread,
    AttachFile,
    AttachDirectory,
    ChangeDirectory,
    Quit,

    // ── Edit ────────────────────────────────────────────────────
    ClearAttachments,

    // ── View ────────────────────────────────────────────────────
    ToggleLeftSidebar,
    ToggleRightSidebar,

    // ── Tools ───────────────────────────────────────────────────
    OpenSecrets,
    OpenSettings,
    OpenSwarm,
    OpenSkills,
    OpenToolPerms,

    // ── Session ─────────────────────────────────────────────────
    SwitchProvider,
    SwitchModel,

    // ── Help ────────────────────────────────────────────────────
    ShowShortcuts,
    OpenDocs,
}

/// A leaf entry in the menu hierarchy.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// Display label.
    pub label: &'static str,
    /// Keyboard shortcut description (display only — not enforced here).
    pub shortcut: Option<&'static str>,
    /// The semantic action this item triggers.
    pub action: MenuAction,
    /// Whether this item is currently enabled.
    pub enabled: bool,
}

impl MenuItem {
    pub const fn new(
        label: &'static str,
        shortcut: Option<&'static str>,
        action: MenuAction,
    ) -> Self {
        Self {
            label,
            shortcut,
            action,
            enabled: true,
        }
    }
}

/// An entry in a menu (either a separator, a leaf item, or a sub-menu).
#[derive(Clone, Debug, PartialEq)]
pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

/// A top-level menu (e.g. "File", "View").
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    /// Display name of this menu group.
    pub label: &'static str,
    /// Entries inside this menu.
    pub entries: Vec<MenuEntry>,
}

/// The full application menu bar.
#[derive(Clone, Debug, PartialEq)]
pub struct AppMenuBar {
    pub menus: Vec<Menu>,
}

impl AppMenuBar {
    /// Return the canonical application menu structure shared by both
    /// desktop (OS menu bar) and TUI (in-app overlay).
    pub fn canonical() -> Self {
        use MenuAction::*;
        use MenuEntry::{Item, Separator};

        Self {
            menus: vec![
                Menu {
                    label: "File",
                    entries: vec![
                        Item(MenuItem::new("New Thread", Some("Ctrl+Shift+E"), NewThread)),
                        Item(MenuItem::new("Close Thread", Some("Ctrl+W"), CloseThread)),
                        Separator,
                        Item(MenuItem::new("Attach File…", Some("Ctrl+Shift+A"), AttachFile)),
                        Item(MenuItem::new(
                            "Attach Directory…",
                            Some("Ctrl+Shift+D"),
                            AttachDirectory,
                        )),
                        Item(MenuItem::new(
                            "Clear Attachments",
                            None,
                            ClearAttachments,
                        )),
                        Separator,
                        Item(MenuItem::new(
                            "Change Directory…",
                            Some("Ctrl+Shift+O"),
                            ChangeDirectory,
                        )),
                        Separator,
                        Item(MenuItem::new("Quit", Some("Ctrl+Q"), Quit)),
                    ],
                },
                Menu {
                    label: "View",
                    entries: vec![
                        Item(MenuItem::new(
                            "Toggle Thread Sidebar",
                            Some("Ctrl+B"),
                            ToggleLeftSidebar,
                        )),
                        Item(MenuItem::new(
                            "Toggle File Browser",
                            Some("Ctrl+Shift+B"),
                            ToggleRightSidebar,
                        )),
                    ],
                },
                Menu {
                    label: "Tools",
                    entries: vec![
                        Item(MenuItem::new(
                            "Secrets Vault…",
                            Some("Ctrl+Shift+S"),
                            OpenSecrets,
                        )),
                        Item(MenuItem::new(
                            "Settings…",
                            Some("Ctrl+,"),
                            OpenSettings,
                        )),
                        Separator,
                        Item(MenuItem::new("Swarm Manager…", None, OpenSwarm)),
                        Item(MenuItem::new("Skills…", None, OpenSkills)),
                        Item(MenuItem::new("Tool Permissions…", None, OpenToolPerms)),
                        Separator,
                        Item(MenuItem::new("Switch Provider…", None, SwitchProvider)),
                        Item(MenuItem::new("Switch Model…", None, SwitchModel)),
                    ],
                },
                Menu {
                    label: "Help",
                    entries: vec![
                        Item(MenuItem::new("Keyboard Shortcuts", Some("?"), ShowShortcuts)),
                        Item(MenuItem::new("Documentation", None, OpenDocs)),
                    ],
                },
            ],
        }
    }

    /// Convenience: iterate all leaf items across all menus.
    pub fn all_items(&self) -> impl Iterator<Item = &MenuItem> {
        self.menus.iter().flat_map(|m| {
            m.entries.iter().filter_map(|e| match e {
                MenuEntry::Item(i) => Some(i),
                MenuEntry::Separator => None,
            })
        })
    }
}

/// State for a TUI in-app menu overlay.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TuiMenuState {
    /// Whether the menu bar is visible at the top.
    pub is_open: bool,
    /// Which top-level menu is currently focused (index into `AppMenuBar::menus`).
    pub focused_menu: usize,
    /// Which entry inside the focused menu is currently highlighted.
    pub focused_entry: usize,
}

impl TuiMenuState {
    pub fn open(&mut self) {
        self.is_open = true;
        self.focused_menu = 0;
        self.focused_entry = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn move_menu_left(&mut self, total: usize) {
        if self.focused_menu == 0 {
            self.focused_menu = total.saturating_sub(1);
        } else {
            self.focused_menu -= 1;
        }
        self.focused_entry = 0;
    }

    pub fn move_menu_right(&mut self, total: usize) {
        self.focused_menu = (self.focused_menu + 1) % total.max(1);
        self.focused_entry = 0;
    }

    pub fn move_entry_up(&mut self, total_entries: usize) {
        if self.focused_entry == 0 {
            self.focused_entry = total_entries.saturating_sub(1);
        } else {
            self.focused_entry -= 1;
        }
    }

    pub fn move_entry_down(&mut self, total_entries: usize) {
        self.focused_entry = (self.focused_entry + 1) % total_entries.max(1);
    }
}

//! Native OS menu bar (muda) for the desktop client.

use std::sync::OnceLock;

use dioxus::desktop::muda;

/// Stable IDs for all menu items, stored after `build_app_menu()` runs.
pub struct AppMenuIds {
    pub new_thread: muda::MenuId,
    pub new_connection_window: muda::MenuId,
    pub quit: muda::MenuId,
    pub toggle_left_sidebar: muda::MenuId,
    pub toggle_right_sidebar: muda::MenuId,
    pub settings: muda::MenuId,
    pub secrets: muda::MenuId,
    pub pair: muda::MenuId,
    pub swarm: muda::MenuId,
    pub skills: muda::MenuId,
    pub system_info: muda::MenuId,
    pub local_models: muda::MenuId,
    pub services: muda::MenuId,
}

static APP_MENU_IDS: OnceLock<AppMenuIds> = OnceLock::new();

/// Returns the menu item IDs after `build_app_menu()` has been called.
pub fn app_menu_ids() -> Option<&'static AppMenuIds> {
    APP_MENU_IDS.get()
}

/// Build the native menu bar and register all item IDs.
/// Call this exactly once before launching the Dioxus app.
pub fn build_app_menu() -> muda::Menu {
    // ── File ──────────────────────────────────────────────────────────────
    let new_thread = muda::MenuItem::new("New Thread", true, "CmdOrCtrl+T".parse().ok());
    let new_connection_window = muda::MenuItem::new(
        "New Connection Window",
        true,
        "CmdOrCtrl+Shift+N".parse().ok(),
    );
    let quit = muda::PredefinedMenuItem::quit(None);

    // ── View ──────────────────────────────────────────────────────────────
    let toggle_left = muda::MenuItem::new("Toggle Left Sidebar", true, "CmdOrCtrl+B".parse().ok());
    let toggle_right = muda::MenuItem::new(
        "Toggle Right Sidebar",
        true,
        "CmdOrCtrl+Shift+B".parse().ok(),
    );

    // ── Tools ─────────────────────────────────────────────────────────────
    let settings = muda::MenuItem::new("Settings…", true, None);
    let system_info = muda::MenuItem::new("System Info…", true, "CmdOrCtrl+I".parse().ok());
    let secrets = muda::MenuItem::new("Secrets Vault…", true, None);
    let pair = muda::MenuItem::new("Pair Gateway…", true, None);
    let swarm = muda::MenuItem::new("Swarm Manager…", true, None);
    let skills = muda::MenuItem::new("Skills…", true, None);
    let services = muda::MenuItem::new("Services…", true, "CmdOrCtrl+J".parse().ok());
    let local_models = muda::MenuItem::new("Local Models…", true, "CmdOrCtrl+Shift+L".parse().ok());

    // Register all IDs before the items are moved into the menu.
    let ids = AppMenuIds {
        new_thread: new_thread.id().clone(),
        new_connection_window: new_connection_window.id().clone(),
        quit: quit.id().clone(),
        toggle_left_sidebar: toggle_left.id().clone(),
        toggle_right_sidebar: toggle_right.id().clone(),
        settings: settings.id().clone(),
        secrets: secrets.id().clone(),
        pair: pair.id().clone(),
        swarm: swarm.id().clone(),
        skills: skills.id().clone(),
        system_info: system_info.id().clone(),
        local_models: local_models.id().clone(),
        services: services.id().clone(),
    };
    let _ = APP_MENU_IDS.set(ids);

    let file_sep = muda::PredefinedMenuItem::separator();
    let tools_sep = muda::PredefinedMenuItem::separator();

    let file_menu = muda::Submenu::with_items(
        "File",
        true,
        &[&new_thread, &new_connection_window, &file_sep, &quit],
    )
    .expect("failed to build File menu");

    // ── Edit ───────────────────────────────────────────────────────────────
    // Predefined items wire Ctrl+X/C/V/A to the webview's clipboard and
    // text-selection commands on all platforms.
    let edit_menu = muda::Submenu::with_items(
        "Edit",
        true,
        &[
            &muda::PredefinedMenuItem::cut(None),
            &muda::PredefinedMenuItem::copy(None),
            &muda::PredefinedMenuItem::paste(None),
            &muda::PredefinedMenuItem::separator(),
            &muda::PredefinedMenuItem::select_all(None),
        ],
    )
    .expect("failed to build Edit menu");

    let view_menu = muda::Submenu::with_items(
        "View",
        true,
        &[&toggle_left, &toggle_right, &system_info, &services],
    )
    .expect("failed to build View menu");

    let tools_menu = muda::Submenu::with_items(
        "Tools",
        true,
        &[
            &settings,
            &secrets,
            &pair,
            &tools_sep,
            &swarm,
            &skills,
            &local_models,
        ],
    )
    .expect("failed to build Tools menu");

    let menu = muda::Menu::new();
    menu.append_items(&[&file_menu, &edit_menu, &view_menu, &tools_menu])
        .expect("failed to append submenus");
    menu
}

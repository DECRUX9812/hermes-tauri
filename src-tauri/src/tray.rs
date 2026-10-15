use std::sync::{Mutex, MutexGuard};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::sidecar::{self, BootManager};

const TRAY_ID: &str = "hermes-tray";
const MENU_SHOW: &str = "show";
const MENU_HIDE: &str = "hide";
const MENU_NEW_CHAT: &str = "new-chat";
const MENU_QUIT: &str = "quit";
const MENU_UPDATE: &str = "install-update";

/// Holds the tray icon so it stays alive (the icon is removed when its last
/// handle is dropped) and remembers whether the update item was added.
pub struct TrayState {
    tray: Mutex<Option<TrayIcon<Wry>>>,
    update_item: Mutex<bool>,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            tray: Mutex::new(None),
            update_item: Mutex::new(false),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn log(message: &str) {
    eprintln!("[hermes-tauri] {message}");
}

/// Builds the tray with Show / Hide / New chat / Quit. A failure (no
/// StatusNotifier host, which is normal under Xvfb) is logged once and the
/// shell falls back to close = quit.
pub fn create(app: &AppHandle) {
    match build_tray(app, false) {
        Ok(tray) => {
            let state = app.state::<TrayState>();
            *lock(&state.tray) = Some(tray);
            sidecar::log("Tray icon created");
        }
        Err(error) => {
            sidecar::log(&format!(
                "No tray icon: {error}. Closing the window will quit the shell"
            ));
        }
    }
}

/// Whether a tray exists; window close hides to the tray only then.
pub fn available(app: &AppHandle) -> bool {
    lock(&app.state::<TrayState>().tray).is_some()
}

fn build_tray(app: &AppHandle, update_available: bool) -> Result<TrayIcon, String> {
    let menu = build_menu(app, update_available)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        // Left click toggles the window; the menu opens on right click.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id().0.as_str()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder
        .build(app)
        .map_err(|error| format!("could not build the tray icon: {error}"))
}

fn build_menu(app: &AppHandle, update_available: bool) -> Result<Menu<Wry>, String> {
    let separator = || PredefinedMenuItem::separator(app).map_err(|error| error.to_string());
    let show = MenuItem::with_id(app, MENU_SHOW, "Show Hermes", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let hide = MenuItem::with_id(app, MENU_HIDE, "Hide", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let new_chat = MenuItem::with_id(app, MENU_NEW_CHAT, "New chat", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)
        .map_err(|error| error.to_string())?;

    if update_available {
        let update = MenuItem::with_id(
            app,
            MENU_UPDATE,
            "Install update and restart",
            true,
            None::<&str>,
        )
        .map_err(|error| error.to_string())?;
        let top = separator()?;
        let bottom = separator()?;
        Menu::with_items(
            app,
            &[&update, &top, &show, &hide, &new_chat, &bottom, &quit],
        )
        .map_err(|error| error.to_string())
    } else {
        let bottom = separator()?;
        Menu::with_items(app, &[&show, &hide, &new_chat, &bottom, &quit])
            .map_err(|error| error.to_string())
    }
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        MENU_SHOW => show_window(app),
        MENU_HIDE => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        MENU_NEW_CHAT => new_chat(app),
        MENU_QUIT => {
            sidecar::log("Quit requested from the tray");
            app.exit(0);
        }
        MENU_UPDATE => {
            let app = app.clone();
            // Downloading can take a while; never block the main thread.
            std::thread::spawn(move || crate::updater::install_and_restart(&app));
        }
        _ => {}
    }
}

/// Shows and focuses the main window (tray item, single-instance focus).
pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Left click and the global hotkey: hide when visible and focused, else
/// show and focus.
pub fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
        let _ = window.hide();
        log("Hid the window");
    } else {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// "New chat": the UI's own new-session action ends on the `#/` hash route,
/// so the tray item navigates the main webview there. Through the deferred
/// path, so a click while the launch page still holds the session token
/// cannot strip it (review N1); a deep link that is already waiting wins over
/// it (review L1, `sidecar::deferred_route`).
fn new_chat(app: &AppHandle) {
    let manager = app.state::<BootManager>();
    if manager.origin().is_none() {
        log("Ignored New chat: Hermes is not connected yet");
        return;
    }
    manager.apply_route_or_defer(app, hermes_shell_config::Route::Home.hash());
}

/// Adds the "Install update and restart" item once an update is found.
pub fn set_update_available(app: &AppHandle) {
    let state = app.state::<TrayState>();
    {
        let mut marked = lock(&state.update_item);
        if *marked {
            return;
        }
        *marked = true;
    }
    let app = app.clone();
    let runner = app.clone();
    let _ = runner.run_on_main_thread(move || {
        let tray = {
            let state = app.state::<TrayState>();
            let mut tray = lock(&state.tray);
            match tray.take() {
                Some(tray) => tray,
                None => {
                    sidecar::log("Update found, but no tray exists to show the install item");
                    return;
                }
            }
        };
        match build_menu(&app, true) {
            Ok(menu) => {
                if let Err(error) = tray.set_menu(Some(menu)) {
                    sidecar::log(&format!("Could not add the update tray item: {error}"));
                } else {
                    sidecar::log("Added the update item to the tray");
                }
            }
            Err(error) => sidecar::log(&format!("Could not build the update tray menu: {error}")),
        }
        *lock(&app.state::<TrayState>().tray) = Some(tray);
    });
}

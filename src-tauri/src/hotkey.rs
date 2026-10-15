use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;

use crate::tray;

/// Registers the global toggle shortcut. A `None` accelerator means the user
/// disabled it. A registration failure (taken by another app, or Wayland
/// without the portal) is logged and surfaced once as a non-fatal
/// notification; the shell keeps running without it.
pub fn register(app: &AppHandle, accelerator: Option<String>) {
    let Some(accelerator) = accelerator else {
        sidecar_log("Global hotkey disabled by configuration");
        return;
    };

    if on_wayland() {
        let message = format!("The global hotkey {accelerator} is unavailable on Wayland");
        sidecar_log(&message);
        surface_failure(app, &message);
        return;
    }

    if let Err(error) =
        app.global_shortcut()
            .on_shortcut(accelerator.as_str(), |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    tray::toggle_window(app);
                }
            })
    {
        let message = format!("The global hotkey {accelerator} is unavailable: {error}");
        sidecar_log(&message);
        surface_failure(app, &message);
    } else {
        sidecar_log(&format!("Global hotkey registered: {accelerator}"));
    }
}

/// The global-hotkey crate is X11-only; on Wayland its register can swallow
/// the failure and still report success (R2, L1), so never claim a hotkey
/// that would not fire.
fn on_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|session| session == "wayland")
}

fn surface_failure(app: &AppHandle, message: &str) {
    if let Err(notify_error) = app
        .notification()
        .builder()
        .title("Hermes Tauri")
        .body(message)
        .show()
    {
        sidecar_log(&format!(
            "Could not show the hotkey notification: {notify_error}"
        ));
    }
}

fn sidecar_log(message: &str) {
    crate::sidecar::log(message);
}

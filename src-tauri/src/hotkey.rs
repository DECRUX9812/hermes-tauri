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
        if let Err(notify_error) = app
            .notification()
            .builder()
            .title("Hermes Tauri")
            .body(&message)
            .show()
        {
            sidecar_log(&format!(
                "Could not show the hotkey notification: {notify_error}"
            ));
        }
    } else {
        sidecar_log(&format!("Global hotkey registered: {accelerator}"));
    }
}

fn sidecar_log(message: &str) {
    crate::sidecar::log(message);
}

use tauri::{AppHandle, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use crate::sidecar::{self, BootManager};

/// Navigates the main webview for a `hermes-tauri://open?path=…` link.
/// Whitelisted routes map to their hash path; anything else is ignored and
/// logged. Returns whether the link was recognized.
pub fn handle_url(app: &AppHandle, url: &str) -> bool {
    let Some(manager) = app.try_state::<BootManager>() else {
        // A launch can arrive while the first instance is still in setup.
        sidecar::log(&format!(
            "Ignored deep link {url:?}: the shell is still starting"
        ));
        return false;
    };
    match hermes_shell_config::parse(url) {
        Ok(route) => {
            manager.apply_route_or_defer(app, route.hash());
            true
        }
        Err(error) => {
            sidecar::log(&format!("Ignored deep link {url:?}: {error}"));
            false
        }
    }
}

/// Forwards deep-link arguments of a second launch (single-instance callback).
pub fn handle_args(app: &AppHandle, args: &[String]) {
    if let Some(url) = hermes_shell_config::from_args(args) {
        handle_url(app, url);
    }
}

/// Handles the current launch arguments (Linux launches carry the URL as
/// argv) and registers the handler for later events.
pub fn init(app: &AppHandle) {
    let event_app = app.clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            handle_url(&event_app, url.as_str());
        }
    });

    handle_args(app, &std::env::args().skip(1).collect::<Vec<String>>());
}

use tauri::{AppHandle, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use crate::sidecar::{self, BootManager};

/// Navigates the main webview for a `hermes-tauri://open?path=…` link.
/// Whitelisted routes map to their hash path; anything else is ignored and
/// logged. Returns whether the link was recognized.
pub fn handle_url(app: &AppHandle, url: &str) -> bool {
    match hermes_shell_config::parse(url) {
        Ok(route) => {
            let manager = app.state::<BootManager>();
            match manager.origin() {
                Some(origin) => manager.apply_route(app, &origin, route.hash()),
                None => {
                    sidecar::log(&format!(
                        "Deep link {url:?} seen while Hermes is still starting; applying it when ready"
                    ));
                    manager.set_pending_route(route.hash().to_owned());
                }
            }
            true
        }
        Err(error) => {
            sidecar::log(&format!("Ignored deep link {url:?}: {error}"));
            false
        }
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

    let mut found = None;
    for argument in std::env::args().skip(1) {
        if let Some(url) = hermes_shell_config::from_args(std::slice::from_ref(&argument)) {
            found = Some(url.to_owned());
            break;
        }
    }
    if let Some(url) = found {
        handle_url(app, &url);
    }
}

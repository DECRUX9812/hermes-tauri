use std::sync::atomic::{AtomicU64, Ordering};

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};
use tauri_plugin_opener::OpenerExt;

use crate::sidecar::{self, BootManager};

pub fn handle_navigation(app: &AppHandle, url: &Url, backend_origin: Option<&str>) -> bool {
    // srcdoc and about:blank iframes are inert and must load inside allowed pages.
    if is_allowed(url, backend_origin) || url.scheme() == "about" {
        return true;
    }

    if is_http(url) {
        // The proof reads this line: an off-origin navigation never loads in
        // a shell window (the token is redacted).
        sidecar::log(&format!(
            "Opening externally: {}",
            hermes_sidecar::redact(url.as_str())
        ));
        open_external(app, url);
    }
    false
}

pub fn handle_new_window(
    app: &AppHandle,
    url: Url,
    features: tauri::webview::NewWindowFeatures,
    backend_origin: Option<&str>,
) -> NewWindowResponse<Wry> {
    match popup_decision(&url, backend_origin) {
        // Same-origin popups (including `blob:` previews) become real windows
        // of ours. `window_features` makes the webview a related view of the
        // caller on Linux (tauri 2.12.4, webview_window.rs `window_features`),
        // and the window carries the same navigation and new-window guards.
        // It gets no runtime capability: those stay scoped to `main`.
        PopupDecision::Preview => match create_preview_window(app, url, features) {
            Ok(window) => NewWindowResponse::Create { window },
            Err(error) => {
                sidecar::log(&format!(
                    "Denied a popup the shell could not create: {error}"
                ));
                NewWindowResponse::Deny
            }
        },
        PopupDecision::OpenExternal => {
            open_external(app, &url);
            NewWindowResponse::Deny
        }
        PopupDecision::Deny => NewWindowResponse::Deny,
    }
}

/// What a `window.open` request may do. Pure so it is testable without GTK.
#[derive(Debug, PartialEq, Eq)]
enum PopupDecision {
    /// Open as a guarded preview window on the boot page or backend origin.
    Preview,
    /// Open in the system browser, no in-app window.
    OpenExternal,
    /// Refuse.
    Deny,
}

fn popup_decision(url: &Url, backend_origin: Option<&str>) -> PopupDecision {
    if is_allowed(url, backend_origin) {
        return PopupDecision::Preview;
    }
    if is_http(url) {
        return PopupDecision::OpenExternal;
    }
    PopupDecision::Deny
}

static PREVIEW_COUNTER: AtomicU64 = AtomicU64::new(1);

fn create_preview_window(
    app: &AppHandle,
    url: Url,
    features: tauri::webview::NewWindowFeatures,
) -> Result<WebviewWindow<Wry>, String> {
    let label = format!("preview-{}", PREVIEW_COUNTER.fetch_add(1, Ordering::SeqCst));
    let navigation_app = app.clone();
    let new_window_app = app.clone();
    WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url.clone()))
        .title("Hermes")
        .inner_size(1100.0, 800.0)
        .window_features(features)
        .on_navigation(move |url| {
            let backend_origin = navigation_app.state::<BootManager>().origin();
            handle_navigation(&navigation_app, url, backend_origin.as_deref())
        })
        .on_new_window(move |url, _features| {
            let backend_origin = new_window_app.state::<BootManager>().origin();
            // Cross-origin new-window requests go to the browser. A preview
            // window never opens further popups: popups opening popups is
            // unbounded nesting with no user value (review F4).
            match popup_decision(&url, backend_origin.as_deref()) {
                PopupDecision::OpenExternal => {
                    sidecar::log(&format!(
                        "Opening externally: {}",
                        hermes_sidecar::redact(url.as_str())
                    ));
                    open_external(&new_window_app, &url);
                    NewWindowResponse::Deny
                }
                PopupDecision::Preview | PopupDecision::Deny => NewWindowResponse::Deny,
            }
        })
        .build()
        .inspect(|window| {
            sidecar::log(&format!(
                "Opened preview window {:?} for {}",
                window.label(),
                hermes_sidecar::redact(url.as_str())
            ));
        })
        .map_err(|error| format!("could not build the preview window {label:?}: {error}"))
}

fn is_allowed(url: &Url, backend_origin: Option<&str>) -> bool {
    is_boot_url(url)
        || backend_origin.is_some_and(|origin| url.origin().ascii_serialization() == origin)
}

fn is_boot_url(url: &Url) -> bool {
    (url.scheme() == "tauri" && url.host_str() == Some("localhost") && url.port().is_none())
        || (matches!(url.scheme(), "http" | "https")
            && url.host_str() == Some("tauri.localhost")
            && url.port().is_none())
}

fn is_http(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

fn open_external(app: &AppHandle, url: &Url) {
    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("Could not open external URL: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("test URL parses")
    }

    const ORIGIN: &str = "http://127.0.0.1:18810";

    #[test]
    fn backend_and_boot_origins_open_as_guarded_previews() {
        assert_eq!(
            popup_decision(&url("http://127.0.0.1:18810/#/team"), Some(ORIGIN)),
            PopupDecision::Preview
        );
        // A blob inherits the origin of its inner URL (url crate, origin.rs).
        assert_eq!(
            popup_decision(&url("blob:http://127.0.0.1:18810/uuid"), Some(ORIGIN)),
            PopupDecision::Preview
        );
        assert_eq!(
            popup_decision(&url("tauri://localhost/index.html"), None),
            PopupDecision::Preview
        );
    }

    #[test]
    fn other_http_origins_go_to_the_browser() {
        assert_eq!(
            popup_decision(&url("https://example.com/page"), Some(ORIGIN)),
            PopupDecision::OpenExternal
        );
        // A blob minted on another origin cannot render anywhere (the system
        // browser cannot resolve a foreign blob URL), so it is denied.
        assert_eq!(
            popup_decision(&url("blob:https://example.com/uuid"), Some(ORIGIN)),
            PopupDecision::Deny
        );
    }

    #[test]
    fn everything_else_is_denied() {
        assert_eq!(
            popup_decision(&url("file:///etc/passwd"), Some(ORIGIN)),
            PopupDecision::Deny
        );
        assert_eq!(
            popup_decision(&url("hermes-tauri://open?path=/team"), Some(ORIGIN)),
            PopupDecision::Deny
        );
    }
}

use tauri::{webview::NewWindowResponse, AppHandle, Url, Wry};
use tauri_plugin_opener::OpenerExt;

pub fn handle_navigation(app: &AppHandle, url: &Url, backend_origin: Option<&str>) -> bool {
    if is_allowed(url, backend_origin) {
        return true;
    }

    if is_http(url) {
        open_external(app, url);
    }
    false
}

pub fn handle_new_window(
    app: &AppHandle,
    url: Url,
    backend_origin: Option<&str>,
) -> NewWindowResponse<Wry> {
    if is_allowed(&url, backend_origin) {
        return NewWindowResponse::Allow;
    }

    if is_http(&url) {
        open_external(app, &url);
    }
    NewWindowResponse::Deny
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

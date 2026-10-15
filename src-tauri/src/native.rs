use serde::Deserialize;
use tauri::{AppHandle, Url};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPayload {
    title: Option<String>,
    body: Option<String>,
}

#[tauri::command]
pub fn native_open_external(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|error| format!("Invalid URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Only HTTP and HTTPS URLs without credentials can be opened".to_owned());
    }

    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|error| format!("Could not open URL: {error}"))
}

#[tauri::command]
pub fn native_notify(app: AppHandle, payload: NotificationPayload) -> bool {
    let title = payload.title.filter(|title| !title.trim().is_empty());
    let mut builder = app
        .notification()
        .builder()
        .title(title.as_deref().unwrap_or("Hermes"));
    if let Some(body) = payload.body {
        builder = builder.body(body);
    }

    match builder.show() {
        Ok(()) => true,
        Err(error) => {
            eprintln!("Could not display Hermes notification: {error}");
            false
        }
    }
}

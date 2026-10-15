use serde::Deserialize;
use tauri::{AppHandle, Url};
use tauri_plugin_dialog::DialogExt;
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
    // Desktop notification daemons always grant the permission (the plugin's
    // `request_permission` documents that it never prompts), so there is
    // nothing to ask for here; the browser bridge's gating is web-only.
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

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SelectPathsOptions {
    title: Option<String>,
    default_path: Option<String>,
    directories: Option<bool>,
    multiple: Option<bool>,
    filters: Option<Vec<DialogFilter>>,
}

#[derive(Deserialize)]
pub struct DialogFilter {
    name: String,
    extensions: Vec<String>,
}

/// Native file/folder picker for `hermesDesktop.selectPaths`. Returns the
/// chosen paths, or an empty list when the dialog is canceled — the same
/// shape as the browser bridge's `<input type=file>` fallback.
#[tauri::command]
pub async fn native_select_paths(
    app: AppHandle,
    options: Option<SelectPathsOptions>,
) -> Result<Vec<String>, String> {
    // The blocking dialog must never run on the main thread.
    tauri::async_runtime::spawn_blocking(move || {
        let options = options.unwrap_or_default();
        let mut builder = app.dialog().file();

        if let Some(title) = options.title {
            builder = builder.set_title(title);
        }
        if let Some(path) = options.default_path {
            builder = builder.set_directory(path);
        }
        for filter in options.filters.iter().flatten() {
            let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
            builder = builder.add_filter(&filter.name, &extensions);
        }

        let directories = options.directories == Some(true);
        let multiple = options.multiple == Some(true);
        let picked = match (directories, multiple) {
            (true, true) => builder.blocking_pick_folders(),
            (true, false) => builder.blocking_pick_folder().map(|path| vec![path]),
            (false, true) => builder.blocking_pick_files(),
            (false, false) => builder.blocking_pick_file().map(|path| vec![path]),
        };

        Ok(picked
            .unwrap_or_default()
            .into_iter()
            .map(|path| {
                path.into_path()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect())
    })
    .await
    .map_err(|error| format!("The file dialog failed: {error}"))?
}

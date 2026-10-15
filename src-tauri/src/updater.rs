use std::sync::Mutex;

use tauri::{Manager, Url};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use hermes_shell_config::UpdaterSettings;

use crate::sidecar::log;
use crate::tray;

/// The found update, held until the user asks for it from the tray.
#[derive(Default)]
pub struct UpdateState {
    update: Mutex<Option<Update>>,
}

/// Registers the updater plugin when the build environment configured a
/// signing key and feed, and starts the check schedule. With no pubkey the
/// plugin is not initialized and update checks report "updates not
/// configured" through the logged reason.
pub fn init(app: &tauri::AppHandle, settings: &UpdaterSettings) {
    let (pubkey, endpoints) = match settings {
        UpdaterSettings::Disabled { reason } => {
            log(&format!("Updates not configured: {reason}"));
            return;
        }
        UpdaterSettings::Enabled { pubkey, endpoints } => (pubkey.clone(), endpoints.clone()),
    };

    let urls: Vec<Url> = endpoints
        .iter()
        .filter_map(|endpoint| match Url::parse(endpoint) {
            Ok(url) => Some(url),
            Err(error) => {
                log(&format!(
                    "Ignoring invalid update endpoint {endpoint:?}: {error}"
                ));
                None
            }
        })
        .collect();
    if urls.is_empty() {
        log("Updates not configured: no valid update feed endpoint");
        return;
    }

    if let Err(error) = app.plugin(tauri_plugin_updater::Builder::new().pubkey(pubkey).build()) {
        log(&format!("Could not initialize the updater: {error}"));
        return;
    }
    app.manage(UpdateState::default());

    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(hermes_shell_config::FIRST_CHECK_DELAY);
        loop {
            check_now(&handle, &urls);
            std::thread::sleep(hermes_shell_config::CHECK_INTERVAL);
        }
    });
    log("Updater configured; first check is scheduled");
}

fn check_now(app: &tauri::AppHandle, endpoints: &[Url]) {
    let handle = app.clone();
    let runner = handle.clone();
    let result = tauri::async_runtime::block_on(async move {
        let builder = runner
            .updater_builder()
            .endpoints(endpoints.to_vec())
            .map_err(|error| error.to_string())?;
        let update = builder
            .build()
            .map_err(|error| error.to_string())?
            .check()
            .await
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(update)
    });

    match result {
        Ok(Some(update)) => {
            let version = update.version.clone();
            match handle.state::<UpdateState>().update.lock() {
                Ok(mut slot) => *slot = Some(update),
                Err(error) => {
                    log(&format!("Update state unavailable: {error}"));
                    return;
                }
            }
            log(&format!("Update {version} is available"));
            if let Err(error) = handle
                .notification()
                .builder()
                .title("Hermes update available")
                .body(format!("Version {version} is ready to install."))
                .show()
            {
                log(&format!("Could not show the update notification: {error}"));
            }
            tray::set_update_available(&handle);
        }
        Ok(None) => log("No update available"),
        Err(error) => log(&format!("Update check failed: {error}")),
    }
}

/// Tray item action: download and install the found update, then restart.
pub fn install_and_restart(app: &tauri::AppHandle) {
    let update = match app.state::<UpdateState>().update.lock() {
        Ok(mut slot) => slot.take(),
        Err(error) => {
            log(&format!("Update state unavailable: {error}"));
            return;
        }
    };
    let Some(update) = update else {
        log("No update is waiting to be installed");
        return;
    };
    let version = update.version.clone();
    log(&format!("Installing update {version}"));
    let result = tauri::async_runtime::block_on(update.download_and_install(
        |chunk, total| {
            if let Some(total) = total {
                log(&format!("Downloading update: {chunk} of {total} bytes"));
            }
        },
        || log("Update download finished"),
    ));
    match result {
        Ok(()) => {
            log(&format!("Update {version} installed; restarting"));
            app.restart();
        }
        Err(error) => log(&format!("Could not install the update: {error}")),
    }
}

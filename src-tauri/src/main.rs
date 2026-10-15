mod adapter;
mod config;
mod deeplink;
mod guard;
mod hotkey;
mod native;
#[cfg(unix)]
mod payload;
mod polyfills;
mod sidecar;
mod tray;
mod updater;

use tauri::{webview::WebviewWindowBuilder, Manager, RunEvent, WebviewUrl, WindowEvent};
use tauri_plugin_notification::NotificationExt;

fn main() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // WebKitGTK's DMABUF renderer can produce a black window under Xvfb and NVIDIA.
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // single-instance must be the first plugin: it decides whether this
    // process becomes the app or forwards its arguments and exits.
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            tray::show_window(app);
            deeplink::handle_args(app, &args);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            sidecar::boot_state,
            sidecar::boot_retry,
            native::native_open_external,
            native::native_notify,
            native::native_select_paths
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let config_dir = handle.path().app_config_dir().ok();
            let log_dir = handle.path().app_log_dir().ok();
            if let Some(dir) = &log_dir {
                // The sidecar crate appends to a log in here; the dir does not
                // exist on a fresh install.
                if let Err(error) = std::fs::create_dir_all(dir) {
                    sidecar::log(&format!(
                        "Could not create the log directory {}: {error}",
                        dir.display()
                    ));
                }
            }

            app.manage(tray::TrayState::default());

            // A payload build ships its backend as one archive in the resource
            // dir; a first run unpacks it into the app-data dir, which takes
            // seconds, so the manager does that on a spawned thread and the
            // boot page shows the phase. Without a payload both files are
            // absent and the shell resolves its backend the usual way.
            #[cfg(unix)]
            let payload = match (handle.path().resource_dir(), handle.path().app_data_dir()) {
                (Ok(resource_dir), Ok(data_dir)) => payload::locate(&resource_dir, &data_dir),
                _ => None,
            };
            // The payload's launcher is a POSIX `sh` script, so no Windows
            // build ships one (docs/python-sidecar.md §5.4).
            #[cfg(not(unix))]
            let payload = None;

            let manager = sidecar::BootManager::new(
                config_dir.clone(),
                payload,
                log_dir.map(|dir| dir.join("hermes-sidecar.log")),
            );
            app.manage(manager.clone());

            let navigation_app = handle.clone();
            let new_window_app = handle.clone();
            let window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Hermes Tauri")
                    .inner_size(1280.0, 820.0)
                    .min_inner_size(900.0, 600.0)
                    // Polyfills first, so app code and the adapter both see them.
                    .initialization_script(polyfills::script())
                    .initialization_script(adapter::script())
                    .on_navigation(move |url| {
                        let backend_origin =
                            navigation_app.state::<sidecar::BootManager>().origin();
                        // Every main-window navigation is logged with the
                        // token redacted; a same-URL repeat is the reload
                        // proof (review item 7).
                        if backend_origin.as_deref()
                            == Some(url.origin().ascii_serialization().as_str())
                        {
                            sidecar::log(&format!(
                                "Main window navigation: {}",
                                hermes_sidecar::redact(url.as_str())
                            ));
                        }
                        guard::handle_navigation(&navigation_app, url, backend_origin.as_deref())
                    })
                    .on_new_window(move |url, features| {
                        let backend_origin =
                            new_window_app.state::<sidecar::BootManager>().origin();
                        guard::handle_new_window(
                            &new_window_app,
                            url,
                            features,
                            backend_origin.as_deref(),
                        )
                    })
                    .build()?;

            let close_app = handle.clone();
            let close_notified = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if tray::available(&close_app) {
                        // A tray exists: closing hides the window, Quit exits.
                        api.prevent_close();
                        let _ = close_app.get_webview_window("main").map(|w| w.hide());
                        sidecar::log("Window closed to the tray");
                        // A user without a visible tray icon still gets told
                        // once how to get back (relaunching also shows it via
                        // single-instance).
                        if !close_notified.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            if let Err(error) = close_app
                                .notification()
                                .builder()
                                .title("Hermes Tauri")
                                .body("Hermes is still running in the tray.")
                                .show()
                            {
                                sidecar::log(&format!("Could not show the tray notice: {error}"));
                            }
                        }
                    }
                }
            });

            let preferences = config::preferences(config_dir);
            tray::create(&handle);
            hotkey::register(&handle, preferences.hotkey);
            deeplink::init(&handle);
            updater::init(&handle, &preferences.updater);

            let signal_app = handle.clone();
            let signal_manager = manager.clone();
            if let Err(error) = ctrlc::set_handler(move || {
                sidecar::log("Termination signal received");
                // `AppHandle::exit` does not always run the run-loop handlers:
                // when the exit request cannot be queued it exits the process
                // directly, which would orphan a spawned backend. Stop it here
                // too; `shutdown` is idempotent, so the run-loop paths below
                // still work (T16 proof check 08).
                signal_manager.shutdown();
                signal_app.exit(0);
            }) {
                sidecar::log(&format!(
                    "Could not install the termination handler: {error}"
                ));
            }

            manager.prepare(handle);
            Ok(())
        })
        .build(tauri::generate_context!());

    match result {
        Ok(app) => app.run(|app, event| match event {
            // Both paths are idempotent: the backend shuts down exactly once.
            RunEvent::ExitRequested { .. } | RunEvent::Exit => {
                app.state::<sidecar::BootManager>().shutdown();
            }
            _ => {}
        }),
        Err(error) => {
            eprintln!("Could not run Hermes Tauri: {error}");
            std::process::exit(1);
        }
    }
}

mod adapter;
mod config;
mod deeplink;
mod guard;
mod hotkey;
mod native;
mod polyfills;
mod sidecar;
mod tray;
mod updater;

use tauri::{webview::WebviewWindowBuilder, Manager, RunEvent, WebviewUrl, WindowEvent};

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
            if let Some(url) = hermes_shell_config::from_args(&args) {
                deeplink::handle_url(app, url);
            }
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
            let settings = config::load(handle.path().app_config_dir().ok());
            let log_dir = handle.path().app_log_dir().ok();

            app.manage(tray::TrayState::default());
            let manager = sidecar::BootManager::new(
                settings.resolved,
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
                        guard::handle_navigation(&navigation_app, url, backend_origin.as_deref())
                    })
                    .on_new_window(move |url, _features| {
                        let backend_origin =
                            new_window_app.state::<sidecar::BootManager>().origin();
                        guard::handle_new_window(&new_window_app, url, backend_origin.as_deref())
                    })
                    .build()?;

            let close_app = handle.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if tray::available(&close_app) {
                        // A tray exists: closing hides the window, Quit exits.
                        api.prevent_close();
                        let _ = close_app.get_webview_window("main").map(|w| w.hide());
                        sidecar::log("Window closed to the tray");
                    }
                }
            });

            tray::create(&handle);
            hotkey::register(&handle, settings.hotkey);
            deeplink::init(&handle);
            updater::init(&handle, &settings.updater);

            let signal_app = handle.clone();
            if let Err(error) = ctrlc::set_handler(move || {
                sidecar::log("Termination signal received");
                signal_app.exit(0);
            }) {
                sidecar::log(&format!(
                    "Could not install the termination handler: {error}"
                ));
            }

            manager.start(handle);
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

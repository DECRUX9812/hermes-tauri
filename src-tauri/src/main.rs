mod adapter;
mod connect;
mod guard;
mod native;

use tauri::{webview::WebviewWindowBuilder, Manager, WebviewUrl};

fn main() {
    eprintln!("=== MAIN ENTERED ===");
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // WebKitGTK's DMABUF renderer can produce a black window under Xvfb and NVIDIA.
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let launch_config = connect::launch_url_from_args();
    let backend_origin = launch_config
        .as_ref()
        .ok()
        .and_then(Option::as_deref)
        .and_then(|url| connect::LaunchTarget::parse(url).ok())
        .map(|target| target.origin);
    let manager = connect::BootManager::new(launch_config);

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            connect::boot_state,
            connect::boot_retry,
            native::native_open_external,
            native::native_notify
        ])
        .setup(move |app| {
            eprintln!("=== SETUP ENTERED ===");
            app.manage(manager.clone());

            let navigation_app = app.handle().clone();
            let new_window_app = app.handle().clone();
            let navigation_origin = backend_origin.clone();
            let new_window_origin = backend_origin.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Hermes Tauri")
                .inner_size(1280.0, 820.0)
                .min_inner_size(900.0, 600.0)
                .initialization_script(adapter::script_for(backend_origin.as_deref()))
                .on_navigation(move |url| {
                    guard::handle_navigation(&navigation_app, url, navigation_origin.as_deref())
                })
                .on_new_window(move |url, _features| {
                    guard::handle_new_window(&new_window_app, url, new_window_origin.as_deref())
                })
                .build()?;

            manager.start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("Could not run Hermes Tauri: {error}");
        std::process::exit(1);
    }
}

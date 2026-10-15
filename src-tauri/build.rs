fn main() {
    // The updater's signing key and feed come from the build environment
    // (CI sets them); they are never committed. build.rs forwards them to the
    // crate's compilation so `option_env!` in config.rs can read them.
    for key in ["TAURI_UPDATER_PUBKEY", "TAURI_UPDATER_ENDPOINT"] {
        if let Ok(value) = std::env::var(key) {
            println!("cargo:rustc-env={key}={value}");
        }
        println!("cargo:rerun-if-env-changed={key}");
    }

    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "boot_state",
            "boot_retry",
            "native_open_external",
            "native_notify",
            "native_select_paths",
        ]),
    ))
    .expect("failed to run tauri-build");
}

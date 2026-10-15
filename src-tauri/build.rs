fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "boot_state",
            "boot_retry",
            "native_open_external",
            "native_notify",
        ]),
    ))
    .expect("failed to run tauri-build");
}

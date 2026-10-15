use std::thread;
use std::time::Duration;

use tauri::{webview::WebviewWindowBuilder, AppHandle, Manager, WebviewUrl};

pub const LABEL: &str = "splash";

/// How long the splash may cover the main window after the shell has handed
/// the webview the app's URL. The app's first frame takes 6-9 s on this
/// workstation (T12), so this cap is a safety net for a page that never
/// reports one: past it the splash goes away whatever the page does.
pub const MAX_VISIBLE: Duration = Duration::from_secs(30);

/// A second copy of the boot page, stacked above the main window until the
/// remote app paints its first frame. The main webview cannot show the boot
/// page and the remote app at once: navigating to the app replaces the page.
///
/// Idempotent: Retry from a splash that is still up must not fail.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Hermes Tauri")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .center()
        .always_on_top(true)
        .skip_taskbar(true)
        .build()?;
    Ok(())
}

/// Removes the splash if it is still there and says why. Every path that ends
/// the app's load calls this: the first painted frame, a boot failure (the
/// main window shows the same boot page, with Retry) and the cap below. The
/// window is destroyed, so `always_on_top` cannot outlive the load either.
pub fn dismiss(app: &AppHandle, reason: &str) {
    if let Some(window) = app.get_webview_window(LABEL) {
        if window.destroy().is_ok() {
            crate::sidecar::log(&format!("Splash removed: {reason}"));
        }
    }
}

/// Starts the cap for the splash that is on screen now. Called when the shell
/// hands the main webview the app's URL, and only there: a page that never
/// paints its first frame (no `load`, a hung document) would otherwise keep the
/// always-on-top splash in front of the window forever. The handle is the
/// splash this watchdog was armed for, so it cannot hit a later one.
pub fn arm_timeout(app: &AppHandle) {
    let Some(splash) = app.get_webview_window(LABEL) else {
        return;
    };
    let cap = cap();
    thread::spawn(move || {
        thread::sleep(cap);
        if splash.is_visible().unwrap_or(false) && splash.destroy().is_ok() {
            crate::sidecar::log(&format!(
                "Splash removed: the app did not paint within {} ms",
                cap.as_millis()
            ));
        }
    });
}

/// `MAX_VISIBLE`, unless `HERMES_TAURI_SPLASH_CAP_MS` shortens it. The proof
/// uses that knob to watch the removal happen, because the app paints in 6-9 s
/// here and a real cap fire is exactly the case that never occurs.
fn cap() -> Duration {
    cap_from(std::env::var("HERMES_TAURI_SPLASH_CAP_MS").ok().as_deref())
}

/// Pure so it is testable without GTK. A value that is not a number of
/// milliseconds is ignored rather than treated as zero, so a typo cannot take
/// the splash away before the app has had a chance to paint.
fn cap_from(value: Option<&str>) -> Duration {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(MAX_VISIBLE)
}

#[cfg(test)]
mod tests {
    use super::{cap_from, MAX_VISIBLE};
    use std::time::Duration;

    #[test]
    fn the_cap_defaults_and_only_a_real_number_shortens_it() {
        assert_eq!(cap_from(None), MAX_VISIBLE);
        assert_eq!(cap_from(Some("3000")), Duration::from_millis(3000));
        assert_eq!(cap_from(Some("0")), Duration::ZERO);
        // A typo keeps the safety net instead of removing the splash at once.
        assert_eq!(cap_from(Some("3s")), MAX_VISIBLE);
        assert_eq!(cap_from(Some("")), MAX_VISIBLE);
        assert_eq!(cap_from(Some("-1")), MAX_VISIBLE);
    }
}

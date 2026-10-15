use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, MutexGuard,
};
use std::thread;
use std::time::Duration;

use hermes_shell_config::Resolved;
use hermes_sidecar::{redact, Backend, Phase, SidecarConfig};
use serde::Serialize;
use tauri::{ipc::CapabilityBuilder, AppHandle, Emitter, Manager, State, Url};

const PENDING_ROUTE_POLL: Duration = Duration::from_millis(250);
const PENDING_ROUTE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long shutdown waits for a cancelled attempt to finish killing the
/// spawned process group. The attempt polls the cancel flag every 50 ms and
/// the health probe at most every 1 s (HEALTH_PROBE_TIMEOUT in
/// hermes-sidecar), so 5 s comfortably bounds one probe plus the SIGKILL and
/// reap of the group.
const ATTEMPT_STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BootPhase {
    NoConfig,
    Probing,
    Starting,
    Waiting,
    Ready,
    Unreachable,
}

#[derive(Clone, Serialize)]
pub struct BootStatus {
    pub phase: BootPhase,
    pub origin: Option<String>,
    pub error: Option<String>,
}

impl BootStatus {
    fn no_config() -> Self {
        Self {
            phase: BootPhase::NoConfig,
            origin: None,
            error: None,
        }
    }

    fn probing(origin: Option<String>) -> Self {
        Self {
            phase: BootPhase::Probing,
            origin,
            error: None,
        }
    }

    fn starting() -> Self {
        Self {
            phase: BootPhase::Starting,
            origin: None,
            error: None,
        }
    }

    fn waiting(origin: Option<String>) -> Self {
        Self {
            phase: BootPhase::Waiting,
            origin,
            error: None,
        }
    }

    fn ready(origin: String) -> Self {
        Self {
            phase: BootPhase::Ready,
            origin: Some(origin),
            error: None,
        }
    }

    fn unreachable(origin: Option<String>, error: impl Into<String>) -> Self {
        Self {
            phase: BootPhase::Unreachable,
            origin,
            error: Some(error.into()),
        }
    }
}

fn status_from_phase(phase: &Phase) -> BootStatus {
    match phase {
        Phase::Probing { origin } => BootStatus::probing(Some(origin.clone())),
        Phase::Spawning { .. } | Phase::WaitingForLaunchLink => BootStatus::starting(),
        Phase::WaitingForHealth { origin } => BootStatus::waiting(Some(origin.clone())),
        Phase::Ready { origin } => BootStatus::ready(origin.clone()),
    }
}

struct Inner {
    status: BootStatus,
    backend: Option<Backend>,
    origin: Option<String>,
    /// A deep-link hash route seen before the backend was ready.
    pending_route: Option<String>,
    shutdown: bool,
    /// Stop signal for the connection attempt in flight (fresh per attempt).
    cancel: hermes_sidecar::Cancel,
    /// Signalled by the attempt thread once it has finished (and the spawned
    /// process group, if any, has been killed) after a shutdown cancel.
    attempt_done_rx: Option<mpsc::Receiver<()>>,
    /// True from the moment the launch URL (which carries the session token
    /// in its fragment) is navigated to until the page has consumed the
    /// token. Deep-link routes wait for it (review F1).
    launching: bool,
}

/// Runs the sidecar and owns the connected `Backend` until exit.
#[derive(Clone)]
pub struct BootManager {
    inner: Arc<Mutex<Inner>>,
    generation: Arc<AtomicU64>,
    resolved: Arc<Result<Resolved, String>>,
    log_path: Arc<Option<PathBuf>>,
    capability_origin: Arc<Mutex<Option<String>>>,
}

impl BootManager {
    pub fn new(resolved: Result<Resolved, String>, log_path: Option<PathBuf>) -> Self {
        let initial = match &resolved {
            Err(error) => BootStatus::unreachable(None, error.clone()),
            Ok(Resolved::NotConfigured) => BootStatus::no_config(),
            Ok(Resolved::Configured { .. }) => BootStatus::probing(None),
        };
        Self {
            inner: Arc::new(Mutex::new(Inner {
                status: initial,
                backend: None,
                origin: None,
                pending_route: None,
                shutdown: false,
                cancel: hermes_sidecar::Cancel::new(),
                attempt_done_rx: None,
                launching: false,
            })),
            generation: Arc::new(AtomicU64::new(0)),
            resolved: Arc::new(resolved),
            log_path: Arc::new(log_path),
            capability_origin: Arc::new(Mutex::new(None)),
        }
    }

    /// Starts the first connection attempt. Called once from setup.
    pub fn start(&self, app: AppHandle) {
        let config = match self.resolved.as_ref() {
            Ok(Resolved::Configured { config, .. }) => config.clone(),
            Ok(Resolved::NotConfigured) | Err(_) => return,
        };
        let attempt = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (done_tx, done_rx) = mpsc::channel();
        lock(&self.inner).attempt_done_rx = Some(done_rx);
        let manager = self.clone();
        thread::spawn(move || {
            manager.connect(app, attempt, config);
            let _ = done_tx.send(());
        });
    }

    /// Runs the connection again from the boot page. When a previous attempt
    /// left a stored backend behind an unreachable status (a capability or
    /// navigation failure after connect), it is dropped so this attempt is
    /// real.
    pub fn retry(&self, app: AppHandle) -> Result<(), String> {
        let mut inner = lock(&self.inner);
        if inner.shutdown {
            return Err("The shell is shutting down".to_owned());
        }
        // At most one attempt runs at a time: an attempt still in flight is
        // cancelled and awaited (bounded) before this one starts, so
        // shutdown() can never return while an older attempt still runs
        // (review F2). When no attempt is in flight the receive returns at
        // once (the sender is already dropped).
        let previous_cancel = inner.cancel.clone();
        let previous_done = inner.attempt_done_rx.take();
        drop(inner);
        previous_cancel.cancel();
        if let Some(done) = previous_done {
            if done.recv_timeout(ATTEMPT_STOP_TIMEOUT) == Err(RecvTimeoutError::Timeout) {
                log("Warning: the previous connection attempt did not finish in time; a spawned backend may have been left running");
            }
        }
        let mut inner = lock(&self.inner);
        // Quit may have raced the wait above: never spawn a fresh attempt
        // during shutdown, or its child could survive the exit (review N3).
        if inner.shutdown {
            return Err("The shell is shutting down".to_owned());
        }
        if inner.backend.is_some() {
            if inner.status.phase != BootPhase::Unreachable {
                return Ok(());
            }
            // A backend whose post-connect work failed is stopped (an
            // attached one is left alone) before reconnecting.
            if let Some(backend) = inner.backend.take() {
                inner.origin = None;
                drop(inner);
                self.stop_backend(backend);
                inner = lock(&self.inner);
            }
        }
        let config = match self.resolved.as_ref() {
            Ok(Resolved::Configured { config, .. }) => config.clone(),
            Ok(Resolved::NotConfigured) => {
                return Err("No Hermes is configured".to_owned());
            }
            Err(error) => return Err(error.clone()),
        };
        let attempt = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        inner.status = BootStatus::probing(None);
        inner.cancel = hermes_sidecar::Cancel::new();
        inner.launching = false;
        let (done_tx, done_rx) = mpsc::channel();
        inner.attempt_done_rx = Some(done_rx);
        drop(inner);

        let manager = self.clone();
        thread::spawn(move || {
            manager.connect(app, attempt, config);
            let _ = done_tx.send(());
        });
        Ok(())
    }

    /// Applies a whitelisted deep-link route: immediately when the backend is
    /// ready and the launch page has consumed its token, otherwise remembered
    /// and applied once it has. The check and the store share one lock with
    /// `ready`, so a route seen while connecting is never stranded.
    pub fn apply_route_or_defer(&self, app: &AppHandle, route: &str) -> bool {
        let mut inner = lock(&self.inner);
        // While the launch page may still hold the session token in its
        // fragment, every route goes through the deferred path, which waits
        // until the token is consumed. Applying one immediately would
        // navigate away from the token-bearing launch page and lose the
        // session (review F1).
        if let Some(origin) = immediate_route_origin(inner.origin.as_deref(), inner.launching) {
            drop(inner);
            self.apply_route(app, &origin, route);
            return true;
        }
        if inner.shutdown {
            return false;
        }
        inner.pending_route = Some(route.to_owned());
        false
    }

    pub fn origin(&self) -> Option<String> {
        lock(&self.inner).origin.clone()
    }

    /// Stops the backend exactly once. Runs on the exit paths (window close
    /// without a tray, tray Quit, Ctrl+C/SIGTERM). Attached backends are left
    /// alone; a backend still connecting is cancelled and its spawned process
    /// group killed. Blocks while a process group is stopping.
    pub fn shutdown(&self) {
        let (backend, attempt_done) = {
            let mut inner = lock(&self.inner);
            if inner.shutdown {
                return;
            }
            inner.shutdown = true;
            inner.pending_route = None;
            inner.cancel.cancel();
            (inner.backend.take(), inner.attempt_done_rx.take())
        };
        // The attempt thread observes the cancel within one poll interval and
        // drops the spawned process group, which kills it. Wait for that
        // before the process exits, or the child is orphaned (review H1).
        if let Some(done) = attempt_done {
            if done.recv_timeout(ATTEMPT_STOP_TIMEOUT) == Err(RecvTimeoutError::Timeout) {
                log("Warning: the connection attempt did not finish in time; a spawned backend may have been left running");
            }
        }
        let Some(backend) = backend else {
            log("Hermes backend stopped (nothing was connected)");
            return;
        };
        self.stop_backend(backend);
    }

    fn stop_backend(&self, backend: Backend) {
        match backend.shutdown() {
            Ok(report) => log(&format!("Hermes backend stopped: {report:?}")),
            Err(error) => log(&format!("Error while stopping Hermes: {error}")),
        }
    }

    fn connect(self, app: AppHandle, attempt: u64, mut config: SidecarConfig) {
        if config.command.is_empty() {
            config.spawn_if_unreachable = false;
        }
        if let Some(path) = self.log_path.as_ref() {
            config.log_path = Some(path.clone());
        }
        // Mode, program and argument count only: the argv itself can carry
        // secrets and must not reach the log.
        let mode = if config.attach_url.is_some() {
            "attach"
        } else {
            "spawn"
        };
        let program = config
            .command
            .first()
            .map(String::as_str)
            .unwrap_or("(none)");
        log(&format!(
            "Connecting to Hermes (mode {mode}, program {program:?}, {} arguments)",
            config.command.len().saturating_sub(1)
        ));

        let cancel = lock(&self.inner).cancel.clone();
        let manager = self.clone();
        let phase_app = app.clone();
        let result = hermes_sidecar::connect_with_cancel(
            &config,
            |phase| {
                manager.on_phase(phase_app.clone(), attempt, &phase);
            },
            &cancel,
        );

        match result {
            Ok(backend) => self.ready(app, attempt, backend),
            Err(error) => {
                let message = redact(&error.to_string());
                log(&format!("Hermes connection failed: {message}"));
                if self.is_current(attempt) && !self.is_shutting_down() {
                    self.set_status(&app, BootStatus::unreachable(None, message));
                }
            }
        }
    }

    fn on_phase(&self, app: AppHandle, attempt: u64, phase: &Phase) {
        if !self.is_current(attempt) || self.is_shutting_down() {
            return;
        }
        self.set_status(&app, status_from_phase(phase));
    }

    fn ready(&self, app: AppHandle, attempt: u64, backend: Backend) {
        if !self.is_current(attempt) || self.is_shutting_down() {
            // A superseded attempt or an exit never adopts a backend: an
            // attached one is a no-op, a spawned one is killed.
            self.stop_backend(backend);
            return;
        }

        // Post-connect work runs BEFORE the backend is stored: when it fails,
        // no backend is kept and Retry reconnects (M3).
        let launch_url = backend.launch_url().to_owned();
        let origin = backend.origin().to_owned();
        self.set_status(&app, BootStatus::ready(origin.clone()));
        log(&format!("Hermes backend ready at {origin}"));
        if let Err(error) = self.add_remote_capability(&app, &origin) {
            self.set_status(&app, BootStatus::unreachable(Some(origin), error));
            self.stop_backend(backend);
            return;
        }
        let url = match Url::parse(&launch_url) {
            Ok(url) => url,
            Err(error) => {
                self.set_status(
                    &app,
                    BootStatus::unreachable(
                        Some(origin),
                        format!("Invalid Hermes launch link: {error}"),
                    ),
                );
                self.stop_backend(backend);
                return;
            }
        };

        {
            let mut inner = lock(&self.inner);
            if inner.shutdown || self.generation.load(Ordering::SeqCst) != attempt {
                drop(inner);
                self.stop_backend(backend);
                return;
            }
            inner.backend = Some(backend);
            inner.origin = Some(origin.clone());
            inner.launching = true;
        }
        self.navigate_main(&app, url, None);

        // Watch the launch page until the token is consumed, then apply a
        // deep link seen in the meantime (F1). Always spawned, so `launching`
        // is cleared even when no route is pending. The route is drained
        // under the lock at consumption time, not captured here, so a link
        // arriving after the spawn is not stranded (review N2).
        let manager = self.clone();
        thread::spawn(move || manager.apply_pending_route(app, origin));
    }

    /// Waits for the main webview to finish loading the launch URL, then
    /// applies the pending deep-link route.
    ///
    /// Race-free because the route is only applied once the URL fragment no
    /// longer carries `#hermes-session=`: the page's bootstrap consumes that
    /// fragment into web-local storage and then strips it with
    /// `history.replaceState` (`browser-launch-session.ts`), so a
    /// fragment-free URL proves the token has already been read. The route is
    /// a same-document hash change on top and cannot race the token read.
    /// (Waiting for the origin alone was not enough: the navigation commits
    /// before the page's scripts run.)
    fn apply_pending_route(self, app: AppHandle, origin: String) {
        let deadline = std::time::Instant::now() + PENDING_ROUTE_TIMEOUT;
        loop {
            if self.is_shutting_down() {
                return;
            }
            let loaded = app
                .get_webview_window("main")
                .and_then(|window| window.url().ok())
                .is_some_and(|url| {
                    url.origin().ascii_serialization() == origin
                        && !url
                            .fragment()
                            .unwrap_or_default()
                            .contains("hermes-session=")
                });
            if loaded {
                let mut inner = lock(&self.inner);
                if inner.origin.as_deref() != Some(origin.as_str()) {
                    // A superseded attempt's watcher: leave `launching` and
                    // the pending route to the current attempt.
                    return;
                }
                inner.launching = false;
                // Drain under the lock: a link may have arrived after this
                // watcher was spawned (review N2).
                let route = inner.pending_route.take();
                drop(inner);
                if let Some(route) = route {
                    self.apply_route(&app, &origin, &route);
                }
                return;
            }
            if std::time::Instant::now() >= deadline {
                let mut inner = lock(&self.inner);
                if inner.origin.as_deref() == Some(origin.as_str()) {
                    inner.launching = false;
                }
                let route = inner.pending_route.take();
                drop(inner);
                if route.is_some() {
                    log("Ignored a deep link: Hermes did not finish loading within 30 s");
                }
                return;
            }
            thread::sleep(PENDING_ROUTE_POLL);
        }
    }

    /// Navigates the main webview to `<origin>/<hash>`, for example
    /// `http://127.0.0.1:8080/#/team`.
    pub fn apply_route(&self, app: &AppHandle, origin: &str, route: &str) {
        let url = format!("{origin}/{route}");
        match Url::parse(&url) {
            Ok(url) => self.navigate_main(app, url.clone(), Some(url.as_str())),
            Err(error) => log(&format!("Invalid navigation target {url:?}: {error}")),
        }
    }

    fn navigate_main(&self, app: &AppHandle, url: Url, label: Option<&str>) {
        let Some(window) = app.get_webview_window("main") else {
            self.set_status(
                app,
                BootStatus::unreachable(None, "The main Hermes window is not available".to_owned()),
            );
            return;
        };
        if let Err(error) = window.navigate(url.clone()) {
            self.set_status(
                app,
                BootStatus::unreachable(
                    window.url().ok().map(|current| current.to_string()),
                    format!("Could not open Hermes: {error}"),
                ),
            );
        } else if let Some(label) = label {
            log(&format!("Navigated to {label}"));
        }
    }

    fn add_remote_capability(&self, app: &AppHandle, origin: &str) -> Result<(), String> {
        let mut added = lock(&self.capability_origin);
        if added.as_deref() == Some(origin) {
            return Ok(());
        }

        // The runtime capability is the IPC boundary: scoped to the exact
        // backend origin, on the main webview only, and limited to the three
        // native commands the adapter overlays. The plugins behind them are
        // called from Rust; the frontend gets no plugin permissions.
        let capability = CapabilityBuilder::new("hermes-backend")
            .remote(format!("{origin}/*"))
            .window("main")
            .permission("allow-native-open-external")
            .permission("allow-native-notify")
            .permission("allow-native-select-paths");
        app.add_capability(capability)
            .map_err(|error| format!("Could not grant Hermes native capabilities: {error}"))?;
        *added = Some(origin.to_owned());
        Ok(())
    }

    fn is_current(&self, attempt: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == attempt
    }

    fn is_shutting_down(&self) -> bool {
        lock(&self.inner).shutdown
    }

    fn set_status(&self, app: &AppHandle, status: BootStatus) {
        {
            let mut inner = lock(&self.inner);
            if inner.shutdown {
                return;
            }
            inner.status = status.clone();
        }
        let _ = app.emit("boot-state", status);
    }

    pub fn status(&self) -> BootStatus {
        lock(&self.inner).status.clone()
    }
}

#[tauri::command]
pub fn boot_state(manager: State<'_, BootManager>) -> BootStatus {
    manager.status()
}

#[tauri::command]
pub fn boot_retry(manager: State<'_, BootManager>, app: AppHandle) -> Result<(), String> {
    manager.retry(app)
}

pub fn log(message: &str) {
    eprintln!("[hermes-tauri] {message}");
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether a deep-link route arriving now can be applied at once, and on
/// which origin: only when the backend origin is known and the launch page
/// has consumed its session token. Pure so it is testable without GTK.
fn immediate_route_origin(origin: Option<&str>, launching: bool) -> Option<String> {
    if launching {
        return None;
    }
    origin.map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::immediate_route_origin;

    const ORIGIN: &str = "http://127.0.0.1:18810";

    #[test]
    fn a_route_is_immediate_only_after_the_token_is_consumed() {
        // No backend origin yet: deferred.
        assert_eq!(immediate_route_origin(None, false), None);
        assert_eq!(immediate_route_origin(None, true), None);
        // Origin known but the launch page may still carry the token (F1):
        // deferred.
        assert_eq!(immediate_route_origin(Some(ORIGIN), true), None);
        // Origin known, token consumed: immediate.
        assert_eq!(
            immediate_route_origin(Some(ORIGIN), false),
            Some(ORIGIN.to_owned())
        );
    }
}

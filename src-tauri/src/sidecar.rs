use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, MutexGuard,
};
use std::thread;
use std::time::Duration;

use hermes_shell_config::{Resolved, Route};
use hermes_sidecar::{redact, Backend, Phase, SidecarConfig};
use serde::Serialize;
use tauri::{ipc::CapabilityBuilder, AppHandle, Emitter, Manager, State, Url};

use crate::config;
#[cfg(unix)]
use crate::payload::{self, PayloadSource};

/// The payload to install. Windows ships none (the payload's launcher is a
/// POSIX `sh` script, docs/python-sidecar.md §5.4), so there the value is a
/// single compile-time `None` and the `payload` module does not exist.
#[cfg(not(unix))]
type PayloadSource = ();

const PENDING_ROUTE_POLL: Duration = Duration::from_millis(250);
const PENDING_ROUTE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long shutdown waits for a cancelled attempt to finish killing the
/// spawned process group. The attempt polls the cancel flag every 50 ms and
/// the health probe at most every 1 s (HEALTH_PROBE_TIMEOUT in
/// hermes-sidecar), so 5 s comfortably bounds one probe plus the SIGKILL and
/// reap of the group.
const ATTEMPT_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// The commands the *remote* backend page may call through IPC, and the only
/// grant of `allow-app-painted` in the app: a local page (the boot page) must
/// never reach any of them. Granted by [`BootManager::add_remote_capability`]
/// as a runtime capability scoped to the backend origin on the main window
/// only; `capabilities/boot-local.json` covers the boot page's own two
/// commands. `tests::the_boot_page_cannot_call_native_commands` pins the
/// separation between the two.
const REMOTE_PERMISSIONS: [&str; 4] = [
    "allow-native-open-external",
    "allow-native-notify",
    "allow-native-select-paths",
    "allow-app-painted",
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BootPhase {
    NoConfig,
    /// The bundled payload is being unpacked (a first run, before any
    /// connection attempt is possible).
    Preparing,
    Probing,
    Starting,
    Waiting,
    Ready,
    Unreachable,
}

impl BootPhase {
    /// A phase the shell cannot leave on its own. The main window shows the
    /// same boot page in one of these, so the splash has nothing left to
    /// cover and must go: it is always on top, and its Retry reaches the same
    /// boot manager.
    fn is_failure(self) -> bool {
        matches!(self, BootPhase::NoConfig | BootPhase::Unreachable)
    }
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

    /// The bundled payload is being installed; the boot page explains the wait
    /// instead of showing an empty window.
    fn preparing() -> Self {
        Self {
            phase: BootPhase::Preparing,
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
    /// True from the moment the launch URL (which carries the session
    /// credential, as a `#hermes-session=` fragment or a `?token=` query) is
    /// navigated to until the page has consumed it. Deep-link routes wait for
    /// it (review F1).
    launching: bool,
}

/// Runs the sidecar and owns the connected `Backend` until exit.
#[derive(Clone)]
pub struct BootManager {
    inner: Arc<Mutex<Inner>>,
    generation: Arc<AtomicU64>,
    /// The app config directory, where `config.toml` is read from. Resolution
    /// happens per attempt (see [`BootManager::resolve`]), not here.
    config_dir: Arc<Option<PathBuf>>,
    /// The bundled payload this build ships, when it ships one.
    payload: Arc<Option<PayloadSource>>,
    log_path: Arc<Option<PathBuf>>,
    capability_origin: Arc<Mutex<Option<String>>>,
}

impl BootManager {
    /// `payload` is the archive a payload build ships in its resource dir
    /// (T16b), or `None` for every other build: the shell then resolves its
    /// backend from the environment, `config.toml` and PATH exactly as before.
    ///
    /// A payload is only *unpacked* in [`Self::prepare`], on a spawned thread;
    /// until that finishes the status is `Preparing`, which is why this
    /// constructor cannot know yet whether a backend will be found.
    pub fn new(
        config_dir: Option<PathBuf>,
        payload: Option<PayloadSource>,
        log_path: Option<PathBuf>,
    ) -> Self {
        let initial = start_status(payload.is_some());
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
            config_dir: Arc::new(config_dir),
            payload: Arc::new(payload),
            log_path: Arc::new(log_path),
            capability_origin: Arc::new(Mutex::new(None)),
        }
    }

    /// Starts the first connection attempt. Called once from setup, which
    /// returns before this finishes: a first run unpacks the bundled payload
    /// (seconds) and that must not hold up the boot page.
    ///
    /// Resolution runs on the attempt thread, so a missing configuration or a
    /// failed payload install is published as a status (NoConfig/Unreachable)
    /// rather than returned here; `set_status` dismisses the splash for those
    /// failure phases, which is the only path that does.
    pub fn prepare(&self, app: AppHandle) {
        self.spawn_attempt(app);
    }

    /// Runs the connection again from the boot page: the source is resolved
    /// from scratch (so a failed payload install is retried, and a changed
    /// `config.toml` is picked up) and a previous attempt is cancelled and
    /// awaited first. When a previous attempt left a stored backend behind an
    /// unreachable status (a capability or navigation failure after connect),
    /// it is dropped so this attempt is real.
    ///
    /// Returns before the attempt has decided anything: resolution and connect
    /// both run on the attempt thread and publish their result as a status.
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
        inner.status = start_status(self.payload.is_some());
        inner.cancel = hermes_sidecar::Cancel::new();
        inner.launching = false;
        drop(inner);

        // A retry loads a fresh copy of the remote app, so the splash goes back
        // up before the attempt starts: the boot page must not be replaced by a
        // blank window while the app's bundle executes, exactly as on the first
        // start. Creating it before the attempt means a failure that follows
        // can only dismiss a splash that exists. The attempt resolves its own
        // source and publishes a NoConfig/Unreachable status through
        // `set_status` when it cannot run, which is what dismisses it again.
        if let Err(error) = crate::splash::create(&app) {
            log(&format!("Could not put the splash back: {error}"));
        }

        self.spawn_attempt(app);
        Ok(())
    }

    /// One connection attempt on its own thread: resolve the source first (a
    /// first run unpacks the bundled payload), then connect. The thread signals
    /// `attempt_done_rx` when it is done, which is what shutdown and a retry
    /// wait for.
    fn spawn_attempt(&self, app: AppHandle) {
        let attempt = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (done_tx, done_rx) = mpsc::channel();
        lock(&self.inner).attempt_done_rx = Some(done_rx);
        let manager = self.clone();
        thread::spawn(move || {
            if let Some(config) = manager.resolve(&app) {
                manager.connect(app, attempt, config);
            }
            let _ = done_tx.send(());
        });
    }

    /// Works out where the backend comes from, installing the bundled payload
    /// when this is a first run. Runs on the attempt thread, so unpacking (a
    /// few seconds) never blocks setup or the webview. Returns the config to
    /// connect with, or `None` after publishing the status that says why there
    /// is nothing to connect.
    fn resolve(&self, app: &AppHandle) -> Option<SidecarConfig> {
        let resolved = match &*self.payload {
            #[cfg(unix)]
            Some(payload) => {
                log("Preparing the bundled Hermes payload (a first run unpacks it)");
                match payload::unpack(payload) {
                    Ok(bundled) => config::load((*self.config_dir).clone(), Some(&bundled)),
                    Err(error) => Err(error),
                }
            }
            _ => config::load((*self.config_dir).clone(), None),
        };
        match resolved {
            Ok(Resolved::Configured { config, source }) => {
                log(&format!(
                    "Hermes backend source: {}",
                    config::source_label(&source)
                ));
                Some(config)
            }
            Ok(Resolved::NotConfigured) => {
                self.set_status(app, BootStatus::no_config());
                None
            }
            Err(error) => {
                log(&format!("Could not configure Hermes: {error}"));
                self.set_status(app, BootStatus::unreachable(None, error));
                None
            }
        }
    }

    /// Applies a whitelisted deep-link route: immediately when the backend is
    /// ready and the launch page has consumed its token, otherwise remembered
    /// and applied once it has. The check and the store share one lock with
    /// `ready`, so a route seen while connecting is never stranded.
    pub fn apply_route_or_defer(&self, app: &AppHandle, route: &str) -> bool {
        let mut inner = lock(&self.inner);
        // While the launch page may still hold the session token, every route
        // goes through the deferred path, which waits until the token is
        // consumed. Applying one immediately would navigate away from the
        // token-bearing launch page and lose the session (review F1).
        if let Some(origin) = immediate_route_origin(inner.origin.as_deref(), inner.launching) {
            drop(inner);
            self.apply_route(app, &origin, route);
            return true;
        }
        if inner.shutdown {
            return false;
        }
        inner.pending_route = Some(deferred_route(inner.pending_route.as_deref(), route));
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
        // The splash is on screen and the app is about to load in the main
        // window: from here the cap runs, so a page that never paints its first
        // frame cannot keep the splash up forever.
        crate::splash::arm_timeout(&app);
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
    /// Race-free because the route is only applied once the launch URL no
    /// longer carries a credential, whatever shape it had: the stock webapp
    /// puts the session token in the `#hermes-session=` fragment and its
    /// bootstrap stores it and strips the fragment with `history.replaceState`
    /// (`browser-launch-session.ts`), while the bundled front door takes
    /// `?token=` on `/`, answers it with a 303 to a clean `/` *and* the
    /// session cookie (`hermes_plus/server/auth.py`), so a credential-free URL
    /// proves the session is already established. The route is a same-document
    /// hash change on top and cannot race that read. (Waiting for the origin
    /// alone was not enough: the navigation commits before the page's scripts
    /// run, and in bundled mode the origin alone never proved consumption.)
    fn apply_pending_route(self, app: AppHandle, origin: String) {
        let deadline = std::time::Instant::now() + PENDING_ROUTE_TIMEOUT;
        loop {
            if self.is_shutting_down() {
                return;
            }
            let loaded = app
                .get_webview_window("main")
                .and_then(|window| window.url().ok())
                .is_some_and(|url| launch_consumed(&url, &origin));
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
        // backend origin, on the main webview only, and limited to the native
        // commands the adapter overlays (plus the first-frame signal). The
        // plugins behind them are called from Rust; the frontend gets no plugin
        // permissions.
        let mut capability = CapabilityBuilder::new("hermes-backend")
            .remote(format!("{origin}/*"))
            .window("main");
        for permission in REMOTE_PERMISSIONS {
            capability = capability.permission(permission);
        }
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
        if status.phase.is_failure() {
            // The main window shows the same boot page (with Retry), so the
            // splash must not stay in front of it.
            crate::splash::dismiss(
                app,
                status.error.as_deref().unwrap_or("the start-up failed"),
            );
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

/// Async on purpose: `retry` puts the splash back up, and Tauri documents that
/// creating a window from a *synchronous* command deadlocks on Windows
/// (WebView2, tauri-apps/wry#583); its fix is "use `async` commands and
/// separate threads when creating windows" (tauri 2.12.4,
/// `window/mod.rs` / `webview/webview_window.rs`, "Known issues"). An async
/// command runs on Tauri's runtime instead of the main thread, so the window
/// creation is dispatched to the event loop as usual. The 5 s at most that
/// `retry` waits for a superseded attempt blocks one runtime worker, not the
/// UI thread.
#[tauri::command]
pub async fn boot_retry(manager: State<'_, BootManager>, app: AppHandle) -> Result<(), String> {
    manager.retry(app)
}

/// Called by the backend page once its first frame is on screen (adapter.js).
/// Takes no arguments and only removes the splash; the capability that reaches
/// it is scoped to the backend origin on the main window.
#[tauri::command]
pub fn app_painted(app: AppHandle) {
    log("First frame of the Hermes app painted");
    crate::splash::dismiss(&app, "the app painted its first frame");
}

pub fn log(message: &str) {
    eprintln!("[hermes-tauri] {message}");
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The status an attempt starts from: `Preparing` while a bundled payload
/// still has to be unpacked, otherwise the ordinary probe of the configured
/// source (which the attempt thread resolves at once). Pure so it is testable
/// without GTK.
fn start_status(has_payload: bool) -> BootStatus {
    if has_payload {
        BootStatus::preparing()
    } else {
        BootStatus::probing(None)
    }
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

/// Whether the webview has committed a launch URL that no longer carries a
/// credential, i.e. whether the session is established and navigating away
/// from the page cannot lose it.
///
/// Both credential shapes count: the stock webapp's `#hermes-session=<token>`
/// fragment and the bundled front door's `?token=<token>` on `/`. The front
/// door consumes the query by answering it with a 303 to `/` and the session
/// cookie in the same response (`hermes_plus/server/auth.py`), so `true` here
/// means the cookie is already stored — independent of what `get_uri()`
/// reported while the redirect was in flight.
///
/// Known residual gap: with `auth = "password"` the front door serves a
/// token-less `/login` and this predicate is true at once, so a deep link
/// arriving then can preempt the login page (the route is lost, the session is
/// not). The bundled payload defaults to `token` (`hermes_plus/server/settings.py`).
fn launch_consumed(url: &Url, origin: &str) -> bool {
    if url.origin().ascii_serialization() != origin {
        return false;
    }
    if url
        .fragment()
        .unwrap_or_default()
        .contains("hermes-session=")
    {
        return false;
    }
    // `token` as a whole parameter only: `xtoken=1` is a different parameter
    // and must not keep the shell waiting.
    !url.query()
        .unwrap_or_default()
        .split('&')
        .any(|part| part == "token" || part.starts_with("token="))
}

/// The route to keep waiting for after `route` arrives while the launch page
/// still holds the token. Only one route can wait, and the tray's "New chat"
/// (`#/`) is the weaker request: it must not overwrite a deep link the user
/// followed (review L1). Every other route replaces the waiting one, so the
/// user's latest navigation wins.
fn deferred_route(pending: Option<&str>, route: &str) -> String {
    match pending {
        Some(waiting) if route == Route::Home.hash() => waiting.to_owned(),
        _ => route.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        deferred_route, immediate_route_origin, launch_consumed, start_status, BootPhase,
        REMOTE_PERMISSIONS,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use tauri::Url;

    const ORIGIN: &str = "http://127.0.0.1:18810";
    /// A front-door token: 43 url-safe characters (`secrets.token_urlsafe(32)`).
    const TOKEN: &str = "Yk3QpZ1sV0dJ7rNfT2xWcLmB6hGqAeUiO4yRtS8nZvP";

    fn url(path: &str) -> Url {
        Url::parse(&format!("{ORIGIN}{path}")).expect("test URL")
    }

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

    /// The splash is always on top, so it may only outlive the app's load while
    /// the shell is still trying. A failure phase leaves the main window
    /// showing the same boot page, and the phases in between are the ones the
    /// splash exists to cover: `Preparing` (a first run unpacking the bundled
    /// payload) is one of them, and the 30 s cap only arms in `ready`, so a
    /// long unpack keeps the splash without the cap firing.
    #[test]
    fn only_a_failure_phase_dismisses_the_splash() {
        for phase in [BootPhase::NoConfig, BootPhase::Unreachable] {
            assert!(phase.is_failure(), "{phase:?} must dismiss the splash");
        }
        for phase in [
            BootPhase::Preparing,
            BootPhase::Probing,
            BootPhase::Starting,
            BootPhase::Waiting,
            BootPhase::Ready,
        ] {
            assert!(!phase.is_failure(), "{phase:?} must keep the splash");
        }
    }

    #[test]
    fn a_launch_url_with_a_credential_is_not_consumed() {
        // The front door's login URL: the token is still in the query, so the
        // 303 (and with it the session cookie) has not happened yet.
        assert!(!launch_consumed(&url(&format!("/?token={TOKEN}")), ORIGIN));
        // The stock webapp's fragment shape, before `replaceState` strips it.
        assert!(!launch_consumed(
            &url(&format!("/#hermes-session={TOKEN}")),
            ORIGIN
        ));
        // A bare `token` parameter counts as well.
        assert!(!launch_consumed(&url("/?token"), ORIGIN));
        // A parameter that merely ends in `token` does not.
        assert!(launch_consumed(&url("/?xtoken=1"), ORIGIN));
        // Nor does a token somewhere other than the query.
        assert!(launch_consumed(&url("/?q=token"), ORIGIN));
    }

    #[test]
    fn a_credential_free_launch_url_is_consumed() {
        // The front door's post-303 URL and the SPA's own hash routes.
        assert!(launch_consumed(&url("/"), ORIGIN));
        assert!(launch_consumed(&url("/#/team"), ORIGIN));
        assert!(launch_consumed(&url("/?a=1&b=2"), ORIGIN));
        // Another origin is never the launch page, whatever it carries.
        assert!(!launch_consumed(&url("/"), "http://127.0.0.1:18811"));
    }

    #[test]
    fn a_route_arriving_before_the_front_door_consumed_the_token_is_deferred() {
        // Regression for the M1/F1 race in bundled mode: the front door's URL
        // is `/?token=<43>` while the redirect is in flight. The watcher's
        // predicate is false there, so it never clears `launching`, so a deep
        // link arriving in that window takes the deferred path instead of
        // navigating away from the token-bearing page.
        let login = url(&format!("/?token={TOKEN}"));
        assert!(!launch_consumed(&login, ORIGIN));
        assert_eq!(immediate_route_origin(Some(ORIGIN), true), None);

        // Once the 303 has been followed the same URL is credential-free and
        // the deferred route is applied.
        let consumed = url("/");
        assert!(launch_consumed(&consumed, ORIGIN));
        assert_eq!(
            immediate_route_origin(Some(ORIGIN), false),
            Some(ORIGIN.to_owned())
        );
    }

    #[test]
    fn the_tray_home_route_does_not_overwrite_a_waiting_deep_link() {
        // A deep link that arrived while the backend was still starting stays
        // the pending route when the tray's New chat asks for `#/` (review L1).
        assert_eq!(deferred_route(Some("#/team"), "#/"), "#/team");
        assert_eq!(deferred_route(Some("#/studio"), "#/"), "#/studio");
        // `#/` on its own still waits, and a later link still wins.
        assert_eq!(deferred_route(None, "#/"), "#/");
        assert_eq!(deferred_route(Some("#/team"), "#/studio"), "#/studio");
        assert_eq!(deferred_route(Some("#/"), "#/team"), "#/team");
    }

    #[test]
    fn a_payload_build_starts_on_the_preparing_phase() {
        // The unpack happens before any connection attempt, so the boot page
        // must say "preparing" rather than "probing".
        assert_eq!(start_status(true).phase, BootPhase::Preparing);
        assert_eq!(start_status(false).phase, BootPhase::Probing);
    }

    /// The boot page is a local page, so its capability is what stands between
    /// it and the native commands the backend page may call (review R8 item 4).
    /// The invariant, not the file: the local boot capability carries the boot
    /// page's own two commands plus the event listener, and none of the remote
    /// grants — while `app_painted`, which removes the splash, is granted only
    /// by the runtime remote capability, scoped to the backend origin.
    #[test]
    fn the_boot_page_cannot_call_native_commands() {
        const BOOT_COMMANDS: [&str; 3] = [
            "allow-boot-state",
            "allow-boot-retry",
            "core:event:allow-listen",
        ];

        let capabilities = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let boot: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(capabilities.join("boot-local.json"))
                .expect("capabilities/boot-local.json is readable"),
        )
        .expect("capabilities/boot-local.json is JSON");

        assert_eq!(
            boot["local"],
            serde_json::Value::Bool(true),
            "the boot page is a local page, not a remote one"
        );
        assert_eq!(
            boot["windows"],
            serde_json::json!(["main", "splash"]),
            "the local boot capability covers exactly the two windows that show the boot page"
        );

        let granted: Vec<String> = boot["permissions"]
            .as_array()
            .expect("permissions is an array")
            .iter()
            .map(|permission| {
                permission
                    .as_str()
                    .expect("a permission is a string")
                    .to_owned()
            })
            .collect();
        for permission in &granted {
            assert!(
                BOOT_COMMANDS.contains(&permission.as_str()),
                "the local boot capability grants {permission}, which the boot page has no use for"
            );
        }
        for permission in REMOTE_PERMISSIONS {
            assert!(
                !granted.iter().any(|entry| entry == permission),
                "the local boot capability grants {permission}: the native commands belong to the \
                 backend origin only"
            );
        }

        // No capability file grants a remote permission either: the runtime
        // capability in `add_remote_capability` is the only grant, and it is
        // the one place where the origin and the main window are checked.
        let mut files: Vec<PathBuf> = fs::read_dir(&capabilities)
            .expect("capabilities is a directory")
            .map(|entry| entry.expect("a capabilities entry").path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        files.sort();
        assert!(
            files.contains(&capabilities.join("boot-local.json")),
            "the boot page's capability file is gone"
        );
        for file in &files {
            let value: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(file).unwrap_or_else(|error| panic!("{file:?}: {error}")),
            )
            .unwrap_or_else(|error| panic!("{file:?} is not JSON: {error}"));
            for permission in value["permissions"]
                .as_array()
                .unwrap_or_else(|| panic!("{file:?} carries no permissions array"))
            {
                let permission = permission.as_str().expect("a permission is a string");
                assert!(
                    !REMOTE_PERMISSIONS.contains(&permission),
                    "{file:?} grants {permission} to every local page; only the runtime remote \
                     capability (sidecar.rs) may"
                );
            }
        }
    }
}

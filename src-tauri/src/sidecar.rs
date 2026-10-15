use std::path::PathBuf;
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

/// Attach-only configurations never spawn: without this, an unreachable
/// attach target would surface the sidecar's "needs a command" error instead
/// of "not responding". A config file may set both `url` and `command` and
/// keep the fall-back-to-spawn semantics.
const PENDING_ROUTE_POLL: Duration = Duration::from_millis(250);
const PENDING_ROUTE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
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
        let manager = self.clone();
        thread::spawn(move || manager.connect(app, attempt, config));
    }

    /// Runs the connection again from the boot page. Never spawns a second
    /// backend behind an already-connected one.
    pub fn retry(&self, app: AppHandle) -> Result<(), String> {
        let mut inner = lock(&self.inner);
        if inner.backend.is_some() {
            return Ok(());
        }
        if inner.shutdown {
            return Err("The shell is shutting down".to_owned());
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
        drop(inner);

        let manager = self.clone();
        thread::spawn(move || manager.connect(app, attempt, config));
        Ok(())
    }

    /// A deep-link route requested while the backend was still connecting.
    /// Applied once the main webview is on the backend origin.
    pub fn set_pending_route(&self, route: String) {
        lock(&self.inner).pending_route = Some(route);
    }

    pub fn origin(&self) -> Option<String> {
        lock(&self.inner).origin.clone()
    }

    /// Stops the backend exactly once. Runs on the exit paths (window close
    /// without a tray, tray Quit, Ctrl+C/SIGTERM); attached backends are left
    /// alone. Blocks while the process group is stopping gracefully.
    pub fn shutdown(&self) {
        let backend = {
            let mut inner = lock(&self.inner);
            inner.shutdown = true;
            inner.pending_route = None;
            inner.backend.take()
        };
        let Some(backend) = backend else {
            log("Hermes backend already stopped or was never connected");
            return;
        };
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
        log(&format!("Connecting to Hermes (config {config:?})"));

        let manager = self.clone();
        let phase_app = app.clone();
        let result = hermes_sidecar::connect(&config, |phase| {
            manager.on_phase(phase_app.clone(), attempt, &phase);
        });

        match result {
            Ok(backend) => self.ready(app, attempt, backend),
            Err(error) => {
                if self.is_current(attempt) && !self.is_shutting_down() {
                    self.set_status(
                        &app,
                        BootStatus::unreachable(None, redact(&error.to_string())),
                    );
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
        let mut inner = lock(&self.inner);
        if self.generation.load(Ordering::SeqCst) != attempt || inner.shutdown {
            drop(inner);
            // A superseded attempt or an exit never adopts a backend: an
            // attached one is a no-op, a spawned one is killed.
            match backend.shutdown() {
                Ok(report) => log(&format!("Discarded superseded backend: {report:?}")),
                Err(error) => log(&format!("Error discarding superseded backend: {error}")),
            }
            return;
        }

        let launch_url = backend.launch_url().to_owned();
        let origin = backend.origin().to_owned();
        inner.backend = Some(backend);
        inner.origin = Some(origin.clone());
        inner.status = BootStatus::ready(origin.clone());
        drop(inner);

        self.set_status(&app, BootStatus::ready(origin.clone()));
        if let Err(error) = self.add_remote_capability(&app, &origin) {
            self.set_status(&app, BootStatus::unreachable(Some(origin), error));
            return;
        }
        match Url::parse(&launch_url) {
            Ok(url) => self.navigate_main(&app, url, None),
            Err(error) => {
                self.set_status(
                    &app,
                    BootStatus::unreachable(
                        Some(origin),
                        format!("Invalid Hermes launch link: {error}"),
                    ),
                );
                return;
            }
        }

        let pending = lock(&self.inner).pending_route.take();
        if let Some(route) = pending {
            let manager = self.clone();
            thread::spawn(move || manager.apply_pending_route(app, origin, route));
        }
    }

    /// Waits for the main webview to reach the backend origin, then applies
    /// the pending deep-link route. The first navigation after connect carries
    /// the session token; the route is a same-document hash change on top.
    fn apply_pending_route(self, app: AppHandle, origin: String, route: String) {
        let deadline = std::time::Instant::now() + PENDING_ROUTE_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if self.is_shutting_down() {
                return;
            }
            if app
                .get_webview_window("main")
                .and_then(|window| window.url().ok())
                .is_some_and(|url| url.origin().ascii_serialization() == origin)
            {
                self.apply_route(&app, &origin, &route);
                return;
            }
            thread::sleep(PENDING_ROUTE_POLL);
        }
        log("Ignored a deep link: Hermes did not finish loading within 30 s");
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
        // backend origin, on the main webview only, and limited to the
        // commands and plugin permissions the UI actually uses.
        let capability = CapabilityBuilder::new("hermes-backend")
            .remote(format!("{origin}/*"))
            .window("main")
            .permission("allow-native-open-external")
            .permission("allow-native-notify")
            .permission("allow-native-select-paths")
            .permission("notification:default")
            .permission("dialog:allow-open")
            .permission_scoped(
                "opener:allow-open-url",
                vec![
                    serde_json::json!({ "url": "http://**" }),
                    serde_json::json!({ "url": "https://**" }),
                ],
                Vec::<serde_json::Value>::new(),
            );
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

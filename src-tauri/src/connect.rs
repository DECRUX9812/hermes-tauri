use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{ipc::CapabilityBuilder, AppHandle, Emitter, Manager, State, Url};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootPhase {
    NoConfig,
    Connecting,
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

    fn connecting(origin: Option<String>) -> Self {
        Self {
            phase: BootPhase::Connecting,
            origin,
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

#[derive(Clone)]
pub struct BootManager {
    launch_config: Arc<Result<Option<String>, String>>,
    state: Arc<Mutex<BootStatus>>,
    generation: Arc<AtomicU64>,
    capability_added: Arc<Mutex<bool>>,
}

#[derive(Clone)]
pub struct LaunchTarget {
    pub launch_link: Url,
    pub origin: String,
}

impl LaunchTarget {
    pub fn parse(value: &str) -> Result<Self, String> {
        let launch_link =
            Url::parse(value).map_err(|error| format!("Invalid launch URL: {error}"))?;
        if !matches!(launch_link.scheme(), "http" | "https")
            || launch_link.host_str().is_none()
            || !launch_link.username().is_empty()
            || launch_link.password().is_some()
        {
            return Err("Launch URL must be an HTTP or HTTPS URL without credentials".to_owned());
        }

        let origin = launch_link.origin().ascii_serialization();
        if origin == "null" {
            return Err("Launch URL does not have a valid HTTP origin".to_owned());
        }

        Ok(Self {
            launch_link,
            origin,
        })
    }
}

impl BootManager {
    pub fn new(launch_config: Result<Option<String>, String>) -> Self {
        let initial_state = match &launch_config {
            Ok(Some(_)) => BootStatus::connecting(None),
            Ok(None) => BootStatus::no_config(),
            Err(error) => BootStatus::unreachable(None, error.clone()),
        };
        Self {
            launch_config: Arc::new(launch_config),
            state: Arc::new(Mutex::new(initial_state)),
            generation: Arc::new(AtomicU64::new(0)),
            capability_added: Arc::new(Mutex::new(false)),
        }
    }

    pub fn start(&self, app: AppHandle) {
        if matches!(self.launch_config.as_ref(), Ok(Some(_))) {
            if let Err(error) = self.retry(app) {
                self.set_status(BootStatus::unreachable(None, error));
            }
        }
    }

    pub fn status(&self) -> BootStatus {
        lock(&self.state).clone()
    }

    pub fn retry(&self, app: AppHandle) -> Result<(), String> {
        let launch_link = match self.launch_config.as_ref() {
            Ok(Some(url)) => url.clone(),
            Ok(None) => return Err("No Hermes URL is configured".to_owned()),
            Err(error) => return Err(error.clone()),
        };
        let attempt = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let target = match LaunchTarget::parse(&launch_link) {
            Ok(target) => target,
            Err(error) => {
                self.set_status(BootStatus::unreachable(None, error.clone()));
                return Err(error);
            }
        };

        self.set_status_if_current(
            &app,
            attempt,
            BootStatus::connecting(Some(target.origin.clone())),
        );
        let manager = self.clone();
        thread::spawn(move || manager.connect(app, attempt, target));
        Ok(())
    }

    fn connect(&self, app: AppHandle, attempt: u64, target: LaunchTarget) {
        if let Err(error) = probe(&target.origin) {
            self.set_status_if_current(
                &app,
                attempt,
                BootStatus::unreachable(Some(target.origin), error),
            );
            return;
        }

        if let Err(error) = self.add_remote_capability(&app, &target.origin) {
            self.set_status_if_current(
                &app,
                attempt,
                BootStatus::unreachable(Some(target.origin), error),
            );
            return;
        }

        if self.generation.load(Ordering::SeqCst) != attempt {
            return;
        }

        match app.get_webview_window("main") {
            Some(webview) => {
                if let Err(error) = webview.navigate(target.launch_link) {
                    self.set_status_if_current(
                        &app,
                        attempt,
                        BootStatus::unreachable(
                            Some(target.origin),
                            format!("Could not open the Hermes launch link: {error}"),
                        ),
                    );
                }
            }
            None => self.set_status_if_current(
                &app,
                attempt,
                BootStatus::unreachable(
                    Some(target.origin),
                    "The main Hermes window is not available",
                ),
            ),
        }
    }

    fn add_remote_capability(&self, app: &AppHandle, origin: &str) -> Result<(), String> {
        let mut added = lock(&self.capability_added);
        if *added {
            return Ok(());
        }

        let capability = CapabilityBuilder::new("hermes-backend")
            .remote(format!("{origin}/*"))
            .window("main".to_owned())
            .permission("allow-native-open-external")
            .permission("allow-native-notify");
        app.add_capability(capability)
            .map_err(|error| format!("Could not grant Hermes native capabilities: {error}"))?;
        *added = true;
        Ok(())
    }

    fn set_status(&self, status: BootStatus) {
        *lock(&self.state) = status;
    }

    fn set_status_if_current(&self, app: &AppHandle, attempt: u64, status: BootStatus) {
        if self.generation.load(Ordering::SeqCst) != attempt {
            return;
        }

        *lock(&self.state) = status.clone();
        let _ = app.emit("boot-state", status);
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

pub fn launch_url_from_args() -> Result<Option<String>, String> {
    let mut args = std::env::args().skip(1);
    let mut command_url = None;

    while let Some(argument) = args.next() {
        let value = if argument == "--url" {
            Some(
                args.next()
                    .ok_or_else(|| "--url requires a launch link".to_owned())?,
            )
        } else {
            argument.strip_prefix("--url=").map(str::to_owned)
        };
        if let Some(value) = value {
            if command_url.replace(value).is_some() {
                return Err("--url may only be provided once".to_owned());
            }
        }
    }

    if let Some(value) = command_url {
        return Ok(Some(value));
    }

    Ok(std::env::var("HERMES_TAURI_URL").ok())
}

fn probe(origin: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Could not create the health-check client: {error}"))?;
    let health_url = format!("{origin}/api/health");
    let started = Instant::now();
    let mut last_error = "health check timed out".to_owned();
    let mut backoff_index = 0;

    loop {
        let elapsed = started.elapsed();
        let Some(remaining) = HEALTH_TIMEOUT.checked_sub(elapsed) else {
            return Err(last_error);
        };
        let response = client.get(&health_url).timeout(remaining).send();
        match response {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                last_error = format!("health check returned HTTP {}", response.status());
            }
            Err(error) => last_error = format!("health check failed: {error}"),
        }

        let elapsed = started.elapsed();
        let Some(remaining) = HEALTH_TIMEOUT.checked_sub(elapsed) else {
            return Err(last_error);
        };
        let delay = RETRY_DELAYS[backoff_index];
        backoff_index = (backoff_index + 1).min(RETRY_DELAYS.len() - 1);
        thread::sleep(delay.min(remaining));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

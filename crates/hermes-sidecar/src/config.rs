use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::launch_link::redact;

#[derive(Clone)]
pub struct SidecarConfig {
    /// A launch link or bare origin of an already running Hermes.
    pub attach_url: Option<String>,
    /// argv used to start Hermes when it is not attached.
    pub command: Vec<String>,
    /// Extra environment for the spawned process.
    pub env: Vec<(String, String)>,
    /// Start `command` when `attach_url` is absent or unreachable.
    pub spawn_if_unreachable: bool,
    pub ready_timeout: Duration,
    pub shutdown_grace: Duration,
    /// Spawned output is appended here with launch-link tokens redacted.
    pub log_path: Option<PathBuf>,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            attach_url: None,
            command: Vec::new(),
            env: Vec::new(),
            spawn_if_unreachable: true,
            ready_timeout: Duration::from_secs(60),
            shutdown_grace: Duration::from_secs(5),
            log_path: None,
        }
    }
}

impl fmt::Debug for SidecarConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let env_keys: Vec<&str> = self.env.iter().map(|(key, _)| key.as_str()).collect();
        f.debug_struct("SidecarConfig")
            .field("attach_url", &self.attach_url.as_deref().map(redact))
            .field("command", &self.command)
            .field("env_keys", &env_keys)
            .field("spawn_if_unreachable", &self.spawn_if_unreachable)
            .field("ready_timeout", &self.ready_timeout)
            .field("shutdown_grace", &self.shutdown_grace)
            .field("log_path", &self.log_path)
            .finish()
    }
}

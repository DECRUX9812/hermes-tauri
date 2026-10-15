use std::fmt;
use std::process::{Child, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::SidecarError;
use crate::launch_link::redact;
use crate::process::ProcessGroup;

const STOP_POLL: Duration = Duration::from_millis(20);
const KILL_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReport {
    /// The backend was attached, so the running Hermes was left alone.
    Detached,
    /// The process exited on its own before shutdown was requested.
    AlreadyExited { code: Option<i32> },
    /// The process group exited after SIGTERM within the grace period.
    Terminated,
    /// The grace period expired and the group was sent SIGKILL.
    Killed,
}

pub struct Backend {
    origin: String,
    launch_url: String,
    process: Option<Process>,
}

impl Backend {
    pub(crate) fn attached(origin: String, launch_url: String) -> Self {
        Self {
            origin,
            launch_url,
            process: None,
        }
    }

    pub(crate) fn spawned(origin: String, launch_url: String, process: Process) -> Self {
        Self {
            origin,
            launch_url,
            process: Some(process),
        }
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Contains the session token; hand it to the webview and never log it.
    pub fn launch_url(&self) -> &str {
        &self.launch_url
    }

    pub fn is_spawned(&self) -> bool {
        self.process.is_some()
    }

    pub fn shutdown(mut self) -> Result<ShutdownReport, SidecarError> {
        match self.process.take() {
            Some(process) => process.stop(),
            None => Ok(ShutdownReport::Detached),
        }
    }
}

impl fmt::Debug for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backend")
            .field("origin", &self.origin)
            .field("launch_url", &redact(&self.launch_url))
            .field("spawned", &self.is_spawned())
            .finish()
    }
}

/// Owns a spawned Hermes and its process group. Dropping it kills the group.
pub(crate) struct Process {
    child: Child,
    group: ProcessGroup,
    status: Option<ExitStatus>,
    grace: Duration,
}

impl Process {
    pub(crate) fn new(child: Child, group: ProcessGroup, grace: Duration) -> Self {
        Self {
            child,
            group,
            status: None,
            grace,
        }
    }

    pub(crate) fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Reaps the leader once it has exited and caches the status.
    pub(crate) fn poll_exit(&mut self) -> Result<Option<ExitStatus>, SidecarError> {
        if self.status.is_none() {
            self.status = self
                .child
                .try_wait()
                .map_err(|source| SidecarError::Wait { source })?;
        }
        Ok(self.status)
    }

    pub(crate) fn exit_code(&self) -> Option<i32> {
        self.status.and_then(|status| status.code())
    }

    fn stop(mut self) -> Result<ShutdownReport, SidecarError> {
        let exited_first = self.poll_exit()?.is_some();
        let grace = self.grace;
        let stopped_gracefully = self.group.request_stop() && self.wait_until_gone(grace)?;
        if stopped_gracefully {
            return Ok(if exited_first {
                ShutdownReport::AlreadyExited {
                    code: self.exit_code(),
                }
            } else {
                ShutdownReport::Terminated
            });
        }

        self.group.kill();
        if !self.wait_until_gone(KILL_WAIT)? {
            return Err(SidecarError::StillRunning { pid: self.pid() });
        }
        Ok(ShutdownReport::Killed)
    }

    fn wait_until_gone(&mut self, timeout: Duration) -> Result<bool, SidecarError> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.poll_exit()?.is_some() && !self.group.has_live_members() {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(STOP_POLL);
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.group.has_members() {
            self.group.kill();
        }
        if self.status.is_none() {
            let _ = self.child.wait();
        }
    }
}

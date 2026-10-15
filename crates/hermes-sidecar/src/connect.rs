use std::io;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::backend::{Backend, Process};
use crate::config::SidecarConfig;
use crate::error::SidecarError;
use crate::health::is_healthy;
use crate::launch_link::{origin_of, LaunchLink};
use crate::output::{pump, Output};
use crate::phase::Phase;
use crate::process;

const ATTACH_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const LINK_POLL: Duration = Duration::from_millis(50);
const BACKOFF_START: Duration = Duration::from_millis(50);
const BACKOFF_MAX: Duration = Duration::from_millis(500);
const EXIT_DRAIN: Duration = Duration::from_millis(500);

/// Attaches to `attach_url` when it answers, otherwise spawns `command` if allowed.
///
/// Blocking: run it on a background thread. `on_phase` is called from that thread.
pub fn connect(
    cfg: &SidecarConfig,
    mut on_phase: impl FnMut(Phase),
) -> Result<Backend, SidecarError> {
    let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
    let attached_origin = match &cfg.attach_url {
        Some(url) => {
            let origin = origin_of(url).ok_or(SidecarError::InvalidAttachUrl)?;
            on_phase(Phase::Probing {
                origin: origin.clone(),
            });
            if is_healthy(&agent, &origin, ATTACH_PROBE_TIMEOUT) {
                on_phase(Phase::Ready {
                    origin: origin.clone(),
                });
                return Ok(Backend::attached(origin, url.clone()));
            }
            Some(origin)
        }
        None => None,
    };

    if !cfg.spawn_if_unreachable {
        return Err(match attached_origin {
            Some(origin) => SidecarError::Unreachable { origin },
            None => SidecarError::NothingToConnect,
        });
    }
    spawn(cfg, &agent, &mut on_phase)
}

fn spawn(
    cfg: &SidecarConfig,
    agent: &ureq::Agent,
    on_phase: &mut impl FnMut(Phase),
) -> Result<Backend, SidecarError> {
    let (program, args) = cfg.command.split_first().ok_or(SidecarError::NoCommand)?;
    on_phase(Phase::Spawning {
        argv0: program.clone(),
    });
    let output = Arc::new(Output::open(cfg.log_path.as_deref())?);

    let mut command = Command::new(program);
    command
        .args(args)
        .envs(cfg.env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::configure(&mut command);
    let spawn_error = |source: io::Error| SidecarError::Spawn {
        program: program.clone(),
        source,
    };
    let mut child = command.spawn().map_err(spawn_error)?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let group = process::adopt(&mut child).map_err(spawn_error)?;
    let mut process = Process::new(child, group, cfg.shutdown_grace);

    let (links_tx, links) = mpsc::channel();
    let (eof_tx, eof) = mpsc::channel();
    pump(
        stdout,
        Arc::clone(&output),
        links_tx.clone(),
        eof_tx.clone(),
    );
    pump(stderr, Arc::clone(&output), links_tx, eof_tx);
    let watch = Watch {
        output,
        links,
        eof,
        ready_timeout: cfg.ready_timeout,
    };
    let deadline = Instant::now() + cfg.ready_timeout;

    on_phase(Phase::WaitingForLaunchLink);
    let link = watch.launch_link(&mut process, deadline)?;
    on_phase(Phase::WaitingForHealth {
        origin: link.origin.clone(),
    });
    watch.health(&mut process, agent, &link.origin, deadline)?;
    on_phase(Phase::Ready {
        origin: link.origin.clone(),
    });
    Ok(Backend::spawned(link.origin, link.url, process))
}

/// Waits for a spawned child to become ready. Any error drops the `Process`, which kills it.
struct Watch {
    output: Arc<Output>,
    links: Receiver<LaunchLink>,
    eof: Receiver<()>,
    ready_timeout: Duration,
}

impl Watch {
    fn launch_link(
        &self,
        process: &mut Process,
        deadline: Instant,
    ) -> Result<LaunchLink, SidecarError> {
        loop {
            if process.poll_exit()?.is_some() {
                return Err(self.exited(process.exit_code()));
            }
            match self.links.recv_timeout(LINK_POLL) {
                Ok(link) => return Ok(link),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => thread::sleep(LINK_POLL),
            }
            if Instant::now() >= deadline {
                return Err(self.timed_out("the launch link".to_owned()));
            }
        }
    }

    fn health(
        &self,
        process: &mut Process,
        agent: &ureq::Agent,
        origin: &str,
        deadline: Instant,
    ) -> Result<(), SidecarError> {
        let mut backoff = BACKOFF_START;
        loop {
            if process.poll_exit()?.is_some() {
                return Err(self.exited(process.exit_code()));
            }
            if is_healthy(agent, origin, HEALTH_PROBE_TIMEOUT) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(self.timed_out(format!("Hermes to answer {origin}/api/health")));
            }
            thread::sleep(backoff);
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    }

    /// Waits briefly for both pipes to reach EOF so the tail includes the child's last lines.
    fn exited(&self, code: Option<i32>) -> SidecarError {
        for _ in 0..2 {
            if self.eof.recv_timeout(EXIT_DRAIN).is_err() {
                break;
            }
        }
        SidecarError::ChildExited {
            code,
            tail: self.output.tail(),
        }
    }

    fn timed_out(&self, waiting_for: String) -> SidecarError {
        SidecarError::Timeout {
            waiting_for,
            after: self.ready_timeout,
            tail: self.output.tail(),
        }
    }
}

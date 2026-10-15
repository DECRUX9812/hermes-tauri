//! The zero-mock proof for the bundled payload: resolve the real payload with
//! `resolve_with_bundled`, then start it through hermes-sidecar's real spawn
//! path and stop it again.
//!
//! Ignored by default: the payload is 341 MB and a cold start to `/healthz`
//! took 24 s on this host (T10 measured 17.5 s to `/api/health`). Run it as
//!
//!   HERMES_TAURI_PAYLOAD_DIR=$HOME/.cache/hermes-tauri/payload \
//!     cargo test -p hermes-shell-config --locked --test bundled_payload -- --ignored --nocapture

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hermes_shell_config::{resolve_with_bundled, Bundled, Inputs, Resolved, Source};
use hermes_sidecar::{connect, Phase, ShutdownReport};

/// The payload's cold start is dominated by the stock Hermes dashboard child.
const READY_TIMEOUT: Duration = Duration::from_secs(90);
/// The launcher's own graceful stop waits up to 10 s for that child.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

#[test]
#[ignore = "needs the 341 MB payload: set HERMES_TAURI_PAYLOAD_DIR and run with --ignored"]
fn the_bundled_payload_spawns_serves_health_and_stops_clean() {
    let payload_dir = payload_dir();
    let data = tempfile::tempdir().expect("tempdir for the app-data dir");
    let config_dir = tempfile::tempdir().expect("tempdir for the config dir");
    let bundled = Bundled {
        payload_dir,
        data_dir: data.path().to_path_buf(),
    };

    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    let (mut config, source) = match resolve_with_bundled(&inputs, Some(&bundled)).expect("resolve")
    {
        Resolved::Configured { config, source } => (config, source),
        Resolved::NotConfigured => panic!("the bundled payload did not resolve"),
    };
    assert_eq!(source, Source::Bundled);
    config.log_path = Some(data.path().join("sidecar.log"));
    config.ready_timeout = READY_TIMEOUT;
    config.shutdown_grace = SHUTDOWN_GRACE;
    let command = config.command.clone();
    let requested_port: u16 = command[4].parse().expect("resolve picked a numeric port");
    println!("resolve: source={source:?} command={command:?}");
    println!(
        "resolve: env keys={:?}",
        config.env.iter().map(|(key, _)| key).collect::<Vec<_>>()
    );

    let started = Instant::now();
    let mut phases = Vec::new();
    let backend = connect(&config, |phase| phases.push(phase)).expect("the payload becomes ready");
    let ready = started.elapsed();
    println!(
        "ready after {:.1}s: origin={}",
        ready.as_secs_f32(),
        backend.origin()
    );
    println!("phases: {phases:?}");

    // The sidecar only learns the origin from a launch link it parses out of the
    // payload's stdout, so this is the front-door login line, not an attach URL.
    assert!(backend.is_spawned());
    assert!(phases.contains(&Phase::WaitingForLaunchLink));
    assert_eq!(
        phases.last(),
        Some(&Phase::Ready {
            origin: backend.origin().to_owned()
        })
    );
    let launch_url = backend.launch_url().to_owned();
    assert!(
        launch_url.starts_with(&format!("http://127.0.0.1:{requested_port}/?token=")),
        "the login line must carry the port the shell picked ({requested_port})"
    );
    let token = launch_url
        .split_once("?token=")
        .map(|(_, token)| token.to_owned())
        .expect("the launch URL carries the front-door token");

    // HTTP: the open liveness route, then the real API through the front door.
    let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
    let healthz = status(&agent, backend.origin(), "/healthz", None);
    let api_health = status(&agent, backend.origin(), "/api/health", Some(&token));
    let api_health_without_token = status(&agent, backend.origin(), "/api/health", None);
    println!(
        "http: /healthz={healthz:?} /api/health+token={api_health:?} /api/health={api_health_without_token:?}"
    );
    assert_eq!(
        healthz,
        Some(200),
        "the payload's own liveness route answers"
    );
    assert_eq!(api_health, Some(200), "the API answers with the token");
    assert_eq!(
        api_health_without_token,
        Some(401),
        "the front door stays shut without the token"
    );

    // The process tree: the leader in its own process group, plus the stock
    // Hermes dashboard child, which hermes_plus starts in its own session. Both
    // inherit the spawn environment, so both carry this temp HERMES_HOME.
    let (leader, pgid) = payload_leader(requested_port).expect("the spawned leader is visible");
    assert_eq!(pgid, leader, "the leader leads its own process group");
    let with_home = procs_with_hermes_home(&data.path().join("hermes"));
    println!("pids: leader={leader} pgid={pgid} with_this_home={with_home:?}");
    assert!(
        with_home.contains(&leader),
        "the launcher runs with the spawn environment"
    );
    assert!(
        with_home.iter().any(|pid| *pid != leader),
        "hermes_plus must have started its dashboard child: {with_home:?}"
    );

    // Privacy: the spawned output is logged with the token masked.
    let log = fs::read_to_string(config.log_path.as_ref().expect("log path"))
        .expect("read the sidecar log");
    assert!(
        log.contains("Hermes Plus login:"),
        "the login line is logged"
    );
    assert!(
        !log.contains(&token),
        "the front-door token must not reach the log"
    );
    assert!(log.contains("?token=<redacted>"));

    let report = backend.shutdown().expect("shutdown");
    println!("shutdown: {report:?}");
    assert_eq!(report, ShutdownReport::Terminated);

    // Nothing of the tree survives: leader, group, and the dashboard child,
    // which runs outside the sidecar's process group in its own session.
    assert!(!alive(leader), "the leader survived shutdown");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let members = group_members(pgid);
        let leftovers = procs_with_hermes_home(&data.path().join("hermes"));
        if members.is_empty() && leftovers.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "leftovers: group members {members:?}, processes with this HERMES_HOME {leftovers:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    println!(
        "after shutdown: group members={:?} env leftovers={:?}",
        group_members(pgid),
        procs_with_hermes_home(&data.path().join("hermes"))
    );
}

fn payload_dir() -> PathBuf {
    let raw = std::env::var_os("HERMES_TAURI_PAYLOAD_DIR").unwrap_or_else(|| {
        panic!("set HERMES_TAURI_PAYLOAD_DIR to the payload dir, e.g. $HOME/.cache/hermes-tauri/payload")
    });
    let dir = PathBuf::from(raw)
        .canonicalize()
        .expect("the payload dir exists");
    assert!(
        dir.join("hermes-payload/bin/hermes-sidecar").is_file(),
        "{} is not a payload dir: no hermes-payload/bin/hermes-sidecar",
        dir.display()
    );
    assert!(
        dir.join("python").is_dir(),
        "{} has no python/ tree, so the payload is incomplete",
        dir.display()
    );
    dir
}

fn status(agent: &ureq::Agent, origin: &str, path: &str, token: Option<&str>) -> Option<u16> {
    let mut request = agent
        .get(&format!("{origin}{path}"))
        .timeout(Duration::from_secs(10));
    if let Some(token) = token {
        request = request.set("x-hermes-session-token", token);
    }
    match request.call() {
        Ok(response) => Some(response.status()),
        Err(ureq::Error::Status(code, _)) => Some(code),
        Err(_) => None,
    }
}

/// The pid and process group of the payload's `hermes_plus` leader, found by the
/// port it was told to listen on. The sidecar does not expose the pid.
fn payload_leader(port: u16) -> Option<(u32, u32)> {
    let needle = format!("--port {port}");
    let mut found = None;
    for pid in pids() {
        let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        if cmdline.contains("hermes_plus.cli serve") && cmdline.contains(&needle) {
            found = Some((pid, pgid_of(pid)));
        }
    }
    let (pid, pgid) = found?;
    Some((pid, pgid?))
}

/// Every live (non-zombie) process in `pgid`.
fn group_members(pgid: u32) -> Vec<u32> {
    pids()
        .into_iter()
        .filter(|pid| alive(*pid) && pgid_of(*pid) == Some(pgid))
        .collect()
}

fn pgid_of(pid: u32) -> Option<u32> {
    // `/proc/<pid>/stat`: `pid (comm) state ppid pgrp ...`, and `comm` may hold
    // spaces and parentheses, so parse after the last ')'.
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.rsplit_once(')')?.1;
    let mut fields = rest.split_whitespace();
    fields.next()?; // state
    fields.next()?; // ppid
    fields.next()?.parse().ok()
}

fn alive(pid: u32) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !stat
            .rsplit_once(')')
            .map(|(_, rest)| rest.starts_with(" Z"))
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Processes whose environment still names this Hermes home. The dashboard
/// child runs in its own session (`hermes_plus` passes `start_new_session=True`),
/// so the sidecar's process group alone does not prove it is gone.
fn procs_with_hermes_home(home: &Path) -> Vec<u32> {
    let needle = format!("HERMES_HOME={}", home.display());
    pids()
        .into_iter()
        .filter(|pid| {
            fs::read(format!("/proc/{pid}/environ"))
                .map(|environ| {
                    environ
                        .split(|byte| *byte == 0)
                        .any(|var| var == needle.as_bytes())
                })
                .unwrap_or(false)
        })
        .collect()
}

fn pids() -> Vec<u32> {
    let mut pids: Vec<u32> = fs::read_dir("/proc")
        .expect("read /proc")
        .filter_map(|entry| {
            entry
                .ok()?
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
        })
        .collect();
    pids.sort_unstable();
    pids
}

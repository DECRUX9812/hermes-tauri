#![allow(dead_code)]

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hermes_sidecar::SidecarConfig;

pub const TOKEN: &str = "Demo0Sidecar0Token0Fixture0Value0Example0AB";

pub struct FixturePids {
    pub main: u32,
    pub grandchild: Option<u32>,
}

/// The fixture is a Python stand-in that imitates only the webapp stdout/health contract.
pub fn fixture_command(extra: &[&str]) -> Vec<String> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/webapp_fixture.py");
    let mut argv = vec!["python3".to_owned(), script.display().to_string()];
    argv.extend(extra.iter().map(|arg| (*arg).to_owned()));
    argv
}

pub fn base_config(command: Vec<String>, log: &Path) -> SidecarConfig {
    SidecarConfig {
        command,
        log_path: Some(log.to_owned()),
        ready_timeout: Duration::from_secs(20),
        shutdown_grace: Duration::from_secs(5),
        ..SidecarConfig::default()
    }
}

pub fn log_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("hermes-tauri-sidecar-tests");
    fs::create_dir_all(&dir).expect("create test scratch dir");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    dir.join(format!("{name}-{}-{nanos}.log", std::process::id()))
}

pub fn wait_for_pids(log: &Path, with_grandchild: bool) -> FixturePids {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = fs::read_to_string(log).unwrap_or_default();
        let mut main = None;
        let mut grandchild = None;
        for line in text.lines() {
            if let Some(pid) = line.strip_prefix("[webapp_fixture] pid=") {
                main = pid.parse().ok();
            } else if let Some(pid) = line.strip_prefix("[webapp_fixture] grandchild pid=") {
                grandchild = pid.parse().ok();
            }
        }
        let ready = main.is_some() && (!with_grandchild || grandchild.is_some());
        if ready {
            return FixturePids {
                main: main.expect("checked above"),
                grandchild,
            };
        }
        assert!(
            Instant::now() < deadline,
            "fixture pids never appeared in {}",
            log.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Zombies count as dead: they have exited and only await reaping.
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let exists = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
    if !exists {
        return false;
    }
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => {
            let state = stat.rsplit(')').next().unwrap_or("").trim_start();
            !state.starts_with('Z') && !state.starts_with('X')
        }
        Err(_) => true,
    }
}

pub fn wait_until_dead(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while process_alive(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
    true
}

pub fn http_status(origin: &str) -> Option<u16> {
    let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
    match agent.get(&format!("{origin}/api/health")).call() {
        Ok(response) => Some(response.status()),
        Err(ureq::Error::Status(code, _)) => Some(code),
        Err(_) => None,
    }
}

/// Returns a port in the 18800–18899 test range that is free right now.
pub fn free_test_port() -> u16 {
    (18800..=18899)
        .find(|port| TcpListener::bind(("127.0.0.1", *port)).is_ok())
        .expect("a free port in 18800-18899")
}

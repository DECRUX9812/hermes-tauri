//! Attach mode. `tests/common` reads `/proc` and uses `kill(2)`, so this target
//! is Unix-only.
#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

use common::*;
use hermes_sidecar::{connect, Phase, ShutdownReport, SidecarConfig, SidecarError};

/// A fixture started by the test itself, so the test can check it survives a detach.
struct RunningFixture {
    child: Child,
    _stdout: BufReader<ChildStdout>,
}

impl Drop for RunningFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_running_fixture() -> (RunningFixture, String) {
    let argv = fixture_command(&["--token", TOKEN]);
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start fixture");
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut reader = BufReader::new(stdout);
    let mut link = String::new();
    reader.read_line(&mut link).expect("read launch link");
    let fixture = RunningFixture {
        child,
        _stdout: reader,
    };
    (fixture, link.trim_end().to_owned())
}

#[test]
fn attaches_to_running_hermes_and_leaves_it_running() {
    let (fixture, link) = start_running_fixture();
    let origin = link
        .split_once("/#")
        .expect("launch link has a fragment")
        .0
        .to_owned();
    let cfg = SidecarConfig {
        attach_url: Some(link.clone()),
        spawn_if_unreachable: false,
        ..SidecarConfig::default()
    };
    let mut phases = Vec::new();

    let backend = connect(&cfg, |phase| phases.push(phase)).expect("attach succeeds");
    assert!(!backend.is_spawned());
    assert_eq!(backend.origin(), origin);
    assert_eq!(backend.launch_url(), link);
    assert_eq!(
        phases,
        vec![
            Phase::Probing {
                origin: origin.clone()
            },
            Phase::Ready {
                origin: origin.clone()
            },
        ]
    );
    assert!(!format!("{cfg:?}").contains(TOKEN));

    assert_eq!(
        backend.shutdown().expect("detach"),
        ShutdownReport::Detached
    );
    assert_eq!(
        http_status(&origin),
        Some(200),
        "attached Hermes must keep running"
    );
    drop(fixture);
}

#[test]
fn unreachable_attach_target_without_spawn_is_an_error() {
    let port = free_test_port();
    let origin = format!("http://127.0.0.1:{port}");
    let cfg = SidecarConfig {
        attach_url: Some(format!("{origin}/#hermes-session={TOKEN}")),
        spawn_if_unreachable: false,
        ..SidecarConfig::default()
    };

    let err = connect(&cfg, |_| {}).expect_err("nothing listens on the port");
    match &err {
        SidecarError::Unreachable { origin: reported } => assert_eq!(reported, &origin),
        other => panic!("expected Unreachable, got {other:?}"),
    }
    assert!(!format!("{err:?}").contains(TOKEN));
}

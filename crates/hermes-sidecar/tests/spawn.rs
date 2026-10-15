//! Process-group lifecycle: the fixture is started, observed and reaped with
//! Unix signals (`tests/common` probes `/proc` and `kill(2)`), so this target is
//! Unix-only. Windows process handling is the Job Object path in
//! `src/process/windows.rs`, which is compile-checked rather than run here.
#![cfg(unix)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use hermes_sidecar::{connect, connect_with_cancel, Cancel, Phase, ShutdownReport, SidecarError};

#[test]
fn spawned_group_including_grandchild_is_reaped_on_shutdown() {
    let log = log_path("lifecycle");
    let cfg = base_config(
        fixture_command(&["--token", TOKEN, "--spawn-grandchild"]),
        &log,
    );
    let mut phases = Vec::new();
    let backend = connect(&cfg, |phase| phases.push(phase)).expect("fixture becomes ready");

    assert!(backend.is_spawned());
    assert!(backend.origin().starts_with("http://127.0.0.1:"));
    assert_ne!(backend.origin(), "http://127.0.0.1:0");
    assert!(backend
        .launch_url()
        .ends_with(&format!("#hermes-session={TOKEN}")));
    assert!(phases.contains(&Phase::WaitingForLaunchLink));
    assert_eq!(
        phases.last(),
        Some(&Phase::Ready {
            origin: backend.origin().to_owned()
        })
    );
    assert_eq!(http_status(backend.origin()), Some(200));

    let pids = wait_for_pids(&log, true);
    let grandchild = pids.grandchild.expect("grandchild pid logged");
    assert_eq!(
        backend.shutdown().expect("shutdown"),
        ShutdownReport::Terminated
    );
    assert!(!process_alive(pids.main), "main process survived shutdown");
    assert!(!process_alive(grandchild), "grandchild survived shutdown");
    let _ = std::fs::remove_file(log);
}

#[test]
fn sigterm_ignoring_group_escalates_to_sigkill_within_grace() {
    let log = log_path("sigterm-ignored");
    let grace = Duration::from_secs(1);
    let cfg = {
        let mut cfg = base_config(
            fixture_command(&["--token", TOKEN, "--ignore-sigterm", "--spawn-grandchild"]),
            &log,
        );
        cfg.shutdown_grace = grace;
        cfg
    };
    let backend = connect(&cfg, |_| {}).expect("fixture becomes ready");
    let pids = wait_for_pids(&log, true);
    let grandchild = pids.grandchild.expect("grandchild pid logged");

    let started = Instant::now();
    let report = backend.shutdown().expect("shutdown");
    assert_eq!(report, ShutdownReport::Killed);
    assert!(started.elapsed() < grace + Duration::from_secs(3));
    assert!(!process_alive(pids.main));
    assert!(!process_alive(grandchild));
    let _ = std::fs::remove_file(log);
}

#[test]
fn child_exiting_early_reports_exit_code_and_tail_without_token() {
    let log = log_path("exit-early");
    let cfg = base_config(fixture_command(&["--token", TOKEN, "--exit-early"]), &log);
    let err = connect(&cfg, |_| {}).expect_err("fixture exits before ready");

    match &err {
        SidecarError::ChildExited { code, tail } => {
            assert_eq!(*code, Some(3));
            assert!(tail.iter().any(|line| line.contains("boot step 2 of 2")));
            assert!(tail
                .iter()
                .any(|line| line.contains("#hermes-session=<redacted>")));
        }
        other => panic!("expected ChildExited, got {other:?}"),
    }
    let rendered = format!("{err} {err:?}");
    assert!(!rendered.contains(TOKEN));
    let _ = std::fs::remove_file(log);
}

#[test]
fn ready_timeout_reports_timeout_and_kills_child() {
    let log = log_path("ready-timeout");
    let mut cfg = base_config(
        fixture_command(&["--token", TOKEN, "--delay-ready", "30"]),
        &log,
    );
    cfg.ready_timeout = Duration::from_millis(1500);

    let started = Instant::now();
    let err = connect(&cfg, |_| {}).expect_err("health never turns ready");
    assert!(matches!(err, SidecarError::Timeout { .. }), "got {err:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!format!("{err} {err:?}").contains(TOKEN));

    let pids = wait_for_pids(&log, false);
    assert!(
        wait_until_dead(pids.main, Duration::from_secs(5)),
        "child survived timeout"
    );
    let _ = std::fs::remove_file(log);
}

#[test]
fn base_path_stays_in_origin_so_health_is_probed_under_it() {
    let log = log_path("base-path");
    let cfg = base_config(
        fixture_command(&["--token", TOKEN, "--base-path", "/hermes"]),
        &log,
    );
    let backend = connect(&cfg, |_| {}).expect("fixture becomes ready");

    assert!(
        backend.origin().ends_with("/hermes"),
        "origin was {}",
        backend.origin()
    );
    assert_eq!(http_status(backend.origin()), Some(200));
    assert_eq!(
        backend.shutdown().expect("shutdown"),
        ShutdownReport::Terminated
    );
    let _ = std::fs::remove_file(log);
}

#[test]
fn dropping_backend_without_shutdown_kills_the_group() {
    let log = log_path("drop");
    let cfg = base_config(
        fixture_command(&["--token", TOKEN, "--spawn-grandchild"]),
        &log,
    );
    let backend = connect(&cfg, |_| {}).expect("fixture becomes ready");
    let pids = wait_for_pids(&log, true);
    let grandchild = pids.grandchild.expect("grandchild pid logged");

    drop(backend);
    assert!(
        wait_until_dead(pids.main, Duration::from_secs(5)),
        "main survived drop"
    );
    assert!(
        wait_until_dead(grandchild, Duration::from_secs(5)),
        "grandchild survived drop"
    );
    let _ = std::fs::remove_file(log);
}

#[test]
fn cancelling_a_slow_spawn_kills_the_child_quickly() {
    let log = log_path("cancel");
    let cfg = base_config(
        fixture_command(&["--token", TOKEN, "--delay-ready", "30"]),
        &log,
    );
    let cancel = Cancel::new();
    let handle = std::thread::spawn({
        let cfg = cfg.clone();
        let cancel = cancel.clone();
        move || connect_with_cancel(&cfg, |_| {}, &cancel)
    });

    // Quit while the backend is still starting, as the shell does on exit.
    let pids = wait_for_pids(&log, false);
    cancel.cancel();

    let started = Instant::now();
    let err = handle
        .join()
        .expect("connect thread")
        .expect_err("cancelled");
    assert!(
        matches!(err, SidecarError::Cancelled),
        "expected Cancelled, got {err:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel took {:?}",
        started.elapsed()
    );
    assert!(
        wait_until_dead(pids.main, Duration::from_secs(5)),
        "child survived cancel"
    );
    let _ = std::fs::remove_file(log);
}

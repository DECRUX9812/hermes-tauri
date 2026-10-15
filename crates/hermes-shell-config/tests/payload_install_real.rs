//! The zero-mock proof for the archive installer: pack the real 341 MB payload
//! with `scripts/pack-payload.sh`, unpack it with `ensure_unpacked`, then start
//! the unpacked launcher through hermes-sidecar's real spawn path and stop it
//! again.
//!
//! Ignored by default: it writes ~430 MB and a cold boot to `/healthz` takes
//! 16-50 s on this host. Run it as
//!
//!   HERMES_TAURI_PAYLOAD_DIR=$HOME/.cache/hermes-tauri/payload \
//!     cargo test -p hermes-shell-config --locked --test payload_install_real -- --ignored --nocapture
//!
//! The process helpers at the bottom mirror `tests/bundled_payload.rs` (T15);
//! each integration test is its own crate, so they are repeated rather than
//! shared.
//!
//! The installer itself (`payload_install`) is Unix-only for now: the launcher
//! it unpacks is a Unix executable, so this target is Unix-only too.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use hermes_shell_config::{
    ensure_unpacked, read_manifest, resolve_with_bundled, Inputs, Resolved, Source,
};
use hermes_sidecar::{connect, Phase, ShutdownReport};

/// The payload's cold start is dominated by the stock Hermes dashboard child.
const READY_TIMEOUT: Duration = Duration::from_secs(90);
/// The launcher's own graceful stop waits up to 10 s for that child.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

#[test]
#[ignore = "packs and unpacks ~430 MB: set HERMES_TAURI_PAYLOAD_DIR and run with --ignored"]
fn the_packed_payload_unpacks_and_boots_from_the_app_data_dir() {
    let payload = payload_dir();
    let scratch = fresh_scratch_dir();
    let pack_out = scratch.join("pack");
    let data_dir = scratch.join("data");

    // 1. Pack the real payload with the script a release build uses, and
    //    without --allow-build-paths: the pack gate itself must pass, so this
    //    test refuses a tree whose *.pyc embed a build path.
    let script = workspace_root().join("scripts/pack-payload.sh");
    assert!(script.is_file(), "{} is missing", script.display());
    let started = Instant::now();
    let output = Command::new("bash")
        .arg(&script)
        .arg("--payload")
        .arg(&payload)
        .arg("--out")
        .arg(&pack_out)
        .output()
        .expect("run scripts/pack-payload.sh");
    let pack_seconds = started.elapsed().as_secs_f32();
    println!(
        "--- pack-payload.sh stdout ---\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "--- pack-payload.sh stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "scripts/pack-payload.sh failed with {:?}",
        output.status
    );

    let archive = pack_out.join("hermes-payload.tar.zst");
    let manifest_path = pack_out.join("payload-manifest.json");
    let manifest = read_manifest(&manifest_path).expect("the packed manifest parses");
    assert_eq!(
        manifest.archive_bytes,
        fs::metadata(&archive).expect("archive metadata").len(),
        "the manifest names the archive it was written for"
    );
    assert!(manifest.unpacked_bytes > 0 && manifest.files > 0);
    println!(
        "packed: version={} archive={} bytes ({:.1} MiB) files={} unpacked={} bytes ({:.1} MiB) in {pack_seconds:.1}s",
        manifest.version,
        manifest.archive_bytes,
        manifest.archive_bytes as f64 / (1024.0 * 1024.0),
        manifest.files,
        manifest.unpacked_bytes,
        manifest.unpacked_bytes as f64 / (1024.0 * 1024.0),
    );

    // 2. The first call unpacks.
    let started = Instant::now();
    let bundled =
        ensure_unpacked(&archive, &manifest_path, &data_dir).expect("the real payload unpacks");
    let unpack_seconds = started.elapsed().as_secs_f32();
    assert_eq!(
        bundled.payload_dir,
        data_dir.join("payload").join(&manifest.version)
    );
    assert!(bundled
        .payload_dir
        .join("hermes-payload/bin/hermes-sidecar")
        .is_file());
    // The symlink Tauri's resource copy would have replaced with a 32 MB copy,
    // which is what made the copied venv fail to find its stdlib (T16a run A).
    let interpreter = bundled.payload_dir.join("hermes-payload/venv/bin/python");
    assert!(
        fs::symlink_metadata(&interpreter)
            .expect("the venv interpreter exists")
            .file_type()
            .is_symlink(),
        "{} must still be a symlink",
        interpreter.display()
    );
    println!(
        "unpacked in {unpack_seconds:.1}s into {}",
        bundled.payload_dir.display()
    );

    // 3. The unpacked venv resolves its own prefix, and imports hermes_plus:
    //    exactly the check that failed for the dereferenced copy.
    let probe = Command::new(&interpreter)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg("-c")
        .arg("import hermes_plus, sys; print(sys.prefix); print(sys.base_prefix); print(sys.executable)")
        .output()
        .expect("run the unpacked venv interpreter");
    let stdout = String::from_utf8_lossy(&probe.stdout);
    println!(
        "venv probe stdout:\n{stdout}venv probe stderr:\n{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    assert!(
        probe.status.success(),
        "the unpacked venv cannot import hermes_plus"
    );
    for line in stdout.lines().take(3) {
        assert!(
            Path::new(line).starts_with(&bundled.payload_dir),
            "{line} is outside the unpacked payload"
        );
    }

    // 4. The second call is the `.ready` short circuit.
    let started = Instant::now();
    let again = ensure_unpacked(&archive, &manifest_path, &data_dir).expect("second call");
    let ready_seconds = started.elapsed().as_secs_f32();
    assert_eq!(again.payload_dir, bundled.payload_dir);
    println!("second call (.ready): {ready_seconds:.3}s");

    // 5. Resolve and boot it: the shell must spawn the unpacked launcher.
    let inputs = Inputs::empty(scratch.join("config"));
    let (mut config, source) = match resolve_with_bundled(&inputs, Some(&again)).expect("resolve") {
        Resolved::Configured { config, source } => (config, source),
        Resolved::NotConfigured => panic!("the unpacked payload did not resolve"),
    };
    assert_eq!(source, Source::Bundled);
    assert!(
        Path::new(&config.command[0]).starts_with(&bundled.payload_dir),
        "the launcher runs from the unpacked payload, not the cache: {:?}",
        config.command[0]
    );
    let env: std::collections::BTreeMap<String, String> = config.env.clone().into_iter().collect();
    assert_eq!(
        env["HERMES_HOME"],
        data_dir.join("hermes").to_string_lossy().into_owned(),
        "user state stays outside the versioned payload"
    );
    let command = config.command.clone();
    let requested_port: u16 = command[4].parse().expect("resolve picked a numeric port");
    println!("resolve: source={source:?} command={command:?}");

    config.log_path = Some(scratch.join("sidecar.log"));
    config.ready_timeout = READY_TIMEOUT;
    config.shutdown_grace = SHUTDOWN_GRACE;

    let started = Instant::now();
    let mut phases = Vec::new();
    let backend = connect(&config, |phase| phases.push(phase)).expect("the payload becomes ready");
    let boot_seconds = started.elapsed().as_secs_f32();
    println!(
        "ready after {boot_seconds:.1}s: origin={}",
        backend.origin()
    );
    println!("phases: {phases:?}");
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

    let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
    let healthz = status(&agent, backend.origin(), "/healthz", None);
    let api_health = status(&agent, backend.origin(), "/api/health", Some(&token));
    let api_health_without_token = status(&agent, backend.origin(), "/api/health", None);
    println!(
        "http: /healthz={healthz:?} /api/health+token={api_health:?} /api/health={api_health_without_token:?}"
    );
    assert_eq!(healthz, Some(200), "the unpacked payload answers /healthz");
    assert_eq!(api_health, Some(200), "the API answers with the token");
    assert_eq!(
        api_health_without_token,
        Some(401),
        "the front door stays shut without the token"
    );

    // The process tree: the leader in its own group plus the dashboard child,
    // which hermes_plus starts in its own session.
    let (leader, pgid) = payload_leader(requested_port).expect("the spawned leader is visible");
    assert_eq!(pgid, leader, "the leader leads its own process group");
    let with_home = procs_with_hermes_home(&data_dir.join("hermes"));
    println!("pids: leader={leader} pgid={pgid} with_this_home={with_home:?}");
    assert!(with_home.contains(&leader));
    assert!(
        with_home.iter().any(|pid| *pid != leader),
        "hermes_plus must have started its dashboard child: {with_home:?}"
    );

    // Privacy: the front-door token never reaches the log.
    let log = fs::read_to_string(config.log_path.as_ref().expect("log path")).expect("read log");
    assert!(
        log.contains("Hermes Plus login:"),
        "the login line is logged"
    );
    assert!(!log.contains(&token), "the token must not reach the log");
    assert!(log.contains("?token=<redacted>"));

    let report = backend.shutdown().expect("shutdown");
    println!("shutdown: {report:?}");
    assert_eq!(report, ShutdownReport::Terminated);

    assert!(!alive(leader), "the leader survived shutdown");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let members = group_members(pgid);
        let leftovers = procs_with_hermes_home(&data_dir.join("hermes"));
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
        procs_with_hermes_home(&data_dir.join("hermes"))
    );

    println!(
        "numbers: archive={} bytes pack={pack_seconds:.1}s unpack={unpack_seconds:.1}s ready-hit={ready_seconds:.3}s boot={boot_seconds:.1}s",
        manifest.archive_bytes
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives under the workspace root")
        .to_path_buf()
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
    dir
}

/// A fresh scratch tree for the archive and the app-data dir. Overridable so a
/// rerun can keep the previous unpack for inspection.
fn fresh_scratch_dir() -> PathBuf {
    let dir = std::env::var_os("HERMES_TAURI_T16B_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/hermes-tauri-t16b"));
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("clear the scratch dir");
    }
    fs::create_dir_all(&dir).expect("create the scratch dir");
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

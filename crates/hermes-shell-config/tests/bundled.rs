//! The bundled-payload source: source order, the launcher check, the spawn
//! command and its environment, purity, and redaction.
//!
//! Real files in real temp dirs; nothing is executed here. The real-process
//! proof against the 341 MB payload is `tests/bundled_payload.rs`, which is
//! `#[ignore]`d by default.
//!
//! The fixture payload tree marks its launcher executable with a Unix mode,
//! which is what the launcher check tests on Unix; Windows decides by extension
//! (and has no bundled launcher yet), so this target is Unix-only.
#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use hermes_shell_config::{resolve, resolve_with_bundled, Bundled, Inputs, Resolved, Source};
use hermes_sidecar::SidecarConfig;

fn token() -> String {
    "t".repeat(43)
}

fn launch_link() -> String {
    format!("http://127.0.0.1:18888/#hermes-session={}", token())
}

fn os_paths(dirs: &[&Path]) -> OsString {
    std::env::join_paths(dirs).expect("paths join")
}

fn expect_configured(resolved: Resolved) -> (SidecarConfig, Source) {
    match resolved {
        Resolved::Configured { config, source } => (config, source),
        Resolved::NotConfigured => panic!("expected Configured, got NotConfigured"),
    }
}

fn write_config(dir: &Path, body: &str) {
    fs::write(dir.join("config.toml"), body).expect("write config.toml");
}

/// A payload tree shaped like the real one, with a real launcher file at
/// `hermes-payload/bin/hermes-sidecar`. The real tree is 341 MB; `resolve`
/// only looks at that path and its exec bit.
fn payload_tree(root: &Path, executable: bool) -> PathBuf {
    let bin = root.join("hermes-payload").join("bin");
    fs::create_dir_all(&bin).expect("create payload dirs");
    let launcher = bin.join("hermes-sidecar");
    fs::write(&launcher, "#!/bin/sh\nexit 0\n").expect("write launcher");
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(&launcher, fs::Permissions::from_mode(mode)).expect("chmod launcher");
    launcher
}

/// A `hermes` executable for the PATH case.
fn install_hermes(dir: &Path, executable: bool) -> PathBuf {
    let path = dir.join("hermes");
    fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write hermes script");
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("chmod");
    path
}

/// `target` expressed relative to this test process's current directory, which
/// really does reach it from here.
fn relative_from_cwd(target: &Path) -> PathBuf {
    let cwd = std::env::current_dir().expect("current dir");
    let mut cwd_parts = cwd.components();
    let mut target_parts = target.components();
    while let (Some(a), Some(b)) = (cwd_parts.clone().next(), target_parts.clone().next()) {
        if a != b {
            break;
        }
        cwd_parts.next();
        target_parts.next();
    }
    let mut relative = PathBuf::new();
    for _ in cwd_parts {
        relative.push("..");
    }
    relative.extend(target_parts);
    relative
}

struct Fixture {
    _payload: tempfile::TempDir,
    _data: tempfile::TempDir,
    bundled: Bundled,
}

fn fixture(executable: bool) -> (Fixture, PathBuf) {
    let payload = tempfile::tempdir().expect("tempdir");
    let data = tempfile::tempdir().expect("tempdir");
    let launcher = payload_tree(payload.path(), executable);
    let bundled = Bundled {
        payload_dir: payload.path().to_path_buf(),
        data_dir: data.path().to_path_buf(),
    };
    (
        Fixture {
            _payload: payload,
            _data: data,
            bundled,
        },
        launcher,
    )
}

// --- Source order ---

#[test]
fn bundled_payload_beats_hermes_on_path() {
    let (fixture, launcher) = fixture(true);
    let on_path = tempfile::tempdir().expect("tempdir");
    let path_hermes = install_hermes(on_path.path(), true);

    let config_dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(config_dir.path().to_path_buf());
    inputs.path = Some(os_paths(&[on_path.path()]));

    let (config, source) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));
    assert_eq!(source, Source::Bundled);
    assert_eq!(
        config.command.first().map(String::as_str),
        launcher.to_str(),
        "the bundled launcher must win over {path_hermes:?}"
    );
    assert!(config.attach_url.is_none());
    assert!(config.spawn_if_unreachable);
}

#[test]
fn every_higher_source_beats_the_bundled_payload() {
    let (fixture, _launcher) = fixture(true);
    let config_dir = tempfile::tempdir().expect("tempdir");

    // --url
    let mut inputs = Inputs::empty(config_dir.path().to_path_buf());
    inputs.args = vec!["--url".to_owned(), launch_link()];
    let (config, source) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));
    assert_eq!(source, Source::CliArg);
    assert!(config.command.is_empty(), "an attach config spawns nothing");

    // HERMES_TAURI_URL
    let mut inputs = Inputs::empty(config_dir.path().to_path_buf());
    inputs.env_url = Some("http://127.0.0.1:18888".to_owned());
    let (_, source) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));
    assert_eq!(source, Source::EnvUrl);

    // HERMES_TAURI_CMD (+ HERMES_TAURI_ENV)
    let mut inputs = Inputs::empty(config_dir.path().to_path_buf());
    inputs.env_cmd = Some(r#"["/bin/echo","serve"]"#.to_owned());
    inputs.env_json = Some(r#"{"HERMES_HOME":"/tmp/demo"}"#.to_owned());
    let (config, source) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));
    assert_eq!(source, Source::EnvCmd);
    assert_eq!(config.command, vec!["/bin/echo", "serve"]);
    assert_eq!(
        config.env,
        vec![("HERMES_HOME".to_owned(), "/tmp/demo".to_owned())],
        "the bundled env must not be merged into an explicit command"
    );

    // config.toml with a command
    write_config(config_dir.path(), "command = [\"/bin/echo\", \"serve\"]\n");
    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    let (config, source) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));
    assert_eq!(
        source,
        Source::ConfigFile(config_dir.path().join("config.toml"))
    );
    assert_eq!(config.command, vec!["/bin/echo", "serve"]);
    fs::remove_file(config_dir.path().join("config.toml")).expect("remove config.toml");
}

#[test]
fn without_a_bundled_payload_path_still_wins() {
    let on_path = tempfile::tempdir().expect("tempdir");
    let path_hermes = install_hermes(on_path.path(), true);
    let config_dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(config_dir.path().to_path_buf());
    inputs.path = Some(os_paths(&[on_path.path()]));

    let (config, source) = expect_configured(resolve_with_bundled(&inputs, None).expect("resolve"));
    assert_eq!(source, Source::DefaultOnPath(path_hermes.clone()));
    assert_eq!(
        config.command.first().map(String::as_str),
        path_hermes.to_str()
    );
}

// --- The launcher check ---

#[test]
fn a_relative_payload_dir_is_rejected() {
    let (fixture, launcher) = fixture(true);
    let relative = relative_from_cwd(&fixture.bundled.payload_dir);
    assert!(!relative.is_absolute(), "{relative:?} should be relative");
    // Fixture check: the relative path really does reach the executable
    // launcher, so the rejection rests on the absolute-path rule.
    let reached = relative.join("hermes-payload/bin/hermes-sidecar");
    assert!(fs::metadata(&reached).expect("launcher exists").is_file());
    assert_eq!(
        fs::metadata(&reached)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o111,
        0o111
    );
    assert_eq!(reached.file_name(), launcher.file_name());

    let bundled = Bundled {
        payload_dir: relative,
        data_dir: fixture.bundled.data_dir.clone(),
    };
    let config_dir = tempfile::tempdir().expect("tempdir");
    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    assert!(matches!(
        resolve_with_bundled(&inputs, Some(&bundled)).expect("resolve"),
        Resolved::NotConfigured
    ));
}

#[test]
fn a_missing_or_non_executable_launcher_is_rejected() {
    // Missing: a payload dir that exists but has no `hermes-payload/` tree.
    let empty = tempfile::tempdir().expect("tempdir");
    let data = tempfile::tempdir().expect("tempdir");
    let payload_without_launcher = Bundled {
        payload_dir: empty.path().to_path_buf(),
        data_dir: data.path().to_path_buf(),
    };
    let config_dir = tempfile::tempdir().expect("tempdir");
    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    assert!(matches!(
        resolve_with_bundled(&inputs, Some(&payload_without_launcher)).expect("resolve"),
        Resolved::NotConfigured
    ));

    // Present but not executable: the launcher must not be spawned.
    let (fixture, launcher) = fixture(false);
    assert!(launcher.is_file());
    assert_eq!(
        fs::metadata(&launcher)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert!(matches!(
        resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"),
        Resolved::NotConfigured
    ));
}

// --- The spawn command and its environment ---

#[test]
fn the_bundled_command_listens_on_a_free_loopback_port() {
    let (fixture, launcher) = fixture(true);
    let config_dir = tempfile::tempdir().expect("tempdir");
    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    let (config, _) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));

    assert_eq!(config.command.len(), 5);
    assert_eq!(
        config.command[0],
        launcher.to_string_lossy().into_owned(),
        "the launcher runs directly, with no shell in between"
    );
    assert_eq!(config.command[1], "--host");
    assert_eq!(config.command[2], "127.0.0.1");
    assert_eq!(config.command[3], "--port");
    let port: u16 = config.command[4].parse().expect("the port is a number");
    // `hermes_plus` refuses `--port 0`, so the shell picks a port the OS
    // reports free. The real bind is proven by the ignored payload test.
    assert!(port > 0, "port {port} would be rejected by hermes_plus");
    assert_eq!(
        config.ready_timeout,
        std::time::Duration::from_secs(60),
        "untouched fields keep the sidecar defaults"
    );
}

#[test]
fn the_bundled_env_keeps_all_state_under_the_data_dir() {
    let (fixture, _launcher) = fixture(true);
    let data_dir = fixture.bundled.data_dir.clone();
    let config_dir = tempfile::tempdir().expect("tempdir");
    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    let (config, _) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));

    assert_eq!(
        config.env,
        vec![
            (
                "HERMES_HOME".to_owned(),
                data_dir.join("hermes").to_string_lossy().into_owned()
            ),
            (
                "HERMES_RUNTIME_DIR".to_owned(),
                data_dir.join("runtime").to_string_lossy().into_owned()
            ),
            ("HERMES_DISABLE_LAZY_INSTALLS".to_owned(), "1".to_owned()),
            ("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned()),
        ]
    );
    for (key, value) in &config.env {
        if key == "PYTHONDONTWRITEBYTECODE" || key == "HERMES_DISABLE_LAZY_INSTALLS" {
            continue;
        }
        assert!(
            Path::new(value).starts_with(&data_dir),
            "{key}={value} must be inside the app-data dir"
        );
        assert!(
            !Path::new(value).starts_with(&fixture.bundled.payload_dir),
            "{key}={value} must never point into the read-only payload"
        );
    }
}

#[test]
fn resolve_creates_nothing_on_disk() {
    let (fixture, _launcher) = fixture(true);
    let data_dir = fixture.bundled.data_dir.clone();
    // The fixture's data dir is empty before and after: `resolve` only decides.
    let before = fs::read_dir(&data_dir).expect("read data dir").count();
    let config_dir = tempfile::tempdir().expect("tempdir");
    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    let resolved = resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve");
    assert!(matches!(resolved, Resolved::Configured { .. }));
    assert_eq!(
        fs::read_dir(&data_dir).expect("read data dir").count(),
        before
    );
    assert!(!data_dir.join("hermes").exists());
    assert!(!data_dir.join("runtime").exists());
    assert!(!config_dir.path().join("config.toml").exists());
}

#[test]
fn resolve_without_a_bundled_payload_matches_resolve() {
    let (fixture, _launcher) = fixture(true);
    let config_dir = tempfile::tempdir().expect("tempdir");
    write_config(config_dir.path(), "command = [\"/bin/echo\", \"serve\"]\n");
    let inputs = Inputs::empty(config_dir.path().to_path_buf());

    let plain = expect_configured(resolve(&inputs).expect("resolve"));
    let explicit = expect_configured(resolve_with_bundled(&inputs, None).expect("resolve"));
    assert_eq!(plain.1, explicit.1);
    assert_eq!(plain.0.command, explicit.0.command);
    assert_eq!(plain.0.env, explicit.0.env);

    // With the payload it still resolves the same way: the file's command wins.
    let bundled =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));
    assert_eq!(bundled.1, plain.1);
    assert_eq!(bundled.0.command, plain.0.command);
}

// --- A config file that only attaches ---

#[test]
fn a_config_file_url_without_a_command_spawns_the_bundled_payload() {
    let (fixture, launcher) = fixture(true);
    let config_dir = tempfile::tempdir().expect("tempdir");
    write_config(config_dir.path(), &format!("url = \"{}\"\n", launch_link()));
    let inputs = Inputs::empty(config_dir.path().to_path_buf());

    let (config, source) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));
    assert_eq!(
        source,
        Source::ConfigFile(config_dir.path().join("config.toml")),
        "the file still decides; the payload only fills the missing command"
    );
    assert!(config.attach_url.is_some());
    assert!(config.spawn_if_unreachable);
    assert_eq!(
        config.command.first().map(String::as_str),
        launcher.to_str(),
        "the fallback is the bundled payload instead of NoCommand"
    );
    assert_eq!(config.env.len(), 4, "the bundled env comes with it");

    // Without a payload the file keeps today's shape: attach, nothing to spawn.
    let (config, _) = expect_configured(resolve_with_bundled(&inputs, None).expect("resolve"));
    assert!(config.attach_url.is_some());
    assert!(config.command.is_empty());
    assert!(config.env.is_empty());

    // A file that opts out of spawning gets no command either way.
    write_config(
        config_dir.path(),
        &format!(
            "url = \"{}\"\nspawn_if_unreachable = false\n",
            launch_link()
        ),
    );
    let (config, _) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));
    assert!(!config.spawn_if_unreachable);
    assert!(config.command.is_empty());
}

#[test]
fn a_config_file_env_overrides_the_bundled_env() {
    let (fixture, _launcher) = fixture(true);
    let data_dir = fixture.bundled.data_dir.clone();
    let config_dir = tempfile::tempdir().expect("tempdir");
    write_config(
        config_dir.path(),
        &format!(
            "url = \"{}\"\nenv = {{ HERMES_HOME = \"/tmp/demo-home\" }}\n",
            launch_link()
        ),
    );
    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    let (config, _) =
        expect_configured(resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve"));

    assert_eq!(
        config.env,
        vec![
            ("HERMES_HOME".to_owned(), "/tmp/demo-home".to_owned()),
            (
                "HERMES_RUNTIME_DIR".to_owned(),
                data_dir.join("runtime").to_string_lossy().into_owned()
            ),
            ("HERMES_DISABLE_LAZY_INSTALLS".to_owned(), "1".to_owned()),
            ("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned()),
        ],
        "the file's HERMES_HOME replaces the bundled one, with no duplicate key"
    );
}

// --- Redaction ---

#[test]
fn debug_of_a_bundled_config_shows_env_keys_only() {
    let (fixture, _launcher) = fixture(true);
    let data_dir = fixture.bundled.data_dir.clone();
    let config_dir = tempfile::tempdir().expect("tempdir");
    let inputs = Inputs::empty(config_dir.path().to_path_buf());
    let resolved = resolve_with_bundled(&inputs, Some(&fixture.bundled)).expect("resolve");

    let debug = format!("{resolved:?}");
    assert!(debug.contains("Bundled"), "source is visible: {debug}");
    assert!(
        debug.contains("HERMES_HOME"),
        "env keys are visible: {debug}"
    );
    assert!(
        !debug.contains(&data_dir.to_string_lossy().into_owned()),
        "env values must not appear: {debug}"
    );

    // `Bundled` itself carries directories only, which the boot page may show.
    let debug = format!("{:?}", fixture.bundled);
    assert!(debug.contains(&fixture.bundled.payload_dir.to_string_lossy().into_owned()));
    assert!(debug.contains(&data_dir.to_string_lossy().into_owned()));
}

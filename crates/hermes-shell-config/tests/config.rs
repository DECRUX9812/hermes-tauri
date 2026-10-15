//! Source order, PATH lookup, config file parsing and error typing for
//! `resolve`. Everything runs against real temp dirs and real files; the
//! spawned-command values are plain data (nothing is executed here).

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hermes_shell_config::{resolve, ConfigError, Inputs, Resolved, Source};
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

fn expect_error(resolved: Result<Resolved, ConfigError>) -> ConfigError {
    resolved.expect_err("expected an error")
}

fn write_config(dir: &Path, body: &str) {
    fs::write(dir.join("config.toml"), body).expect("write config.toml");
}

// --- Source order ---

#[test]
fn cli_flag_beats_env_file_and_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path(), "command = [\"/bin/echo\", \"from-file\"]");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.args = vec!["--url".to_owned(), launch_link()];
    inputs.env_url = Some("http://127.0.0.1:18889".to_owned());
    inputs.env_cmd = Some("[\"/bin/echo\"]".to_owned());
    inputs.path = Some(os_paths(&[dir.path()]));

    let (config, source) = expect_configured(resolve(&inputs).expect("resolve"));
    assert_eq!(config.attach_url.as_deref(), Some(launch_link().as_str()));
    assert_eq!(config.command, Vec::<String>::new());
    assert_eq!(source, Source::CliArg);
}

#[test]
fn env_url_beats_file_and_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path(), "command = [\"/bin/echo\", \"from-file\"]");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.env_url = Some(format!(
        "{}/#hermes-session={}",
        "http://127.0.0.1:18890",
        token()
    ));
    inputs.env_cmd = Some("[\"/bin/echo\"]".to_owned());
    inputs.path = Some(os_paths(&[dir.path()]));

    let (config, source) = expect_configured(resolve(&inputs).expect("resolve"));
    assert_eq!(source, Source::EnvUrl);
    assert!(config.attach_url.unwrap().ends_with(&token()));
}

#[test]
fn env_cmd_beats_file_and_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path(), "command = [\"/bin/echo\", \"from-file\"]");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.env_cmd = Some("[\"/bin/echo\", \"webapp\"]".to_owned());
    inputs.env_json = Some("{\"HERMES_HOME\": \"/tmp/demo\"}".to_owned());
    inputs.path = Some(os_paths(&[dir.path()]));

    let (config, source) = expect_configured(resolve(&inputs).expect("resolve"));
    assert_eq!(config.command, vec!["/bin/echo", "webapp"]);
    assert_eq!(
        config.env,
        vec![("HERMES_HOME".to_owned(), "/tmp/demo".to_owned())]
    );
    assert_eq!(config.attach_url, None);
    assert_eq!(source, Source::EnvCmd);
}

#[test]
fn config_file_beats_path_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path(), "command = [\"/bin/echo\", \"from-file\"]");
    let bin = tempfile::tempdir().expect("tempdir");
    install_hermes(bin.path(), true);
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.path = Some(os_paths(&[bin.path()]));

    let (config, source) = expect_configured(resolve(&inputs).expect("resolve"));
    assert_eq!(config.command, vec!["/bin/echo", "from-file"]);
    assert_eq!(source, Source::ConfigFile(dir.path().join("config.toml")));
}

#[test]
fn earlier_cli_url_short_circuits_a_later_invalid_env_cmd() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.args = vec![format!("--url={}", launch_link())];
    inputs.env_cmd = Some("not json at all".to_owned());
    assert!(matches!(
        resolve(&inputs),
        Ok(Resolved::Configured {
            source: Source::CliArg,
            ..
        })
    ));
}

// --- Config file ---

#[test]
fn config_file_supports_every_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(
        dir.path(),
        &format!(
            "url = \"http://127.0.0.1:18891/#hermes-session={}\"\n\
             command = [\"/bin/echo\", \"serve\"]\n\
             env = {{ HERMES_HOME = \"/tmp/demo\" }}\n\
             spawn_if_unreachable = false\n\
             hotkey = \"Ctrl+Shift+H\"\n",
            token()
        ),
    );
    let inputs = Inputs::empty(dir.path().to_path_buf());

    let (config, source) = expect_configured(resolve(&inputs).expect("resolve"));
    assert!(config.attach_url.unwrap().ends_with(&token()));
    assert_eq!(config.command, vec!["/bin/echo", "serve"]);
    assert_eq!(
        config.env,
        vec![("HERMES_HOME".to_owned(), "/tmp/demo".to_owned())]
    );
    assert!(!config.spawn_if_unreachable);
    // Untouched fields keep the sidecar defaults.
    assert_eq!(config.ready_timeout, Duration::from_secs(60));
    assert_eq!(config.shutdown_grace, Duration::from_secs(5));
    assert_eq!(source, Source::ConfigFile(dir.path().join("config.toml")));
}

#[test]
fn config_file_defaults_spawn_if_unreachable_to_true() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path(), "command = [\"/bin/echo\", \"serve\"]");
    let inputs = Inputs::empty(dir.path().to_path_buf());
    let (config, _) = expect_configured(resolve(&inputs).expect("resolve"));
    assert!(config.spawn_if_unreachable);
}

#[test]
fn missing_config_file_is_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let inputs = Inputs::empty(dir.path().to_path_buf());
    assert!(matches!(resolve(&inputs), Ok(Resolved::NotConfigured)));
}

#[test]
fn config_file_with_only_a_hotkey_falls_through() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path(), "hotkey = \"Ctrl+Shift+H\"");
    let bin = tempfile::tempdir().expect("tempdir");
    install_hermes(bin.path(), true);
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.path = Some(os_paths(&[bin.path()]));
    assert!(matches!(
        resolve(&inputs),
        Ok(Resolved::Configured {
            source: Source::DefaultOnPath(_),
            ..
        })
    ));
}

#[test]
fn unknown_config_key_names_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(
        dir.path(),
        "command = [\"/bin/echo\"]\nknown_extra = true\n",
    );
    let inputs = Inputs::empty(dir.path().to_path_buf());
    match expect_error(resolve(&inputs)) {
        ConfigError::ConfigFile { path, .. } => {
            assert_eq!(path, dir.path().join("config.toml"));
        }
        other => panic!("expected ConfigFile, got {other:?}"),
    }
}

#[test]
fn broken_toml_and_wrong_types_are_config_file_errors() {
    let broken = tempfile::tempdir().expect("tempdir");
    write_config(broken.path(), "command = [\"/bin/echo\"\n");
    let inputs = Inputs::empty(broken.path().to_path_buf());
    assert!(matches!(
        expect_error(resolve(&inputs)),
        ConfigError::ConfigFile { .. }
    ));

    let wrong_type = tempfile::tempdir().expect("tempdir");
    write_config(wrong_type.path(), "command = \"/bin/echo\"\n");
    let inputs = Inputs::empty(wrong_type.path().to_path_buf());
    assert!(matches!(
        expect_error(resolve(&inputs)),
        ConfigError::ConfigFile { .. }
    ));
}

#[test]
fn unreadable_config_file_is_a_typed_io_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    write_config(dir.path(), "command = [\"/bin/echo\"]");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod 000");
    if fs::read_to_string(&path).is_ok() {
        // Running with privileges that ignore the exec-bit trick; skip.
        return;
    }
    let inputs = Inputs::empty(dir.path().to_path_buf());
    assert!(matches!(
        expect_error(resolve(&inputs)),
        ConfigError::ConfigFileIo { .. }
    ));
}

// --- Typed env errors ---

#[test]
fn invalid_cmd_json_is_typed() {
    let dir = tempfile::tempdir().expect("tempdir");
    for bad in ["not json", "\"hermes\"", "[\"a\", 1]"] {
        let mut inputs = Inputs::empty(dir.path().to_path_buf());
        inputs.env_cmd = Some(bad.to_owned());
        assert!(
            matches!(resolve(&inputs), Err(ConfigError::InvalidCmdJson(_))),
            "expected InvalidCmdJson for {bad}"
        );
    }
}

#[test]
fn empty_cmd_array_is_empty_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.env_cmd = Some("[]".to_owned());
    assert!(matches!(resolve(&inputs), Err(ConfigError::EmptyCommand)));
}

#[test]
fn invalid_env_json_is_typed() {
    let dir = tempfile::tempdir().expect("tempdir");
    for bad in ["[1, 2]", "{\"A\": 1}", "\"x\""] {
        let mut inputs = Inputs::empty(dir.path().to_path_buf());
        inputs.env_cmd = Some("[\"/bin/echo\"]".to_owned());
        inputs.env_json = Some(bad.to_owned());
        assert!(
            matches!(resolve(&inputs), Err(ConfigError::InvalidEnvJson(_))),
            "expected InvalidEnvJson for {bad}"
        );
    }
}

#[test]
fn env_json_without_env_cmd_is_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.env_json = Some("not json".to_owned());
    assert!(matches!(resolve(&inputs), Ok(Resolved::NotConfigured)));
}

// --- Invalid URLs ---

#[test]
fn invalid_urls_are_typed_and_redacted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cases = [
        "--url=ftp://127.0.0.1:18892".to_owned(),
        "--url=not-a-url".to_owned(),
        // A link-shaped URL with a broken session token.
        "--url=http://127.0.0.1:18892/#hermes-session=short".to_owned(),
    ];
    for case in cases {
        let mut inputs = Inputs::empty(dir.path().to_path_buf());
        inputs.args = vec![case];
        let error = expect_error(resolve(&inputs));
        assert!(matches!(error, ConfigError::InvalidUrl(_)), "got {error}");
        assert!(!format!("{error:?}").contains("hermes-session=t"));
    }

    let mut flag_without_value = Inputs::empty(dir.path().to_path_buf());
    flag_without_value.args = vec!["--url".to_owned()];
    assert!(matches!(
        expect_error(resolve(&flag_without_value)),
        ConfigError::InvalidUrl(_)
    ));

    let mut env = Inputs::empty(dir.path().to_path_buf());
    env.env_url = Some("ftp://127.0.0.1:18892".to_owned());
    assert!(matches!(
        expect_error(resolve(&env)),
        ConfigError::InvalidUrl(_)
    ));
}

// --- PATH default ---

fn install_hermes(dir: &Path, executable: bool) -> PathBuf {
    let path = dir.join("hermes");
    fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write hermes script");
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("chmod");
    path
}

#[test]
fn default_finds_a_real_executable_on_a_temp_path() {
    let decoy = tempfile::tempdir().expect("tempdir");
    let real = tempfile::tempdir().expect("tempdir");
    install_hermes(decoy.path(), false);
    let real_path = install_hermes(real.path(), true);

    let dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.path = Some(os_paths(&[decoy.path(), real.path()]));

    let (config, source) = expect_configured(resolve(&inputs).expect("resolve"));
    assert_eq!(
        config.command,
        vec![
            real_path.to_string_lossy().into_owned(),
            "webapp".to_owned(),
            "--isolated".to_owned(),
            "--host".to_owned(),
            "127.0.0.1".to_owned(),
            "--port".to_owned(),
            "0".to_owned(),
            "--skip-build".to_owned(),
        ]
    );
    assert_eq!(source, Source::DefaultOnPath(real_path));
    assert!(config.attach_url.is_none());
}

#[test]
fn default_skips_directories_and_missing_entries() {
    let dir_as_file = tempfile::tempdir().expect("tempdir");
    let real = tempfile::tempdir().expect("tempdir");
    let real_path = install_hermes(real.path(), true);

    let dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    // A PATH entry that is a plain file, one that does not exist, then the real one.
    inputs.path = Some(os_paths(&[
        dir_as_file.path(),
        Path::new("/nonexistent-hermes-path"),
        real.path(),
    ]));
    let (config, source) = expect_configured(resolve(&inputs).expect("resolve"));
    assert_eq!(source, Source::DefaultOnPath(real_path));
    assert!(config.attach_url.is_none());
}

#[test]
fn empty_or_missing_path_is_not_configured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let inputs = Inputs::empty(dir.path().to_path_buf());
    assert!(matches!(resolve(&inputs), Ok(Resolved::NotConfigured)));

    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.path = Some(OsString::new());
    assert!(matches!(resolve(&inputs), Ok(Resolved::NotConfigured)));

    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.path = Some(OsString::from("/nonexistent-hermes-path"));
    assert!(matches!(resolve(&inputs), Ok(Resolved::NotConfigured)));
}

// --- Redaction ---

#[test]
fn debug_of_a_resolved_attach_config_hides_the_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.args = vec!["--url".to_owned(), launch_link()];
    let resolved = resolve(&inputs).expect("resolve");
    let debug = format!("{resolved:?}");
    assert!(!debug.contains(&token()));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn debug_of_inputs_hides_the_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut inputs = Inputs::empty(dir.path().to_path_buf());
    inputs.args = vec![format!("--url={}", launch_link())];
    inputs.env_url = Some(launch_link());
    inputs.env_json = Some("{\"SECRET_KEY\": \"shh\"}".to_owned());
    let debug = format!("{inputs:?}");
    assert!(!debug.contains(&token()));
    assert!(!debug.contains("shh"));
    assert!(debug.contains("SECRET_KEY"));
}

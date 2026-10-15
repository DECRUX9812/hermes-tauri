use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hermes_shell_config::{Bundled, Inputs, Resolved, Source, UpdaterSettings};
use hermes_sidecar::SidecarConfig;

/// What the tray, the global hotkey and the updater need. None of it depends
/// on where the backend comes from, so setup reads it straight away — even
/// when a bundled payload still has to be unpacked before there is anything to
/// connect to.
pub struct Preferences {
    /// The global hotkey accelerator, `None` when disabled.
    pub hotkey: Option<String>,
    /// Whether the updater plugin runs, from the build environment.
    pub updater: UpdaterSettings,
}

/// How long the bundled payload may take to become ready.
///
/// Its first run unpacks the archive (13.9 s measured on the 341 MB payload)
/// and then pays the payload's own cold start (14-53 s, mostly the stock
/// Hermes dashboard child), so the sidecar's 60 s default does not cover both
/// (docs/python-sidecar.md §5.8).
const BUNDLED_READY_TIMEOUT: Duration = Duration::from_secs(90);

/// Reads the hotkey and updater settings from the app config directory.
pub fn preferences(app_config_dir: Option<PathBuf>) -> Preferences {
    let config_file = app_config_dir.map(|dir| dir.join("config.toml"));
    Preferences {
        hotkey: hermes_shell_config::hotkey(config_file.as_deref().and_then(|path| path.to_str())),
        updater: hermes_shell_config::settings(
            option_env!("TAURI_UPDATER_PUBKEY"),
            option_env!("TAURI_UPDATER_ENDPOINT"),
        ),
    }
}

/// Resolves where the Hermes backend comes from. `bundled` is the payload this
/// app ships, already unpacked, or `None` for a build without one — the
/// resolver then falls back to the environment, `config.toml` and `hermes` on
/// PATH exactly as it did before the payload existed.
pub fn load(
    app_config_dir: Option<PathBuf>,
    bundled: Option<&Bundled>,
) -> Result<Resolved, String> {
    let inputs = Inputs {
        args: env::args().skip(1).collect(),
        env_url: env::var("HERMES_TAURI_URL").ok(),
        env_cmd: env::var("HERMES_TAURI_CMD").ok(),
        env_json: env::var("HERMES_TAURI_ENV").ok(),
        // Without a config directory there is no config file: point the lookup
        // at a path that is never a file so it reads as absent, and env/args
        // still work.
        config_dir: app_config_dir.unwrap_or_else(|| PathBuf::from("/")),
        path: env::var_os("PATH"),
    };
    resolve(&inputs, bundled)
}

/// The resolution itself, from plain inputs: no environment and no app handle,
/// so it is testable on the host.
fn resolve(inputs: &Inputs, bundled: Option<&Bundled>) -> Result<Resolved, String> {
    let resolved = hermes_shell_config::resolve_with_bundled(inputs, bundled)
        .map_err(|error| error.to_string())?;
    Ok(with_bundled_timeout(resolved, bundled))
}

/// A short label for where the backend came from, for the shell's log. It
/// carries no path, so a config or PATH location cannot leak into a log.
pub fn source_label(source: &Source) -> &'static str {
    match source {
        Source::CliArg => "the --url flag",
        Source::EnvUrl => "HERMES_TAURI_URL",
        Source::EnvCmd => "HERMES_TAURI_CMD",
        Source::ConfigFile(_) => "config.toml",
        Source::Bundled => "the bundled payload",
        Source::DefaultOnPath(_) => "hermes on PATH",
    }
}

/// Gives a spawn of the bundled launcher the payload's longer readiness budget
/// (see [`BUNDLED_READY_TIMEOUT`]). The command is what decides, not the
/// source: a `config.toml` that names a URL and no command is filled in with
/// the bundled launcher too (`resolve_with_bundled`), and it is just as slow.
fn with_bundled_timeout(resolved: Resolved, bundled: Option<&Bundled>) -> Resolved {
    match resolved {
        Resolved::Configured { mut config, source } => {
            if let Some(bundled) = bundled {
                if spawns_the_payload(&config, bundled) {
                    config.ready_timeout = config.ready_timeout.max(BUNDLED_READY_TIMEOUT);
                }
            }
            Resolved::Configured { config, source }
        }
        Resolved::NotConfigured => Resolved::NotConfigured,
    }
}

/// Whether the command the resolver produced runs the bundled launcher.
fn spawns_the_payload(config: &SidecarConfig, bundled: &Bundled) -> bool {
    config
        .command
        .first()
        .is_some_and(|program| Path::new(program).starts_with(&bundled.payload_dir))
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use hermes_shell_config::{Bundled, Inputs, Resolved, Source};
    use hermes_sidecar::SidecarConfig;

    use super::{resolve, source_label, with_bundled_timeout, BUNDLED_READY_TIMEOUT};

    /// A real directory tree with the launcher the resolver looks for, so
    /// `resolve_with_bundled` really picks the bundled source.
    struct StagedPayload {
        root: PathBuf,
        bundled: Bundled,
    }

    impl StagedPayload {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hermes-tauri-config-test-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let payload_dir = root.join("resources");
            let launcher = payload_dir.join("hermes-payload/bin/hermes-sidecar");
            fs::create_dir_all(launcher.parent().expect("launcher dir")).expect("create payload");
            fs::write(&launcher, "#!/bin/sh\nexit 0\n").expect("write launcher");
            fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).expect("chmod");
            Self {
                root: root.clone(),
                bundled: Bundled {
                    payload_dir,
                    data_dir: root.join("data"),
                },
            }
        }
    }

    impl Drop for StagedPayload {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn load_without_a_payload_behaves_as_it_did_before_the_payload_existed() {
        // `Inputs::empty` is exactly the state of a fresh install with nothing
        // configured: no flag, no environment, no config file, no PATH. The
        // resolver reports "not configured", and the shell shows the boot
        // page's no-config state rather than a spawn error.
        let inputs = Inputs::empty(PathBuf::from("/"));
        match resolve(&inputs, None).expect("resolution") {
            Resolved::NotConfigured => {}
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }

    #[test]
    fn load_with_a_staged_payload_picks_the_bundled_source_and_its_timeout() {
        let staged = StagedPayload::new("bundled");
        let inputs = Inputs::empty(PathBuf::from("/"));
        let resolved = resolve(&inputs, Some(&staged.bundled)).expect("resolution");
        let Resolved::Configured { config, source } = resolved else {
            panic!("the staged payload was not picked");
        };
        assert!(matches!(source, Source::Bundled));
        let launcher = config.command.first().expect("launcher");
        assert!(
            Path::new(launcher).starts_with(&staged.bundled.payload_dir),
            "the command does not run the staged launcher: {launcher}"
        );
        assert_eq!(config.ready_timeout, BUNDLED_READY_TIMEOUT);
    }

    #[test]
    fn a_bundled_spawn_gets_the_payload_ready_timeout() {
        let staged = StagedPayload::new("timeout");
        let launcher = staged
            .bundled
            .payload_dir
            .join("hermes-payload/bin/hermes-sidecar")
            .to_string_lossy()
            .into_owned();

        let bundled_config = |command: Vec<String>| Resolved::Configured {
            config: SidecarConfig {
                command,
                ..SidecarConfig::default()
            },
            source: Source::Bundled,
        };

        // The payload launcher: the longer budget.
        match with_bundled_timeout(
            bundled_config(vec![launcher.clone()]),
            Some(&staged.bundled),
        ) {
            Resolved::Configured { config, .. } => {
                assert_eq!(config.ready_timeout, BUNDLED_READY_TIMEOUT)
            }
            Resolved::NotConfigured => panic!("the config was dropped"),
        }

        // The same command without a payload in play keeps the default.
        match with_bundled_timeout(bundled_config(vec![launcher]), None) {
            Resolved::Configured { config, .. } => {
                assert_eq!(config.ready_timeout, SidecarConfig::default().ready_timeout)
            }
            Resolved::NotConfigured => panic!("the config was dropped"),
        }

        // A command that is not the payload keeps its own budget even though a
        // payload exists (it is only the fallback here).
        match with_bundled_timeout(
            bundled_config(vec!["/usr/bin/hermes".to_owned(), "webapp".to_owned()]),
            Some(&staged.bundled),
        ) {
            Resolved::Configured { config, .. } => {
                assert_eq!(config.ready_timeout, SidecarConfig::default().ready_timeout)
            }
            Resolved::NotConfigured => panic!("the config was dropped"),
        }
    }

    #[test]
    fn source_labels_name_no_path() {
        let labels = [
            source_label(&Source::CliArg),
            source_label(&Source::EnvUrl),
            source_label(&Source::EnvCmd),
            source_label(&Source::ConfigFile(PathBuf::from("/home/demo/config.toml"))),
            source_label(&Source::Bundled),
            source_label(&Source::DefaultOnPath(PathBuf::from("/usr/bin/hermes"))),
        ];
        for label in labels {
            assert!(!label.contains('/'), "label leaks a path: {label}");
        }
    }
}

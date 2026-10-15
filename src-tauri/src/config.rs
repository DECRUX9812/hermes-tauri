use std::env;
use std::path::PathBuf;

use hermes_shell_config::{Inputs, Resolved, UpdaterSettings};

/// Everything the shell needs from its configuration sources, resolved once at
/// startup. The typed resolution lives in `hermes-shell-config`; this module
/// only collects the real inputs and carries the results.
pub struct ShellSettings {
    /// How to get a Hermes backend, or the error that stopped the lookup.
    pub resolved: Result<Resolved, String>,
    /// The global hotkey accelerator, `None` when disabled.
    pub hotkey: Option<String>,
    /// Whether the updater plugin runs, from the build environment.
    pub updater: UpdaterSettings,
}

pub fn load(app_config_dir: Option<PathBuf>) -> ShellSettings {
    // Without a config directory there is no config file: point the lookup at
    // a path that is never a file so it reads as absent, and env/args work.
    let config_dir = app_config_dir.clone().unwrap_or_else(|| PathBuf::from("/"));
    let config_file = app_config_dir.map(|dir| dir.join("config.toml"));

    let inputs = Inputs {
        args: env::args().skip(1).collect(),
        env_url: env::var("HERMES_TAURI_URL").ok(),
        env_cmd: env::var("HERMES_TAURI_CMD").ok(),
        env_json: env::var("HERMES_TAURI_ENV").ok(),
        config_dir,
        path: env::var_os("PATH"),
    };

    ShellSettings {
        resolved: hermes_shell_config::resolve(&inputs).map_err(|error| error.to_string()),
        hotkey: hermes_shell_config::hotkey(config_file.as_deref().and_then(|path| path.to_str())),
        updater: hermes_shell_config::settings(
            option_env!("TAURI_UPDATER_PUBKEY"),
            option_env!("TAURI_UPDATER_ENDPOINT"),
        ),
    }
}

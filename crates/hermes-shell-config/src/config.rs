use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use hermes_sidecar::{is_attach_url, redact, SidecarConfig};
use serde::Deserialize;

/// Where a resolved configuration came from, so the boot page can say so.
#[derive(Clone, PartialEq, Eq)]
pub enum Source {
    /// The `--url` command line flag.
    CliArg,
    /// The `HERMES_TAURI_URL` environment variable.
    EnvUrl,
    /// The `HERMES_TAURI_CMD` environment variable.
    EnvCmd,
    /// The config file at this path.
    ConfigFile(PathBuf),
    /// A `hermes` executable found at this path on PATH.
    DefaultOnPath(PathBuf),
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::CliArg => f.write_str("CliArg"),
            Source::EnvUrl => f.write_str("EnvUrl"),
            Source::EnvCmd => f.write_str("EnvCmd"),
            Source::ConfigFile(path) => f.debug_tuple("ConfigFile").field(path).finish(),
            Source::DefaultOnPath(path) => f.debug_tuple("DefaultOnPath").field(path).finish(),
        }
    }
}

/// What `resolve` decided.
#[derive(Debug)]
pub enum Resolved {
    Configured {
        config: SidecarConfig,
        source: Source,
    },
    /// No source produced anything: no flag, no env, no config file, no
    /// `hermes` on PATH.
    NotConfigured,
}

/// The plain values the shell collects at startup. Passed by value so tests
/// never touch the real environment.
pub struct Inputs {
    /// Process arguments, scanned for `--url <link>` / `--url=<link>`.
    pub args: Vec<String>,
    /// `HERMES_TAURI_URL`
    pub env_url: Option<String>,
    /// `HERMES_TAURI_CMD`, a JSON array of strings.
    pub env_cmd: Option<String>,
    /// `HERMES_TAURI_ENV`, a JSON object of string to string.
    pub env_json: Option<String>,
    /// The app config directory; `config.toml` is read from here.
    pub config_dir: PathBuf,
    /// The PATH string used to find a default `hermes` executable.
    pub path: Option<OsString>,
}

impl Inputs {
    pub fn empty(config_dir: PathBuf) -> Self {
        Self {
            args: Vec::new(),
            env_url: None,
            env_cmd: None,
            env_json: None,
            config_dir,
            path: None,
        }
    }
}

impl fmt::Debug for Inputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inputs")
            .field("args", &redact_argv(&self.args))
            .field("env_url", &self.env_url.as_deref().map(redact))
            .field("env_cmd", &self.env_cmd.as_deref().map(command_json_debug))
            .field("env_json", &self.env_json.as_deref().map(env_keys_only))
            .field("config_dir", &self.config_dir)
            .field("path", &self.path.as_ref().map(|p| p.to_string_lossy()))
            .finish()
    }
}

const REDACTED: &str = "<redacted>";

/// Debug view of an argv: a command line can carry a secret in a
/// `#hermes-session=` token or behind a flag that names one, so every element
/// goes through [`redact`] and the value of a secret-naming flag is dropped
/// entirely (the flag itself stays, so the line is still readable).
fn redact_argv(argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len());
    let mut drop_next = false;
    for arg in argv {
        if drop_next {
            drop_next = false;
            out.push(REDACTED.to_owned());
            continue;
        }
        if let Some((flag, _)) = arg.split_once('=') {
            if flag.starts_with('-') && names_a_secret(flag) {
                out.push(format!("{flag}={REDACTED}"));
                continue;
            }
        } else if arg.starts_with('-') && names_a_secret(arg) {
            drop_next = true;
        }
        out.push(redact(arg));
    }
    out
}

/// Debug view of `HERMES_TAURI_CMD`: the parsed argv, redacted. An unparsable
/// value is not echoed, since it could be anything.
fn command_json_debug(json: &str) -> String {
    match serde_json::from_str::<Vec<String>>(json) {
        Ok(argv) => format!("{:?}", redact_argv(&argv)),
        Err(_) => "<not a JSON array>".to_owned(),
    }
}

fn names_a_secret(flag: &str) -> bool {
    let flag = flag.to_ascii_lowercase();
    ["token", "key", "secret", "password"]
        .iter()
        .any(|needle| flag.contains(needle))
}

/// Env values can hold secrets, so Debug shows their keys only.
fn env_keys_only(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            let object = value.as_object()?;
            let keys: Vec<&String> = object.keys().collect();
            Some(format!("keys {keys:?}"))
        })
        .unwrap_or_else(|| "<not a JSON object>".to_owned())
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("HERMES_TAURI_CMD / config `command` must be a JSON array of strings: {0}")]
    InvalidCmdJson(String),
    #[error("HERMES_TAURI_CMD / config `command` is an empty command")]
    EmptyCommand,
    #[error("HERMES_TAURI_ENV / config `env` must be a JSON object of string to string: {0}")]
    InvalidEnvJson(String),
    #[error("Could not parse the config file {}: {source}", .path.display())]
    ConfigFile {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("Could not read the config file {}: {source}", .path.display())]
    ConfigFileIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Invalid Hermes URL: {0}")]
    InvalidUrl(String),
}

/// Arguments after the program for the spawned default instance, so a stray
/// production Hermes is never touched.
const DEFAULT_COMMAND_ARGS: [&str; 7] = [
    "webapp",
    "--isolated",
    "--host",
    "127.0.0.1",
    "--port",
    "0",
    "--skip-build",
];

#[cfg(windows)]
const EXECUTABLE_NAMES: [&str; 2] = ["hermes.exe", "hermes.cmd"];
#[cfg(not(windows))]
const EXECUTABLE_NAMES: [&str; 1] = ["hermes"];

/// Works out how the shell should get its Hermes backend.
///
/// Source order, first match wins: the `--url` flag, then `HERMES_TAURI_URL`
/// (both attach), then `HERMES_TAURI_CMD` (spawn), then `config.toml`, then a
/// `hermes` executable on PATH (spawn), then nothing.
pub fn resolve(inputs: &Inputs) -> Result<Resolved, ConfigError> {
    if let Some(url) = cli_url(&inputs.args)? {
        return attach(url, Source::CliArg);
    }
    if let Some(url) = present(inputs.env_url.as_deref()) {
        return attach(url.to_owned(), Source::EnvUrl);
    }
    if let Some(cmd_json) = present(inputs.env_cmd.as_deref()) {
        let command = parse_command(cmd_json)?;
        let env = parse_env_json(present(inputs.env_json.as_deref()))?;
        return Ok(Resolved::Configured {
            config: SidecarConfig {
                command,
                env,
                ..SidecarConfig::default()
            },
            source: Source::EnvCmd,
        });
    }

    let file_path = inputs.config_dir.join("config.toml");
    if let Some(file) = read_config_file(&file_path)? {
        if let Some(config) = config_from_file(file)? {
            return Ok(Resolved::Configured {
                config,
                source: Source::ConfigFile(file_path),
            });
        }
        // The file parsed but names neither a URL nor a command; treat it as
        // no configuration and fall through to the PATH default.
    }

    if let Some(program) = find_on_path(inputs.path.as_deref()) {
        let mut command = vec![program.to_string_lossy().into_owned()];
        command.extend(DEFAULT_COMMAND_ARGS.map(str::to_owned));
        return Ok(Resolved::Configured {
            config: SidecarConfig {
                command,
                ..SidecarConfig::default()
            },
            source: Source::DefaultOnPath(program),
        });
    }

    Ok(Resolved::NotConfigured)
}

fn attach(url: String, source: Source) -> Result<Resolved, ConfigError> {
    let url = url.trim().to_owned();
    if !is_attach_url(&url) {
        return Err(ConfigError::InvalidUrl(redact(&url)));
    }
    Ok(Resolved::Configured {
        config: SidecarConfig {
            attach_url: Some(url),
            ..SidecarConfig::default()
        },
        source,
    })
}

/// Finds the first `--url <value>` or `--url=<value>` in the arguments.
fn cli_url(args: &[String]) -> Result<Option<String>, ConfigError> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--url" {
            let value = args.get(index + 1).ok_or_else(|| {
                ConfigError::InvalidUrl("The --url flag is missing its value".to_owned())
            })?;
            return Ok(Some(value.to_owned()));
        }
        if let Some(value) = arg.strip_prefix("--url=") {
            return Ok(Some(value.to_owned()));
        }
    }
    Ok(None)
}

fn parse_command(json: &str) -> Result<Vec<String>, ConfigError> {
    let command: Vec<String> = serde_json::from_str(json)
        .map_err(|error| ConfigError::InvalidCmdJson(error.to_string()))?;
    if command.is_empty() {
        return Err(ConfigError::EmptyCommand);
    }
    Ok(command)
}

fn parse_env_json(json: Option<&str>) -> Result<Vec<(String, String)>, ConfigError> {
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    let env: std::collections::BTreeMap<String, String> = serde_json::from_str(json)
        .map_err(|error| ConfigError::InvalidEnvJson(error.to_string()))?;
    Ok(env.into_iter().collect())
}

/// Reads and parses `config.toml`. A missing file is `Ok(None)`; a file that
/// cannot be read or parsed is an error.
fn read_config_file(path: &Path) -> Result<Option<ConfigFile>, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ConfigError::ConfigFileIo {
                path: path.to_owned(),
                source: error,
            })
        }
    };
    let file: ConfigFile = toml::from_str(&text).map_err(|source| ConfigError::ConfigFile {
        path: path.to_owned(),
        source,
    })?;
    Ok(Some(file))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    url: Option<String>,
    command: Option<Vec<String>>,
    env: Option<std::collections::BTreeMap<String, String>>,
    spawn_if_unreachable: Option<bool>,
    hotkey: Option<String>,
}

/// Builds the sidecar config from a parsed file, or `None` when the file
/// carries neither a `url` nor a `command`.
fn config_from_file(file: ConfigFile) -> Result<Option<SidecarConfig>, ConfigError> {
    let url = file
        .url
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty());
    if let Some(url) = &url {
        if !is_attach_url(url) {
            return Err(ConfigError::InvalidUrl(redact(url)));
        }
    }
    let command = match &file.command {
        Some(command) if command.is_empty() => return Err(ConfigError::EmptyCommand),
        Some(command) => command.clone(),
        None => Vec::new(),
    };
    let env = match &file.env {
        Some(env) => env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        None => Vec::new(),
    };
    if url.is_none() && file.command.is_none() {
        return Ok(None);
    }
    Ok(Some(SidecarConfig {
        attach_url: url,
        command,
        env,
        spawn_if_unreachable: file.spawn_if_unreachable.unwrap_or(true),
        ..SidecarConfig::default()
    }))
}

/// The raw `hotkey` entry of the config file for [`crate::hotkey`]. A missing
/// or unparsable file yields `None`; parsing problems are surfaced by
/// `resolve`, not duplicated here.
pub(crate) fn hotkey_entry(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let file: ConfigFile = toml::from_str(&text).ok()?;
    file.hotkey
}

/// Looks up a `hermes` executable on PATH: each entry must be absolute and
/// name a regular file with an exec bit (Unix).
fn find_on_path(path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let path = path?;
    for dir in std::env::split_paths(path) {
        // A relative entry (`.`, `bin`, `..`) is resolved against whatever
        // directory the shell happens to run in, so it could pick up an
        // attacker-planted `hermes`; only absolute entries are trusted. This
        // also covers the empty entry, which is not absolute.
        if !dir.is_absolute() {
            continue;
        }
        for name in EXECUTABLE_NAMES {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

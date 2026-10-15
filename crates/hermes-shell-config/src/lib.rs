//! Pure configuration logic for the Tauri shell: where the Hermes backend
//! comes from, deep-link routes, hotkey and updater settings.
//!
//! No Tauri here: everything is plain values in, typed results out, so it is
//! testable on the host. The shell (`src-tauri`) only glues the results in.

mod config;
mod deeplink;
mod hotkey;
mod updater;

pub use config::{resolve, resolve_with_bundled, Bundled, ConfigError, Inputs, Resolved, Source};
pub use deeplink::{from_args, parse, DeepLinkError, Route};
pub use hotkey::{hotkey, DEFAULT_HOTKEY};
pub use updater::{settings, UpdaterSettings, CHECK_INTERVAL, FIRST_CHECK_DELAY};

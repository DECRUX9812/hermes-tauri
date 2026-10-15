use std::path::Path;

use crate::config::hotkey_entry;

/// The default global hotkey accelerator (Cmd on macOS, Ctrl elsewhere).
pub const DEFAULT_HOTKEY: &str = "CommandOrControl+Shift+Space";

/// The global hotkey to register, or `None` when the hotkey is disabled.
///
/// `file` is the path to config.toml. A missing file, a missing `hotkey` key,
/// or a file that does not parse selects the default; `hotkey = ""` (or
/// whitespace only) disables the hotkey. The accelerator syntax itself is
/// validated by the global-shortcut plugin when it is registered, not here.
pub fn hotkey(file: Option<&str>) -> Option<String> {
    let Some(file) = file else {
        return Some(DEFAULT_HOTKEY.to_owned());
    };
    match hotkey_entry(Path::new(file)).as_deref().map(str::trim) {
        None => Some(DEFAULT_HOTKEY.to_owned()),
        Some("") => None,
        Some(value) => Some(value.to_owned()),
    }
}

//! The global-hotkey setting, read from real config files in temp dirs.

use hermes_shell_config::{hotkey, DEFAULT_HOTKEY};

fn write_config(body: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("config.toml"), body).expect("write config.toml");
    let path = dir
        .path()
        .join("config.toml")
        .to_string_lossy()
        .into_owned();
    (dir, path)
}

#[test]
fn no_config_file_selects_the_default() {
    assert_eq!(hotkey(None).as_deref(), Some(DEFAULT_HOTKEY));
    assert_eq!(
        hotkey(Some("/nonexistent-hermes-path/config.toml")).as_deref(),
        Some(DEFAULT_HOTKEY)
    );
}

#[test]
fn file_without_a_hotkey_key_selects_the_default() {
    let (_dir, path) = write_config("command = [\"/bin/echo\"]\n");
    assert_eq!(hotkey(Some(&path)).as_deref(), Some(DEFAULT_HOTKEY));
}

#[test]
fn empty_hotkey_disables_the_hotkey() {
    let (_dir, path) = write_config("hotkey = \"\"\n");
    assert_eq!(hotkey(Some(&path)), None);

    let (_dir, path) = write_config("hotkey = \"   \"\n");
    assert_eq!(hotkey(Some(&path)), None);

    let (_dir, path) = write_config("hotkey = \"\\t\"\n");
    assert_eq!(hotkey(Some(&path)), None);
}

#[test]
fn keeps_a_configured_accelerator_trimmed() {
    let (_dir, path) = write_config("hotkey = \"Ctrl+Shift+P\"\n");
    assert_eq!(hotkey(Some(&path)).as_deref(), Some("Ctrl+Shift+P"));

    let (_dir, path) = write_config("hotkey = \"  F1  \"\n");
    assert_eq!(hotkey(Some(&path)).as_deref(), Some("F1"));
}

#[test]
fn unparsable_file_falls_back_to_the_default() {
    let (_dir, path) = write_config("command = [\"/bin/echo\"\n");
    assert_eq!(hotkey(Some(&path)).as_deref(), Some(DEFAULT_HOTKEY));
}

//! The bundled Python payload as it ships in the app's resource directory.
//!
//! A payload build puts one archive plus a manifest next to the app binary
//! (`src-tauri/tauri.payload.conf.json`, written by `scripts/pack-payload.sh`).
//! The archive cannot be run from the resource dir — a `deb` installs it
//! read-only and an AppImage mounts it at a new path every launch — so the
//! shell unpacks it into the app-data directory on first run.
//!
//! Only the lookup and the call into `hermes-shell-config` live here: the
//! installer itself (`ensure_unpacked`) owns the archive format, the manifest
//! hash check and the safe extraction.
//!
//! Unix only, like the payload's POSIX launcher (docs/python-sidecar.md §5.4).

use std::path::{Path, PathBuf};

use hermes_shell_config::{ensure_unpacked, Bundled};

/// The archive and manifest names `src-tauri/tauri.payload.conf.json` maps
/// into the resource dir.
const ARCHIVE: &str = "hermes-payload.tar.zst";
const MANIFEST: &str = "payload-manifest.json";

/// A payload this build ships, and where its unpacked copy belongs.
#[derive(Clone, Debug)]
pub struct PayloadSource {
    /// `<resource_dir>/hermes-payload.tar.zst`.
    pub archive: PathBuf,
    /// `<resource_dir>/payload-manifest.json`.
    pub manifest: PathBuf,
    /// The per-user app-data directory: `<data_dir>/payload/<version>/` is the
    /// unpacked payload, and `HERMES_HOME`/`HERMES_RUNTIME_DIR` sit beside it.
    pub data_dir: PathBuf,
}

/// The payload this build ships, or `None` when it ships none.
///
/// Every build without the payload overlay has neither file, which is the
/// normal case for a development build, so the shell silently falls back to
/// the environment, `config.toml` and PATH.
pub fn locate(resource_dir: &Path, data_dir: &Path) -> Option<PayloadSource> {
    let archive = resource_dir.join(ARCHIVE);
    let manifest = resource_dir.join(MANIFEST);
    (archive.is_file() && manifest.is_file()).then(|| PayloadSource {
        archive,
        manifest,
        data_dir: data_dir.to_owned(),
    })
}

/// Installs the payload into `<data_dir>/payload/<version>/` and returns the
/// `Bundled` the resolver takes.
///
/// Cheap on every run after the first: the installer returns as soon as the
/// version directory's `.ready` marker names the manifest's sha256. The error
/// is the installer's typed one (a failed hash check, a corrupt archive, a
/// refused member, an I/O error) and reaches the boot page unchanged.
pub fn unpack(source: &PayloadSource) -> Result<Bundled, String> {
    ensure_unpacked(&source.archive, &source.manifest, &source.data_dir)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{locate, ARCHIVE, MANIFEST};

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dir");
        }
        fs::write(path, b"x").expect("write file");
    }

    #[test]
    fn a_build_without_the_payload_ships_none() {
        let root = std::env::temp_dir().join(format!(
            "hermes-tauri-payload-test-{}-empty",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create root");

        assert!(locate(&root, &root.join("data")).is_none());

        // Half a payload is not a payload: both files have to be there.
        touch(&root.join(ARCHIVE));
        assert!(locate(&root, &root.join("data")).is_none());
        touch(&root.join(MANIFEST));
        assert!(locate(&root, &root.join("data")).is_some());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_located_paths_are_the_ones_the_overlay_ships() {
        let root = std::env::temp_dir().join(format!(
            "hermes-tauri-payload-test-{}-paths",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let (resources, data) = (root.join("resources"), root.join("data"));
        touch(&resources.join(ARCHIVE));
        touch(&resources.join(MANIFEST));

        let source = locate(&resources, &data).expect("payload");
        assert_eq!(source.archive, resources.join(ARCHIVE));
        assert_eq!(source.manifest, resources.join(MANIFEST));
        assert_eq!(source.data_dir, data);

        let _ = fs::remove_dir_all(&root);
    }
}

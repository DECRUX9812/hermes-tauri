//! First-run install of the bundled payload archive into the app-data dir.
//!
//! `scripts/pack-payload.sh` writes one `hermes-payload.tar.zst` (plus a JSON
//! manifest) into the app's resource dir. That dir is read-only (a deb install,
//! a squashfs AppImage mount), and Tauri's resource copy cannot even reproduce
//! the payload because it dereferences symlinks (T16a run A: the copied venv
//! dies with "Failed to import encodings module"). So the shell ships the
//! archive and unpacks it once, with symlinks intact, into
//! `<data_dir>/payload/<version>/`; see docs/python-sidecar.md
//! "Archive + first-run unpack".
//!
//! Layout under `data_dir` (the two trees stay disjoint):
//!
//! ```text
//! <data_dir>/payload/.lock             exclusive install lock
//! <data_dir>/payload/<version>/        the unpacked payload
//! <data_dir>/payload/<version>/.ready  the archive sha256 it was unpacked from
//! <data_dir>/hermes                    HERMES_HOME         (T15 `bundled_env`)
//! <data_dir>/runtime                   HERMES_RUNTIME_DIR  (T15 `bundled_env`)
//! ```
//!
//! Only the `payload` tree is ever written or pruned here; the user's Hermes
//! home is never touched.
//!
//! Unix only, like the payload's launcher (T15's `bundled_launcher`): a Windows
//! payload needs a Windows launcher first (docs/python-sidecar.md §5.4).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use fs4::FileExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tar::EntryType;

use crate::Bundled;

/// Marker inside a version dir naming the archive it was unpacked from.
const READY_FILE: &str = ".ready";
/// The exclusive lock, so two shells cannot unpack at once.
const LOCK_FILE: &str = ".lock";
/// Staging dirs and the ready-file temp are named with this infix.
const TMP_INFIX: &str = ".tmp-";

/// `payload-manifest.json`, written next to the archive by
/// `scripts/pack-payload.sh`. Only `version` and `sha256` are needed to install
/// the payload; the three sizes document what was packed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// The payload's own identity, from `hermes-payload/MANIFEST` (`built`).
    pub version: String,
    /// Lowercase hex sha256 of the archive.
    pub sha256: String,
    /// Size of the archive in bytes (informational).
    #[serde(default)]
    pub archive_bytes: u64,
    /// Sum of the regular-file sizes in the archive (informational).
    #[serde(default)]
    pub unpacked_bytes: u64,
    /// Number of regular files in the archive (informational).
    #[serde(default)]
    pub files: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("Could not read the payload manifest {}: {source}", .path.display())]
    ManifestIo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Could not parse the payload manifest {}: {source}", .path.display())]
    ManifestParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("The payload manifest {} has no `{field}`", .path.display())]
    ManifestField { path: PathBuf, field: &'static str },
    #[error("The payload version {0:?} is not a plain directory name")]
    InvalidVersion(String),
    #[error("Could not read the payload archive {}: {source}", .path.display())]
    ArchiveIo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("The payload archive {} is not readable as a tar.zst stream: {source}", .path.display())]
    ArchiveCorrupt {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "The payload archive does not match its manifest: expected sha256 {expected}, got {actual}"
    )]
    HashMismatch { expected: String, actual: String },
    #[error("The payload archive contains an absolute path: {}", .0.display())]
    AbsolutePath(PathBuf),
    #[error("The payload archive contains a path with a `..` component: {}", .0.display())]
    ParentComponent(PathBuf),
    #[error(
        "The payload archive contains the symlink {} whose target {} leaves the payload",
        .path.display(),
        .target.display()
    )]
    EscapingSymlink { path: PathBuf, target: PathBuf },
    #[error("The payload archive contains a hard link: {}", .0.display())]
    HardLink(PathBuf),
    #[error("The payload archive contains an unsupported entry {} of type {kind}", .path.display())]
    UnsupportedEntry { path: PathBuf, kind: String },
    #[error("Could not write the payload under {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Could not take the payload lock {}: {source}", .path.display())]
    Lock {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Makes sure `archive` is unpacked under `data_dir` and returns the
/// [`Bundled`] the resolver takes.
///
/// Returns as soon as `<data_dir>/payload/<version>/.ready` names the manifest's
/// sha256. Otherwise it verifies the archive hash first — so an archive that
/// does not match writes nothing at all — then takes the exclusive lock,
/// unpacks into a staging dir next to the version dir, renames that into place,
/// marks it ready, and only then prunes stale staging dirs and older versions.
pub fn ensure_unpacked(
    archive: &Path,
    manifest_path: &Path,
    data_dir: &Path,
) -> Result<Bundled, InstallError> {
    let manifest = read_manifest(manifest_path)?;
    let version = safe_version(&manifest.version)?;
    let root = data_dir.join("payload");
    let target = root.join(&version);

    if is_ready(&target, &manifest.sha256) {
        return Ok(bundled(target, data_dir));
    }

    // Hashing before anything is created means a corrupt or wrong archive
    // leaves no trace at all.
    let actual = sha256_file(archive)?;
    if actual != manifest.sha256 {
        return Err(InstallError::HashMismatch {
            expected: manifest.sha256,
            actual,
        });
    }

    fs::create_dir_all(&root).at(&root)?;
    let lock_path = root.join(LOCK_FILE);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|source| InstallError::Lock {
            path: lock_path.clone(),
            source,
        })?;
    // `FileExt` explicitly: the std `File::lock` shorthand needs Rust 1.89,
    // this crate's MSRV is 1.80.
    FileExt::lock(&lock).map_err(|source| InstallError::Lock {
        path: lock_path.clone(),
        source,
    })?;

    // Another process may have finished while this one waited for the lock.
    if is_ready(&target, &manifest.sha256) {
        let _ = FileExt::unlock(&lock);
        return Ok(bundled(target, data_dir));
    }

    let staging = root.join(format!("{version}{TMP_INFIX}{}", std::process::id()));
    remove_any(&staging);
    if let Err(error) = extract(archive, &staging) {
        remove_any(&staging);
        let _ = FileExt::unlock(&lock);
        return Err(error);
    }

    // An interrupted install of the same version, or a stale tree from an
    // archive rebuilt under the same version, has to go before the rename.
    remove_any(&target);
    fs::rename(&staging, &target).at(&target)?;
    write_ready(&target, &manifest.sha256)?;
    prune_old(&root, &version);

    let _ = FileExt::unlock(&lock);
    Ok(bundled(target, data_dir))
}

/// Reads and parses `payload-manifest.json`.
pub fn read_manifest(path: &Path) -> Result<Manifest, InstallError> {
    let text = fs::read_to_string(path).map_err(|source| InstallError::ManifestIo {
        path: path.to_owned(),
        source,
    })?;
    let manifest: Manifest =
        serde_json::from_str(&text).map_err(|source| InstallError::ManifestParse {
            path: path.to_owned(),
            source,
        })?;
    if manifest.version.trim().is_empty() {
        return Err(InstallError::ManifestField {
            path: path.to_owned(),
            field: "version",
        });
    }
    if manifest.sha256.trim().is_empty() {
        return Err(InstallError::ManifestField {
            path: path.to_owned(),
            field: "sha256",
        });
    }
    Ok(manifest)
}

fn bundled(payload_dir: PathBuf, data_dir: &Path) -> Bundled {
    Bundled {
        payload_dir,
        data_dir: data_dir.to_owned(),
    }
}

/// A version is used as one directory name, so it may not be empty, may not be
/// `.`/`..`, and may not carry a separator or any other exotic byte.
fn safe_version(version: &str) -> Result<String, InstallError> {
    let version = version.trim();
    let plain = !version.is_empty()
        && version != "."
        && version != ".."
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if plain {
        Ok(version.to_owned())
    } else {
        Err(InstallError::InvalidVersion(version.to_owned()))
    }
}

/// True when the version dir carries a `.ready` naming exactly this archive.
fn is_ready(target: &Path, sha256: &str) -> bool {
    fs::read_to_string(target.join(READY_FILE))
        .map(|text| text.trim() == sha256)
        .unwrap_or(false)
}

fn write_ready(target: &Path, sha256: &str) -> Result<(), InstallError> {
    let temp = target.join(format!("{READY_FILE}{TMP_INFIX}{}", std::process::id()));
    fs::write(&temp, sha256).at(&temp)?;
    let ready = target.join(READY_FILE);
    fs::rename(&temp, &ready).at(&ready)
}

fn sha256_file(path: &Path) -> Result<String, InstallError> {
    let mut file = File::open(path).map_err(|source| InstallError::ArchiveIo {
        path: path.to_owned(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| InstallError::ArchiveIo {
                path: path.to_owned(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Unpacks `archive` into `dest`, which must not exist yet.
///
/// Every entry is checked before it is written: absolute paths, `..`
/// components, hard links and symlinks that leave `dest` are refused, and
/// nothing else (devices, fifos, sparse GNU entries) is accepted either. The
/// archive's own mode bits are applied, so the launcher and the interpreter
/// keep their exec bits.
fn extract(archive: &Path, dest: &Path) -> Result<(), InstallError> {
    let file = File::open(archive).map_err(|source| InstallError::ArchiveIo {
        path: archive.to_owned(),
        source,
    })?;
    let decoder =
        zstd::stream::read::Decoder::new(file).map_err(|source| InstallError::ArchiveCorrupt {
            path: archive.to_owned(),
            source,
        })?;
    let mut reader = tar::Archive::new(decoder);

    fs::create_dir_all(dest).at(dest)?;
    let mut dir_modes: Vec<(PathBuf, u32)> = Vec::new();
    let corrupt = |source| InstallError::ArchiveCorrupt {
        path: archive.to_owned(),
        source,
    };

    for entry in reader.entries().map_err(corrupt)? {
        let mut entry = entry.map_err(corrupt)?;
        let member = entry.path().map_err(corrupt)?.into_owned();
        let out = member_path(dest, &member)?;
        let mode = entry.header().mode().map_err(corrupt)? & 0o7777;

        match entry.header().entry_type() {
            EntryType::Directory => {
                fs::create_dir_all(&out).at(&out)?;
                dir_modes.push((out, mode));
            }
            EntryType::Regular | EntryType::Continuous => {
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent).at(parent)?;
                }
                let mut file = File::create(&out).at(&out)?;
                let mut buffer = [0u8; 64 * 1024];
                loop {
                    let read = entry.read(&mut buffer).map_err(corrupt)?;
                    if read == 0 {
                        break;
                    }
                    file.write_all(&buffer[..read]).at(&out)?;
                }
                file.flush().at(&out)?;
                drop(file);
                set_mode(&out, mode)?;
            }
            EntryType::Symlink => {
                let target = entry
                    .link_name()
                    .map_err(corrupt)?
                    .map(|target| target.into_owned())
                    .ok_or_else(|| InstallError::EscapingSymlink {
                        path: member.clone(),
                        target: PathBuf::new(),
                    })?;
                if !symlink_stays_inside(&member, &target) {
                    return Err(InstallError::EscapingSymlink {
                        path: member,
                        target,
                    });
                }
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent).at(parent)?;
                }
                std::os::unix::fs::symlink(&target, &out).at(&out)?;
            }
            EntryType::Link => return Err(InstallError::HardLink(member)),
            other => {
                return Err(InstallError::UnsupportedEntry {
                    path: member,
                    kind: format!("{other:?}"),
                })
            }
        }
    }

    // Deepest last in the archive, first here: a read-only parent must not
    // block writing its children.
    for (dir, mode) in dir_modes.into_iter().rev() {
        set_mode(&dir, mode)?;
    }
    Ok(())
}

/// `dest` joined with an archive member path, refusing anything that is not a
/// plain relative path.
fn member_path(dest: &Path, member: &Path) -> Result<PathBuf, InstallError> {
    if member.is_absolute() {
        return Err(InstallError::AbsolutePath(member.to_owned()));
    }
    let mut out = dest.to_owned();
    for component in member.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(InstallError::ParentComponent(member.to_owned())),
            Component::RootDir | Component::Prefix(_) => {
                return Err(InstallError::AbsolutePath(member.to_owned()))
            }
        }
    }
    Ok(out)
}

/// True when a relative symlink `target`, read from the member `link`, cannot
/// walk above the payload root.
fn symlink_stays_inside(link: &Path, target: &Path) -> bool {
    if target.as_os_str().is_empty() || target.is_absolute() {
        return false;
    }
    let mut depth = link
        .parent()
        .map(|parent| {
            parent
                .components()
                .filter(|component| matches!(component, Component::Normal(_)))
                .count()
        })
        .unwrap_or(0);
    for component in target.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

fn set_mode(path: &Path, mode: u32) -> Result<(), InstallError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).at(path)
}

/// Removes a leftover staging dir, an older version, or an incomplete install
/// of the same version. Best effort: a leftover that cannot be removed does not
/// fail an install that is otherwise done.
fn prune_old(root: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_str() == Some(LOCK_FILE) || name.to_str() == Some(keep) {
            continue;
        }
        remove_any(&entry.path());
    }
}

fn remove_any(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            let _ = fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// `io::Result` with the path the operation was about, so every filesystem
/// error surfaces with the path it failed on.
trait AtPath<T> {
    fn at(self, path: &Path) -> Result<T, InstallError>;
}

impl<T> AtPath<T> for io::Result<T> {
    fn at(self, path: &Path) -> Result<T, InstallError> {
        self.map_err(|source| InstallError::Io {
            path: path.to_owned(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    use tar::{Builder, Header};
    use tempfile::TempDir;

    use super::*;
    use crate::{resolve_with_bundled, Inputs, Resolved, Source};

    /// A real tar.zst archive, built in memory and written to disk: every test
    /// below exercises the same reader the shipped payload goes through.
    fn write_archive(path: &Path, build: impl FnOnce(&mut Builder<Vec<u8>>)) {
        let mut builder = Builder::new(Vec::new());
        build(&mut builder);
        let tar_bytes = builder.into_inner().expect("finish the tar stream");
        let file = File::create(path).expect("create the archive");
        let mut encoder = zstd::Encoder::new(file, 3).expect("zstd encoder");
        encoder
            .write_all(&tar_bytes)
            .expect("compress the tar stream");
        encoder.finish().expect("finish the zstd stream");
    }

    fn file<W: Write>(builder: &mut Builder<W>, name: &str, data: &[u8], mode: u32) {
        let mut header = Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_mtime(0);
        header.set_entry_type(EntryType::Regular);
        builder
            .append_data(&mut header, name, data)
            .expect("append a file");
    }

    fn dir<W: Write>(builder: &mut Builder<W>, name: &str, mode: u32) {
        let mut header = Header::new_gnu();
        header.set_size(0);
        header.set_mode(mode);
        header.set_mtime(0);
        header.set_entry_type(EntryType::Directory);
        builder
            .append_data(&mut header, name, io::empty())
            .expect("append a directory");
    }

    fn symlink<W: Write>(builder: &mut Builder<W>, name: &str, target: &str) {
        let mut header = Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_mtime(0);
        header.set_entry_type(EntryType::Symlink);
        builder
            .append_link(&mut header, name, target)
            .expect("append a symlink");
    }

    /// Appends an entry whose name and link target are written raw into the
    /// header, so a hostile member reaches the archive exactly as written.
    fn raw_entry<W: Write>(
        builder: &mut Builder<W>,
        name: &[u8],
        kind: EntryType,
        link: Option<&[u8]>,
        data: &[u8],
    ) {
        assert!(name.len() <= 100, "the test pokes the 100-byte tar name");
        let mut header = Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_entry_type(kind);
        header.as_old_mut().name = [0; 100];
        header.as_old_mut().name[..name.len()].copy_from_slice(name);
        if let Some(link) = link {
            header
                .set_link_name_literal(link)
                .expect("set the link name");
        }
        header.set_cksum();
        builder.append(&header, data).expect("append a raw entry");
    }

    /// A member name past the 100-byte header field, i.e. a GNU long name.
    fn long_name() -> String {
        format!(
            "hermes-payload/venv/lib/python3.14/site-packages/{}/long_module.py",
            "a".repeat(60)
        )
    }

    struct Packed {
        archive: PathBuf,
        manifest: PathBuf,
        sha256: String,
        version: String,
    }

    /// Returns the packed pair and the temp dir that holds them, so the caller
    /// keeps the archive and the manifest alive for the whole test.
    fn pack(version: &str, build: impl FnOnce(&mut Builder<Vec<u8>>)) -> (Packed, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir for the archive");
        let archive = dir.path().join("hermes-payload.tar.zst");
        write_archive(&archive, build);
        let sha256 = sha256_file(&archive).expect("hash the archive");
        let bytes = fs::metadata(&archive).expect("archive metadata").len();
        let manifest = dir.path().join("payload-manifest.json");
        let json = serde_json::json!({
            "version": version,
            "sha256": sha256,
            "archive_bytes": bytes,
            "unpacked_bytes": 0,
            "files": 0,
        });
        fs::write(
            &manifest,
            serde_json::to_vec_pretty(&json).expect("manifest json"),
        )
        .expect("write the manifest");
        (
            Packed {
                archive,
                manifest,
                sha256,
                version: version.to_owned(),
            },
            dir,
        )
    }

    /// The payload's shape in miniature: the launcher, the venv interpreter
    /// symlink and the interpreter alias are the parts T16a showed a resource
    /// copy destroys.
    fn tiny_payload(version: &str) -> (Packed, TempDir) {
        pack(version, |builder| {
            dir(builder, "hermes-payload", 0o755);
            dir(builder, "hermes-payload/bin", 0o755);
            file(
                builder,
                "hermes-payload/bin/hermes-sidecar",
                b"#!/bin/sh\nexit 0\n",
                0o755,
            );
            file(
                builder,
                "hermes-payload/MANIFEST",
                b"built=2026-01-02T03:04:05Z\n",
                0o644,
            );
            dir(builder, "hermes-payload/venv", 0o755);
            dir(builder, "hermes-payload/venv/lib", 0o755);
            file(
                builder,
                "hermes-payload/venv/lib/site.py",
                b"X = 1\n",
                0o644,
            );
            symlink(builder, "hermes-payload/venv/lib64", "lib");
            dir(builder, "python", 0o755);
            dir(builder, "python/cpython-3.14.8-linux-x86_64-gnu", 0o755);
            file(
                builder,
                "python/cpython-3.14.8-linux-x86_64-gnu/bin/python3.14",
                b"binary\n",
                0o755,
            );
            symlink(
                builder,
                "python/cpython-3.14-linux-x86_64-gnu",
                "cpython-3.14.8-linux-x86_64-gnu",
            );
            file(builder, &long_name(), b"LONG = 1\n", 0o644);
        })
    }

    fn staging_dirs(data_dir: &Path) -> Vec<PathBuf> {
        let root = data_dir.join("payload");
        let mut found = Vec::new();
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().contains(TMP_INFIX) {
                    found.push(entry.path());
                }
            }
        }
        found
    }

    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    /// Nothing of the payload exists: neither a version dir nor a staging dir.
    fn assert_nothing_installed(data_dir: &Path, version: &str) {
        let installed = data_dir.join("payload").join(version);
        assert!(
            !installed.exists(),
            "{} must not exist",
            installed.display()
        );
        let staging = staging_dirs(data_dir);
        assert!(staging.is_empty(), "staging dirs left: {staging:?}");
    }

    #[test]
    fn an_archive_with_a_symlink_and_an_exec_file_round_trips() {
        let (packed, _scratch) = tiny_payload("2026-01-02T03-04-05Z");
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let bundled =
            ensure_unpacked(&packed.archive, &packed.manifest, data.path()).expect("unpack");

        let root = data.path().join("payload").join(&packed.version);
        assert_eq!(bundled.payload_dir, root);
        assert_eq!(bundled.data_dir, data.path());
        assert_eq!(
            fs::read_to_string(root.join(READY_FILE)).unwrap(),
            packed.sha256
        );

        let launcher = root.join("hermes-payload/bin/hermes-sidecar");
        assert_eq!(
            fs::read(&launcher).unwrap(),
            b"#!/bin/sh\nexit 0\n".to_vec()
        );
        assert!(is_executable(&launcher), "the launcher keeps its exec bit");
        assert!(is_executable(
            &root.join("python/cpython-3.14.8-linux-x86_64-gnu/bin/python3.14")
        ));

        let link = root.join("hermes-payload/venv/lib64");
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("lib"));
        assert!(root.join("hermes-payload/venv/lib/site.py").is_file());

        let alias = root.join("python/cpython-3.14-linux-x86_64-gnu");
        assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
        assert_eq!(
            fs::read_link(&alias).unwrap(),
            Path::new("cpython-3.14.8-linux-x86_64-gnu")
        );

        assert!(root.join(long_name()).is_file(), "a GNU long name survives");
        assert!(staging_dirs(data.path()).is_empty());
    }

    #[test]
    fn a_ready_marker_short_circuits_even_without_the_archive() {
        let (packed, _scratch) = tiny_payload("v1");
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let first =
            ensure_unpacked(&packed.archive, &packed.manifest, data.path()).expect("unpack");

        fs::remove_file(&packed.archive).expect("the archive is gone");
        let second = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect("the ready marker answers without the archive");

        assert_eq!(second.payload_dir, first.payload_dir);
        assert_eq!(staging_dirs(data.path()).len(), 0);
    }

    #[test]
    fn a_wrong_sha256_is_rejected_before_anything_is_written() {
        let (packed, _scratch) = tiny_payload("v1");
        let json = serde_json::json!({
            "version": packed.version,
            "sha256": "0".repeat(64),
        });
        fs::write(&packed.manifest, serde_json::to_vec(&json).unwrap()).unwrap();

        let data = tempfile::tempdir().expect("tempdir for the app data");
        let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect_err("a wrong digest must be refused");

        assert!(
            matches!(error, InstallError::HashMismatch { .. }),
            "{error}"
        );
        assert_eq!(
            fs::read_dir(data.path()).unwrap().count(),
            0,
            "nothing at all is written for a mismatching archive"
        );
    }

    #[test]
    fn a_member_with_an_absolute_path_is_refused() {
        let (packed, _scratch) = pack("v1", |builder| {
            raw_entry(builder, b"/etc/passwd", EntryType::Regular, None, b"x")
        });
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect_err("an absolute member is refused");

        assert!(matches!(error, InstallError::AbsolutePath(_)), "{error}");
        assert_nothing_installed(data.path(), "v1");
    }

    #[test]
    fn a_member_with_a_parent_component_is_refused() {
        let (packed, _scratch) = pack("v1", |builder| {
            raw_entry(builder, b"../escape", EntryType::Regular, None, b"x")
        });
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect_err("a `..` member is refused");

        assert!(matches!(error, InstallError::ParentComponent(_)), "{error}");
        assert_nothing_installed(data.path(), "v1");
    }

    #[test]
    fn a_symlink_that_leaves_the_payload_is_refused() {
        // The link sits three levels deep, so it may walk up three levels and
        // no further.
        for target in [
            &b"../../../../outside"[..],
            &b"../../../.."[..],
            &b"../../../../etc/passwd"[..],
            &b"/etc/passwd"[..],
            &b""[..],
        ] {
            let (packed, _scratch) = pack("v1", |builder| {
                raw_entry(
                    builder,
                    b"hermes-payload/venv/bin/python",
                    EntryType::Symlink,
                    Some(target),
                    b"",
                )
            });
            let data = tempfile::tempdir().expect("tempdir for the app data");
            let error =
                ensure_unpacked(&packed.archive, &packed.manifest, data.path()).unwrap_err();
            assert!(
                matches!(error, InstallError::EscapingSymlink { .. }),
                "target {target:?} gave {error}"
            );
            assert_nothing_installed(data.path(), "v1");
        }
    }

    #[test]
    fn a_symlink_that_stays_inside_is_accepted() {
        let (packed, _scratch) = pack("v1", |builder| {
            dir(builder, "hermes-payload", 0o755);
            dir(builder, "hermes-payload/venv", 0o755);
            dir(builder, "hermes-payload/venv/bin", 0o755);
            dir(builder, "python", 0o755);
            file(builder, "python/python3.14", b"binary\n", 0o755);
            // Three levels up from the link's directory is the payload root.
            raw_entry(
                builder,
                b"hermes-payload/venv/bin/python",
                EntryType::Symlink,
                Some(b"../../../python/python3.14"),
                b"",
            );
            // ... and exactly the root is still inside it.
            raw_entry(
                builder,
                b"hermes-payload/venv/bin/root",
                EntryType::Symlink,
                Some(b"../../.."),
                b"",
            );
        });
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let bundled = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect("a link that stays inside is fine");
        assert_eq!(
            fs::read_link(bundled.payload_dir.join("hermes-payload/venv/bin/python")).unwrap(),
            Path::new("../../../python/python3.14")
        );
        assert_eq!(
            fs::read_link(bundled.payload_dir.join("hermes-payload/venv/bin/root")).unwrap(),
            Path::new("../../..")
        );
    }

    #[test]
    fn a_hard_link_is_refused() {
        let (packed, _scratch) = pack("v1", |builder| {
            file(builder, "hermes-payload/MANIFEST", b"built=x\n", 0o644);
            raw_entry(
                builder,
                b"hermes-payload/alias",
                EntryType::Link,
                Some(b"hermes-payload/MANIFEST"),
                b"",
            )
        });
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect_err("a hard link is refused");

        assert!(matches!(error, InstallError::HardLink(_)), "{error}");
        assert_nothing_installed(data.path(), "v1");
    }

    #[test]
    fn an_entry_that_is_not_a_file_directory_or_symlink_is_refused() {
        let (packed, _scratch) = pack("v1", |builder| {
            raw_entry(builder, b"hermes-payload/fifo", EntryType::Fifo, None, b"")
        });
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect_err("a fifo is refused");

        assert!(
            matches!(error, InstallError::UnsupportedEntry { .. }),
            "{error}"
        );
        assert_nothing_installed(data.path(), "v1");
    }

    #[test]
    fn a_stale_staging_dir_and_older_versions_are_pruned_but_the_hermes_home_is_kept() {
        let (packed, _scratch) = tiny_payload("v2");
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let payload_root = data.path().join("payload");

        fs::create_dir_all(payload_root.join("v1/hermes-payload")).unwrap();
        fs::write(payload_root.join("v1/old.txt"), b"old").unwrap();
        fs::create_dir_all(payload_root.join("v2.tmp-999999/junk")).unwrap();
        fs::create_dir_all(data.path().join("hermes")).unwrap();
        fs::write(data.path().join("hermes/sessions.json"), b"{}").unwrap();
        fs::create_dir_all(data.path().join("runtime")).unwrap();
        fs::write(data.path().join("runtime/sidecar.json"), b"{}").unwrap();

        ensure_unpacked(&packed.archive, &packed.manifest, data.path()).expect("unpack");

        assert!(
            !payload_root.join("v1").exists(),
            "the older version is pruned"
        );
        assert!(
            staging_dirs(data.path()).is_empty(),
            "stale staging is pruned"
        );
        assert!(payload_root.join("v2").join(READY_FILE).is_file());
        assert!(payload_root.join(LOCK_FILE).exists(), "the lock file stays");
        assert!(
            data.path().join("hermes/sessions.json").is_file(),
            "HERMES_HOME is never touched"
        );
        assert!(
            data.path().join("runtime/sidecar.json").is_file(),
            "HERMES_RUNTIME_DIR is never touched"
        );
    }

    #[test]
    fn an_interrupted_install_of_the_same_version_is_replaced() {
        let (packed, _scratch) = tiny_payload("v6");
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let target = data.path().join("payload/v6");
        fs::create_dir_all(target.join("leftover")).unwrap();
        fs::write(target.join("leftover/garbage"), b"junk").unwrap();

        let bundled = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect("an incomplete install is replaced");

        assert_eq!(bundled.payload_dir, target);
        assert!(!target.join("leftover").exists());
        assert!(target.join("hermes-payload/bin/hermes-sidecar").is_file());
        assert_eq!(
            fs::read_to_string(target.join(READY_FILE)).unwrap(),
            packed.sha256
        );
    }

    #[test]
    fn a_second_caller_blocks_on_the_lock_until_it_is_released() {
        let (packed, _scratch) = tiny_payload("v5");
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let root = data.path().join("payload");
        fs::create_dir_all(&root).unwrap();

        // A different open file description, so flock conflicts even inside
        // this one process: exactly what a second shell does.
        let held = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(LOCK_FILE))
            .expect("open the lock");
        FileExt::lock(&held).expect("hold the lock");

        let archive = packed.archive.clone();
        let manifest = packed.manifest.clone();
        let data_dir = data.path().to_path_buf();
        let caller = std::thread::spawn(move || ensure_unpacked(&archive, &manifest, &data_dir));

        std::thread::sleep(Duration::from_millis(400));
        assert!(
            !caller.is_finished(),
            "a second caller has to wait for the lock"
        );
        assert!(
            !root.join("v5").exists(),
            "nothing is installed while the lock is held"
        );

        FileExt::unlock(&held).expect("release the lock");
        let bundled = caller
            .join()
            .expect("the caller thread")
            .expect("the waiting caller installs once the lock is free");
        assert_eq!(bundled.payload_dir, root.join("v5"));
    }

    #[test]
    fn racing_callers_install_one_payload() {
        let (packed, _scratch) = tiny_payload("v4");
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let callers = 8;
        let barrier = Arc::new(Barrier::new(callers));

        let handles: Vec<_> = (0..callers)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let archive = packed.archive.clone();
                let manifest = packed.manifest.clone();
                let data_dir = data.path().to_path_buf();
                std::thread::spawn(move || {
                    barrier.wait();
                    ensure_unpacked(&archive, &manifest, &data_dir)
                        .map(|bundled| bundled.payload_dir)
                })
            })
            .collect();

        let installed = data.path().join("payload/v4");
        for handle in handles {
            let payload_dir = handle
                .join()
                .expect("the caller thread")
                .expect("every racing caller succeeds");
            assert_eq!(payload_dir, installed);
        }

        assert_eq!(
            fs::read_to_string(installed.join(READY_FILE)).unwrap(),
            packed.sha256
        );
        assert!(installed
            .join("hermes-payload/bin/hermes-sidecar")
            .is_file());
        assert!(
            staging_dirs(data.path()).is_empty(),
            "no staging dir survives"
        );

        let mut names: Vec<String> = fs::read_dir(data.path().join("payload"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec![LOCK_FILE.to_owned(), "v4".to_owned()]);
    }

    #[test]
    fn the_installed_payload_resolves_with_state_outside_it() {
        let (packed, _scratch) = tiny_payload("v3");
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let bundled =
            ensure_unpacked(&packed.archive, &packed.manifest, data.path()).expect("unpack");

        let inputs = Inputs::empty(data.path().join("config"));
        let (config, source) = match resolve_with_bundled(&inputs, Some(&bundled)).expect("resolve")
        {
            Resolved::Configured { config, source } => (config, source),
            Resolved::NotConfigured => panic!("the installed payload did not resolve"),
        };

        assert_eq!(source, Source::Bundled);
        assert!(
            Path::new(&config.command[0]).starts_with(&bundled.payload_dir),
            "the launcher runs from the unpacked payload: {:?}",
            config.command[0]
        );
        let env: BTreeMap<String, String> = config.env.into_iter().collect();
        assert_eq!(
            env["HERMES_HOME"],
            data.path().join("hermes").to_string_lossy().into_owned()
        );
        assert_eq!(
            env["HERMES_RUNTIME_DIR"],
            data.path().join("runtime").to_string_lossy().into_owned()
        );
        for key in ["HERMES_HOME", "HERMES_RUNTIME_DIR"] {
            assert!(
                !Path::new(&env[key]).starts_with(&bundled.payload_dir),
                "{key} must stay outside the payload tree"
            );
        }
    }

    #[test]
    fn an_empty_manifest_field_is_refused() {
        for (field, json) in [
            (
                "version",
                serde_json::json!({ "version": "", "sha256": "ab" }),
            ),
            (
                "sha256",
                serde_json::json!({ "version": "v1", "sha256": "" }),
            ),
        ] {
            let (packed, _scratch) = tiny_payload("v1");
            fs::write(&packed.manifest, serde_json::to_vec(&json).unwrap()).unwrap();
            let data = tempfile::tempdir().expect("tempdir for the app data");
            let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
                .expect_err("an empty manifest field is refused");
            match error {
                InstallError::ManifestField { field: got, .. } => assert_eq!(got, field),
                other => panic!("{field}: expected ManifestField, got {other}"),
            }
            assert_eq!(fs::read_dir(data.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn a_version_that_is_not_a_plain_directory_name_is_refused() {
        for bad in [".", "..", "a/b", "../x", "with space", "v1\u{0}"] {
            let (packed, _scratch) = tiny_payload("v1");
            let json = serde_json::json!({ "version": bad, "sha256": packed.sha256 });
            fs::write(&packed.manifest, serde_json::to_vec(&json).unwrap()).unwrap();
            let data = tempfile::tempdir().expect("tempdir for the app data");
            let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
                .expect_err("a version that is not one path component is refused");
            assert!(
                matches!(error, InstallError::InvalidVersion(_)),
                "{bad:?} gave {error}"
            );
            assert_eq!(fs::read_dir(data.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn a_manifest_that_is_not_json_is_refused() {
        let (packed, _scratch) = tiny_payload("v1");
        fs::write(&packed.manifest, b"{not json").unwrap();
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect_err("a broken manifest is refused");
        assert!(
            matches!(error, InstallError::ManifestParse { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_missing_archive_is_refused() {
        let (packed, _scratch) = tiny_payload("v1");
        fs::remove_file(&packed.archive).unwrap();
        let data = tempfile::tempdir().expect("tempdir for the app data");
        let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect_err("a missing archive is refused");
        assert!(matches!(error, InstallError::ArchiveIo { .. }), "{error}");
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_corrupt_archive_is_refused_and_leaves_nothing() {
        let (packed, _scratch) = tiny_payload("v1");
        // A digest that matches, but bytes that are not a zstd stream at all.
        fs::write(&packed.archive, b"not a zstd frame").unwrap();
        let sha256 = sha256_file(&packed.archive).unwrap();
        let json = serde_json::json!({ "version": "v1", "sha256": sha256 });
        fs::write(&packed.manifest, serde_json::to_vec(&json).unwrap()).unwrap();

        let data = tempfile::tempdir().expect("tempdir for the app data");
        let error = ensure_unpacked(&packed.archive, &packed.manifest, data.path())
            .expect_err("a corrupt stream is refused");
        assert!(
            matches!(error, InstallError::ArchiveCorrupt { .. }),
            "{error}"
        );
        assert_nothing_installed(data.path(), "v1");
    }
}

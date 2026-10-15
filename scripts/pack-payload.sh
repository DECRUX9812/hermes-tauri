#!/usr/bin/env bash
# Pack the bundled Python payload (docs/python-sidecar.md) into ONE archive the
# Tauri bundle can ship as a resource: a deterministic tar stream compressed
# with zstd, plus a small JSON manifest beside it.
#
#   scripts/pack-payload.sh [--payload DIR] [--out DIR] [--level N] [--allow-build-paths]
#
# Why an archive instead of a resource tree: Tauri's resource copy dereferences
# symlinks, and the payload's `venv/bin/python` symlink is what makes the venv
# find its own stdlib (T16a run A: "Fatal Python error: Failed to import
# encodings module"). tar keeps the symlinks, so the unpacked payload runs
# exactly like the cached one. See docs/python-sidecar.md "Archive + first-run
# unpack" and scripts/payload-deref-boot-test.sh for the measurements.
#
# Output (default $PWD/src-tauri/payload, gitignored):
#   hermes-payload.tar.zst   the archive (top-level entries: hermes-payload/ python/)
#   payload-manifest.json    { version, sha256, archive_bytes, unpacked_bytes, files }
#
# The archive is deterministic for a given input tree: names sorted, every mtime
# set to the epoch, numeric owner/group 0, no ACLs/xattrs, and relative member
# names only (so no build path can leak into a name).
#
# zstd comes from python3's `compression.zstd` (CPython 3.14+, PEP 784) rather
# than a `zstd` binary, because the builder image that runs the tests ships no
# zstd CLI; both the host and hermes-tauri-builder:26.04 have Python 3.14.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ARCHIVE_NAME="hermes-payload.tar.zst"
MANIFEST_NAME="payload-manifest.json"

SRC="${HERMES_TAURI_PAYLOAD_DIR:-$HOME/.cache/hermes-tauri/payload}"
OUT="$ROOT/src-tauri/payload"
LEVEL="${HERMES_TAURI_PAYLOAD_ZSTD_LEVEL:-19}"
WORKERS="${HERMES_TAURI_PAYLOAD_ZSTD_WORKERS:-4}"
allow_build_paths=0

log() { printf '[pack-payload] %s\n' "$*" >&2; }
fail() { printf '[pack-payload] error: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
usage: scripts/pack-payload.sh [options]

Packs $HERMES_TAURI_PAYLOAD_DIR/{hermes-payload,python} (default
$HOME/.cache/hermes-tauri/payload) into <out>/hermes-payload.tar.zst plus
<out>/payload-manifest.json. The tar stream preserves the payload's relative
symlinks; names are sorted, mtimes are the epoch and the owner is numeric 0.

  --payload DIR         payload source (default $HERMES_TAURI_PAYLOAD_DIR,
                        else $HOME/.cache/hermes-tauri/payload)
  --out DIR             output directory (default src-tauri/payload)
  --level N             zstd level 1-22 (default 19)
  --allow-build-paths   pack even though *.pyc files embed a build path
                        (local experiments only; the bundle would leak it)
  -h, --help            this text
EOF
}

while (($# > 0)); do
  case "$1" in
    --payload)
      [[ -n "${2:-}" ]] || fail "--payload needs a directory"
      SRC="$2"
      shift 2
      ;;
    --payload=*)
      SRC="${1#*=}"
      shift
      ;;
    --out)
      [[ -n "${2:-}" ]] || fail "--out needs a directory"
      OUT="$2"
      shift 2
      ;;
    --out=*)
      OUT="${1#*=}"
      shift
      ;;
    --level)
      [[ "${2:-}" =~ ^[0-9]+$ ]] || fail "--level needs a number"
      LEVEL="$2"
      shift 2
      ;;
    --level=*)
      LEVEL="${1#*=}"
      shift
      ;;
    --allow-build-paths)
      allow_build_paths=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) fail "unknown argument: $1 (try --help)" ;;
  esac
done

((LEVEL >= 1 && LEVEL <= 22)) || fail "zstd level must be 1..22, got $LEVEL"

# ---------------------------------------------------------------- preflight

[[ -d "$SRC/hermes-payload" ]] || fail "payload not found at $SRC/hermes-payload (build it with scripts/build-python-payload.sh, or pass --payload)"
[[ -d "$SRC/python" ]] || fail "bundled interpreter not found at $SRC/python"
[[ -x "$SRC/hermes-payload/bin/hermes-sidecar" ]] || fail "payload launcher is missing or not executable: $SRC/hermes-payload/bin/hermes-sidecar"
[[ -f "$SRC/hermes-payload/MANIFEST" ]] || fail "missing $SRC/hermes-payload/MANIFEST"

command -v python3 >/dev/null 2>&1 || fail "python3 is required (it compresses the tar stream and self-checks the archive)"
python3 -c 'import compression.zstd' 2>/dev/null ||
  fail "python3 $(python3 -c 'import sys; print("%d.%d" % sys.version_info[:2])') has no compression.zstd module (needs CPython 3.14+, PEP 784)"

# The payload's own identity. `built` is the field that changes with every
# rebuild, so it is what names the unpacked directory under the app-data dir.
built="$(sed -n 's/^built=//p' "$SRC/hermes-payload/MANIFEST" | head -n 1 | tr -d '\r')"
[[ -n "$built" ]] || fail "$SRC/hermes-payload/MANIFEST has no 'built' field, so the payload has no version"
version="$(printf '%s' "$built" | tr -c 'A-Za-z0-9._-' '-')"

# ------------------------------------------------------- source-tree checks

# Symlink audit + byte accounting in one real walk of the tree.
summary="$(
  python3 - "$SRC" <<'PY'
import os, stat, sys

src = sys.argv[1]
files = unpacked = symlinks = dirs = others = 0
bad = []

for base in ("hermes-payload", "python"):
    for dirpath, dirnames, filenames in os.walk(os.path.join(src, base), followlinks=False):
        rel_dir = os.path.relpath(dirpath, src)
        parts = [] if rel_dir == "." else rel_dir.split(os.sep)
        for name in filenames + dirnames:
            path = os.path.join(dirpath, name)
            info = os.lstat(path)
            rel = os.path.relpath(path, src)
            if stat.S_ISLNK(info.st_mode):
                symlinks += 1
                target = os.readlink(path)
                if not target:
                    bad.append((rel, target, "empty target"))
                    continue
                if os.path.isabs(target):
                    bad.append((rel, target, "absolute target"))
                    continue
                # Resolve the target lexically: a link may only walk up as far
                # as the payload root.
                stack = list(parts)
                escaped = False
                for component in target.split(os.sep):
                    if component in ("", "."):
                        continue
                    if component == "..":
                        if not stack:
                            escaped = True
                            break
                        stack.pop()
                    else:
                        stack.append(component)
                if escaped:
                    bad.append((rel, target, "leaves the payload tree"))
            elif stat.S_ISDIR(info.st_mode):
                dirs += 1
            elif stat.S_ISREG(info.st_mode):
                files += 1
                unpacked += info.st_size
            else:
                others += 1
                bad.append((rel, "", "not a file, directory or symlink"))

for rel, target, why in bad[:20]:
    print(f"bad entry: {rel} -> {target} ({why})", file=sys.stderr)
if bad:
    sys.exit(1)

print(f"files {files}")
print(f"symlinks {symlinks}")
print(f"dirs {dirs}")
print(f"others {others}")
print(f"unpacked {unpacked}")
PY
)" || fail "the payload has symlinks that cannot be packed safely (see above)"

files="$(printf '%s\n' "$summary" | sed -n 's/^files //p')"
unpacked="$(printf '%s\n' "$summary" | sed -n 's/^unpacked //p')"
symlinks="$(printf '%s\n' "$summary" | sed -n 's/^symlinks //p')"
log "source: $files files, $symlinks symlinks, $unpacked unpacked bytes, version $version (MANIFEST built=)"

# Bytecode written by an in-place run embeds the absolute path it was imported
# from, which would put a build-machine path inside the shipped archive.
src_abs="$(cd -- "$SRC" && pwd -P)"
needles=("$src_abs")
if [[ -n "${HOME:-}" && "$HOME" != "/" && "$HOME" != "$src_abs" ]]; then
  needles+=("$HOME")
fi

pyc_total=0
pyc_hits=0
pyc_needle=""
while IFS= read -r -d '' pyc; do
  pyc_total=$((pyc_total + 1))
  for needle in "${needles[@]}"; do
    if grep -q -a -F -- "$needle" "$pyc"; then
      pyc_hits=$((pyc_hits + 1))
      [[ -z "$pyc_needle" ]] && pyc_needle="$needle"
      break
    fi
  done
done < <(find "$SRC/hermes-payload" "$SRC/python" -name '*.pyc' -type f -print0)

if ((pyc_hits > 0)); then
  if ((allow_build_paths)); then
    log "warning: $pyc_hits of $pyc_total *.pyc files embed the build path $pyc_needle; packing anyway because --allow-build-paths was given"
  else
    log "error: $pyc_hits of $pyc_total *.pyc files embed the build path $pyc_needle"
    log "       the shipped archive must not carry a build-machine path; rebuild the payload with"
    log "       scripts/build-python-payload.sh (its build sets PYTHONDONTWRITEBYTECODE=1 and strips __pycache__)"
    log "       pass --allow-build-paths only for a local experiment"
    exit 1
  fi
fi

# ------------------------------------------------------------------- packing

mkdir -p -- "$OUT"
[[ -d "$OUT" && ! -L "$OUT" ]] || fail "output is not a directory: $OUT"

tmp_archive="$(mktemp -p "$OUT" ".$ARCHIVE_NAME.XXXXXX")"
tmp_manifest="$(mktemp -p "$OUT" ".$MANIFEST_NAME.XXXXXX")"
cleanup() {
  [[ -n "$tmp_archive" && -e "$tmp_archive" ]] && rm -f -- "$tmp_archive"
  [[ -n "$tmp_manifest" && -e "$tmp_manifest" ]] && rm -f -- "$tmp_manifest"
  return 0
}
trap cleanup EXIT

log "packing tar+zstd level $LEVEL ($WORKERS workers) from $SRC"
started="$(date +%s)"

# GNU tar 1.28+ flags: --sort=name plus a fixed mtime and numeric owner make the
# stream byte-identical for the same input tree. `-C "$SRC"` keeps the member
# names relative, so no build path can appear in a name.
COMPRESS_PY='
import sys, compression.zstd as zstd

path, level, workers = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
options = {zstd.CompressionParameter.compression_level: level}
if workers > 1:
    options[zstd.CompressionParameter.nb_workers] = workers
with open(path, "wb") as raw, zstd.ZstdFile(raw, "wb", options=options) as dst:
    src = sys.stdin.buffer
    while True:
        chunk = src.read(1 << 20)
        if not chunk:
            break
        dst.write(chunk)
'

tar --sort=name \
  --mtime=@0 \
  --owner=0 --group=0 --numeric-owner \
  --format=gnu \
  --no-acls --no-xattrs --no-selinux \
  -cf - -C "$SRC" hermes-payload python |
  python3 -c "$COMPRESS_PY" "$tmp_archive" "$LEVEL" "$WORKERS"

sha="$(sha256sum -- "$tmp_archive" | cut -d' ' -f1)"
archive_bytes="$(stat -c %s -- "$tmp_archive")"

# Self-check: every member name must be relative and free of `..`.
entries="$(
  python3 - "$tmp_archive" <<'PY'
import sys, tarfile, compression.zstd as zstd

bad = []
count = 0
with zstd.open(sys.argv[1], "rb") as raw, tarfile.open(fileobj=raw, mode="r|") as tar:
    for member in tar:
        count += 1
        name = member.name
        if name.startswith("/") or ".." in name.split("/"):
            bad.append(name)
if bad:
    for name in bad[:10]:
        print(f"unsafe member name: {name}", file=sys.stderr)
    sys.exit(1)
print(count)
PY
)" || fail "the archive contains an absolute or '..' member name"

mv -f -- "$tmp_archive" "$OUT/$ARCHIVE_NAME"
tmp_archive=""
# mktemp creates 0600; the bundler may run as another user than the packer.
chmod 0644 -- "$OUT/$ARCHIVE_NAME"

python3 - "$tmp_manifest" "$version" "$sha" "$archive_bytes" "$unpacked" "$files" <<'PY'
import json, sys

out, version, sha, archive_bytes, unpacked_bytes, files = sys.argv[1:7]
with open(out, "w") as handle:
    json.dump(
        {
            "version": version,
            "sha256": sha,
            "archive_bytes": int(archive_bytes),
            "unpacked_bytes": int(unpacked_bytes),
            "files": int(files),
        },
        handle,
        indent=2,
    )
    handle.write("\n")
PY
mv -f -- "$tmp_manifest" "$OUT/$MANIFEST_NAME"
tmp_manifest=""
chmod 0644 -- "$OUT/$MANIFEST_NAME"

elapsed=$(($(date +%s) - started))
log "wrote $OUT/$ARCHIVE_NAME ($archive_bytes bytes, sha256 $sha)"
log "wrote $OUT/$MANIFEST_NAME (version $version, $files files, $unpacked unpacked bytes)"
log "packed $entries archive entries in ${elapsed}s"
log "build the payload bundle with:"
printf '%s\n' "scripts/builder.sh bash -c 'cd src-tauri && cargo tauri build --bundles deb,appimage --config tauri.payload.conf.json'" >&2

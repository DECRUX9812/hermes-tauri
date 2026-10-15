#!/usr/bin/env bash
# Stage the bundled Python payload into src-tauri/payload/ so a payload build can
# ship it as Tauri resources (src-tauri/tauri.payload.conf.json).
#
#   scripts/stage-payload.sh          # stage (hardlinks when possible)
#   scripts/stage-payload.sh --clean  # remove the staged tree
#
# Source: $HERMES_TAURI_PAYLOAD_DIR (default $HOME/.cache/hermes-tauri/payload),
# built by scripts/build-python-payload.sh. The staged tree is generated, is
# gitignored, and is never read at runtime: the bundler copies it into the app's
# resource directory.
#
# The relative symlinks inside the payload are staged as symlinks (cp -a/-al).
# Tauri's own resource copy is what dereferences them; see
# scripts/payload-deref-boot-test.sh.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/src-tauri/payload"
SRC="${HERMES_TAURI_PAYLOAD_DIR:-$HOME/.cache/hermes-tauri/payload}"
BUILD_LINE="scripts/builder.sh bash -c 'cd src-tauri && cargo tauri build --bundles deb,appimage --config tauri.payload.conf.json'"

log() { printf '[stage-payload] %s\n' "$*" >&2; }
fail() { printf '[stage-payload] error: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
usage: scripts/stage-payload.sh [--clean]

Stages $HERMES_TAURI_PAYLOAD_DIR/{hermes-payload,python} (default
$HOME/.cache/hermes-tauri/payload) into src-tauri/payload/ and prints the
payload build command. --clean removes the staged tree.
EOF
}

clean=0
while (($# > 0)); do
  case "$1" in
    --clean) clean=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) fail "unknown argument: $1 (try --help)" ;;
  esac
  shift
done

# The staged tree is the only path this script may write to.
[[ "$DEST" == "$ROOT/src-tauri/payload" ]] || fail "refusing to stage outside src-tauri/payload: $DEST"

if ((clean)); then
  if [[ -e "$DEST" || -L "$DEST" ]]; then
    [[ -d "$DEST" && ! -L "$DEST" ]] || fail "refusing to remove a non-directory: $DEST"
    rm -rf -- "$DEST"
    log "removed $DEST"
  else
    log "nothing to clean: $DEST does not exist"
  fi
  exit 0
fi

[[ -d "$SRC/hermes-payload" ]] || fail "payload not found at $SRC/hermes-payload (build it with scripts/build-python-payload.sh, or set HERMES_TAURI_PAYLOAD_DIR)"
[[ -d "$SRC/python" ]] || fail "bundled interpreter not found at $SRC/python"
[[ -x "$SRC/hermes-payload/bin/hermes-sidecar" ]] || fail "payload launcher is missing or not executable: $SRC/hermes-payload/bin/hermes-sidecar"

[[ ! -e "$DEST" || -d "$DEST" ]] || fail "refusing to replace a non-directory: $DEST"
[[ ! -L "$DEST" ]] || fail "refusing to stage into a symlink: $DEST"
mkdir -p -- "$DEST"

# Refresh in place: the staged tree is generated, so replacing it is safe.
rm -rf -- "$DEST/hermes-payload" "$DEST/python"

# cp -al hardlinks (no data copy, same filesystem only); cp -a copies for real,
# which is 341 MB of I/O and takes the shared build lock.
same_fs() { [[ "$(stat -c %d -- "$1" 2>/dev/null)" == "$(stat -c %d -- "$2" 2>/dev/null)" ]]; }
if same_fs "$SRC" "$DEST"; then
  log "hardlinking $SRC/{hermes-payload,python} into $DEST"
  cp -al -- "$SRC/hermes-payload" "$SRC/python" "$DEST/"
else
  log "copying $SRC/{hermes-payload,python} into $DEST (under the shared build lock)"
  flock /tmp/hermes-build.lock cp -a -- "$SRC/hermes-payload" "$SRC/python" "$DEST/"
fi

[[ -x "$DEST/hermes-payload/bin/hermes-sidecar" ]] || fail "staged launcher lost its exec bit: $DEST/hermes-payload/bin/hermes-sidecar"

# Bytecode written by an in-place run after the payload was built embeds the
# build path (the payload builder strips __pycache__ at build time).
pycache="$(find "$DEST" -name __pycache__ -type d | wc -l)"
if [[ "$pycache" != 0 ]]; then
  log "warning: $pycache __pycache__ directories in the staged tree; rebuild the payload (scripts/build-python-payload.sh) so the bundle carries no build path"
fi

log "launcher mode $(stat -c %A -- "$DEST/hermes-payload/bin/hermes-sidecar"), symlinks $(find "$DEST" -type l | wc -l), size $(du -sh -- "$DEST" | cut -f1)"
log "staged; build the payload bundle with:"
printf '%s\n' "$BUILD_LINE"

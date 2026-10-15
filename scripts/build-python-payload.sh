#!/usr/bin/env bash
# Build the relocatable Python payload that lets the Tauri shell ship its own
# Hermes backend: a python-build-standalone interpreter, a relocatable venv with
# stock Hermes (the pinned checkout) plus the hermes_plus package installed
# non-editable, the Desktop webapp renderer, and a small launcher.
#
# Idempotent: re-running refreshes the payload in place. Never writes inside the
# source checkouts. Outcome: $HERMES_TAURI_PAYLOAD_DIR/hermes-payload/.
#
# Usage:
#   scripts/build-python-payload.sh [--slim] [--force] [--keep-pyc]
#   HERMES_PIN=<dir> HERMES_PLUS_SRC=<dir> HERMES_TAURI_PAYLOAD_DIR=<dir> ...
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

PAYLOAD_ROOT="${HERMES_TAURI_PAYLOAD_DIR:-$HOME/.cache/hermes-tauri/payload}"
PYDIR="$PAYLOAD_ROOT/python"
OUT="$PAYLOAD_ROOT/hermes-payload"
VENV="$OUT/venv"
UI_DIST="$OUT/ui_dist"
BUILD_LOCK="${HERMES_BUILD_LOCK:-/tmp/hermes-build.lock}"
PY_VERSION="${HERMES_PAYLOAD_PYTHON:-3.14}"
MIN_FREE_GB="${HERMES_PAYLOAD_MIN_FREE_GB:-30}"

SLIM=0
FORCE=0

# Never let a build step drop bytecode caches carrying this machine's paths.
export PYTHONDONTWRITEBYTECODE=1

log() { printf '[payload] %s\n' "$*" >&2; }
fail() { printf '[payload] error: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --slim) SLIM=1; shift ;;
    --force) FORCE=1; shift ;;
    --keep-pyc) HERMES_PAYLOAD_KEEP_PYC=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown argument: $1" ;;
  esac
done

# --- locate the read-only source checkouts -----------------------------------
# Both are siblings of the worktree; env overrides win.
find_source() {
  local var_name="$1"; shift
  local override="${!var_name:-}"
  local candidate
  if [[ -n "$override" ]]; then
    [[ -d "$override" ]] || fail "$var_name=$override is not a directory"
    printf '%s\n' "$(cd -- "$override" && pwd)"
    return 0
  fi
  for candidate in "$@"; do
    if [[ -d "$candidate" ]]; then
      printf '%s\n' "$(cd -- "$candidate" && pwd)"
      return 0
    fi
  done
  fail "could not locate source; set $var_name"
}

PIN="$(find_source HERMES_PIN \
  "$REPO_ROOT/../_hermes-pin" \
  "$REPO_ROOT/../../_hermes-pin")"
PLUS_SRC="$(find_source HERMES_PLUS_SRC \
  "$REPO_ROOT/../../hermes-desktop-plus/packages/hermes_plus" \
  "$REPO_ROOT/../hermes-desktop-plus/packages/hermes_plus")"

[[ -f "$PIN/hermes" ]] || fail "pinned Hermes checkout looks wrong: $PIN"
[[ -f "$PIN/apps/desktop/dist-webapp/index.html" ]] || \
  fail "pinned webapp renderer is missing (--skip-build payload expects it): $PIN/apps/desktop/dist-webapp"
[[ -f "$PLUS_SRC/pyproject.toml" ]] || fail "hermes_plus source looks wrong: $PLUS_SRC"

command -v uv >/dev/null 2>&1 || fail "uv is required"
command -v flock >/dev/null 2>&1 || fail "flock is required"

with_lock() { flock "$BUILD_LOCK" "$@"; }

check_disk() {
  local avail_kb
  mkdir -p "$PAYLOAD_ROOT"
  avail_kb="$(df -Pk "$PAYLOAD_ROOT" | awk 'NR==2 {print $4}')"
  [[ "$avail_kb" =~ ^[0-9]+$ ]] || fail "could not read free space for $PAYLOAD_ROOT"
  if (( avail_kb < MIN_FREE_GB * 1024 * 1024 )); then
    fail "free disk is below ${MIN_FREE_GB}G ($((avail_kb / 1024 / 1024))G available); stopping"
  fi
}

# --- steps -------------------------------------------------------------------

install_python() {
  mkdir -p "$PYDIR"
  export UV_PYTHON_INSTALL_DIR="$PYDIR"
  log "ensuring python-build-standalone CPython $PY_VERSION in $PYDIR"
  with_lock uv python install --install-dir "$PYDIR" "$PY_VERSION"
}

managed_python() {
  local candidate
  candidate="$(ls -d "$PYDIR"/cpython-"${PY_VERSION}"*/bin/python3."${PY_VERSION#3.}" 2>/dev/null | sort -V | tail -1 || true)"
  [[ -n "$candidate" ]] || fail "managed Python $PY_VERSION not found under $PYDIR"
  printf '%s\n' "$candidate"
}

create_venv() {
  local base_python="$1"
  export UV_PYTHON_INSTALL_DIR="$PYDIR"
  if [[ "$FORCE" == 1 || ! -x "$VENV/bin/python" ]]; then
    log "creating relocatable venv at $VENV"
    rm -rf -- "$VENV"
    mkdir -p "$OUT"
    with_lock uv venv --relocatable --python "$base_python" "$VENV"
  else
    log "reusing existing venv at $VENV"
  fi
}

install_packages() {
  # Stock Hermes (pinned) is installed NON-editable so the payload never points
  # back at the checkout. Its setup.py refuses to build wheels outside a Nix
  # sandbox unless HERMES_NIX_BUILD=1, so set it for this deliberate packaging
  # build. hermes_plus is installed normally; it pulls a starlette < 1 that also
  # satisfies the pin's core fastapi range (the exact starlette==1.3.1 lives only
  # in the pin's [web]/[mcp] extras and conflicts with hermes_plus's < 1).
  log "installing stock Hermes (pinned) and hermes_plus (non-editable)"
  HERMES_NIX_BUILD=1 with_lock uv pip install --python "$VENV/bin/python" "$PIN" "$PLUS_SRC"
}

copy_webapp() {
  if [[ -d "$UI_DIST" && "$FORCE" != 1 ]]; then
    log "webapp renderer already present at $UI_DIST"
    return 0
  fi
  log "copying pinned dist-webapp into the payload (served via --ui-dir)"
  rm -rf -- "$UI_DIST"
  mkdir -p "$UI_DIST"
  cp -a "$PIN/apps/desktop/dist-webapp/." "$UI_DIST/"
}

fix_relocation() {
  # uv's relocatable venv rewrites console-script shebangs to a /bin/sh wrapper,
  # but bin/python stays an absolute symlink to the managed interpreter. Repoint
  # it relative so the whole payload can be moved (and greps clean of the build
  # path). Also normalise pyvenv.cfg's home to the same relative target.
  local base_python rel_from_bin
  base_python="$(managed_python)"
  rel_from_bin="$(python3 - "$base_python" "$VENV/bin" <<'PY'
import os, sys
print(os.path.relpath(sys.argv[1], sys.argv[2]))
PY
)"
  log "repointing venv interpreter symlinks relative to the payload"
  ln -sfn "$rel_from_bin" "$VENV/bin/python"
  ln -sfn "python" "$VENV/bin/python3"
  if [[ -e "$VENV/bin/python${PY_VERSION#3.}" || -L "$VENV/bin/python${PY_VERSION#3.}" ]]; then
    ln -sfn "python" "$VENV/bin/python${PY_VERSION#3.}"
  fi
  local rel_home
  rel_home="$(dirname -- "$rel_from_bin")"
  if [[ -f "$VENV/pyvenv.cfg" ]]; then
    python3 - "$VENV/pyvenv.cfg" "$rel_home" <<'PY'
import sys
path, home = sys.argv[1], sys.argv[2]
lines = []
for line in open(path, encoding="utf-8"):
    if line.startswith("home ="):
        line = f"home = {home}\n"
    lines.append(line)
open(path, "w", encoding="utf-8").write("".join(lines))
PY
  fi

  # `uv python install` also leaves an unversioned alias symlink
  # (cpython-3.14-linux-x86_64-gnu) that points at the versioned directory by
  # ABSOLUTE path. A grep of file *contents* cannot see symlink targets, so
  # repoint every absolute symlink that resolves inside the payload itself.
  local link target rel
  while IFS= read -r -d '' link; do
    target="$(readlink -- "$link")"
    case "$target" in
      "$PAYLOAD_ROOT"/*)
        rel="$(python3 -c 'import os,sys; print(os.path.relpath(sys.argv[1], os.path.dirname(sys.argv[2])))' "$target" "$link")"
        ln -sfn -- "$rel" "$link"
        ;;
    esac
  done < <(find "$PYDIR" "$OUT" -type l -print0)
}

sanitize_paths() {
  # Bytecode caches embed the absolute build path in co_filename, and the wheel
  # installers drop direct_url.json naming the source checkouts. Both break the
  # "no file references the build path or the sources" rule and are regenerated
  # (or unused) at runtime, so drop them, then self-check.
  log "stripping bytecode caches and installer source references"
  find "$OUT" "$PYDIR" -type d -name __pycache__ -prune -exec rm -rf -- {} + 2>/dev/null || true
  find "$VENV" -path '*.dist-info/direct_url.json' -delete 2>/dev/null || true

  # CPython's generated sysconfig data bakes the interpreter's install prefix
  # into every *_sysconfigdata*.py. sysconfig overrides prefix/exec_prefix from
  # sys.base_prefix at runtime (verified by running the moved payload), so
  # replacing the build path with the upstream build prefix is behaviour-safe
  # and leaves no reference to this machine's build location.
  local sysdata
  for sysdata in "$PYDIR"/cpython-"${PY_VERSION}"*/lib/python"${PY_VERSION}"/_sysconfigdata*.py; do
    [[ -f "$sysdata" ]] || continue
    python3 - "$sysdata" "$PAYLOAD_ROOT" <<'PY'
import sys
path, old = sys.argv[1], sys.argv[2]
data = open(path, encoding="utf-8").read()
open(path, "w", encoding="utf-8").write(data.replace(old, "/install"))
PY
  done

  local hits src_hits link_hits
  hits="$(grep -rl -F "$PAYLOAD_ROOT" "$OUT" "$PYDIR" 2>/dev/null || true)"
  [[ -z "$hits" ]] || fail "payload still references build path:
$hits"
  src_hits="$(grep -rl -F "$PIN" "$OUT" 2>/dev/null || true)
$(grep -rl -F "$PLUS_SRC" "$OUT" 2>/dev/null || true)"
  [[ -z "${src_hits//$'\n'/}" ]] || fail "payload still references source checkouts:
$src_hits"
  # Symlink targets are invisible to grep -rl, so check them explicitly.
  link_hits="$(find "$OUT" "$PYDIR" -type l -print0 2>/dev/null | while IFS= read -r -d '' link; do
    target="$(readlink -- "$link")"
    case "$target" in "$PAYLOAD_ROOT"*) printf '%s -> %s\n' "$link" "$target" ;; esac
  done)"
  [[ -z "$link_hits" ]] || fail "payload symlinks still reference build path:
$link_hits"
}

prune_payload() {
  # --slim applies the two largest *safe* trims: non-runtime interpreter files
  # and build tooling. Neither is imported by the serving path (a headless HTTP
  # server); the heavy third-party dependencies stay, since sidelining them
  # would break dashboard features rather than just shrink the payload.
  [[ "$SLIM" == 1 ]] || return 0
  local lib_root stdlib rc="$(ls -d "$PYDIR"/cpython-"${PY_VERSION}"* 2>/dev/null | head -1 || true)"
  stdlib="$(ls -d "$PYDIR"/cpython-"${PY_VERSION}"*/lib/python"${PY_VERSION}" 2>/dev/null | head -1 || true)"

  log "--slim: removing non-runtime interpreter files (terminfo, headers, stdlib UIs)"
  if [[ -n "$rc" ]]; then
    # terminfo/man (curses DB), C headers and config (only needed to compile), and
    # stdlib packages no server imports (IDLE, Tk, the venv/ensurepip bootstrap).
    rm -rf -- \
      "$rc/share/terminfo" \
      "$rc/share/man" \
      "$rc/include" \
      "$rc"/lib/python"${PY_VERSION}"/config-* \
      "$rc"/lib/python"${PY_VERSION}"/pydoc_data 2>/dev/null || true
  fi
  if [[ -n "$stdlib" ]]; then
    rm -rf -- \
      "$stdlib/test" \
      "$stdlib/idlelib" \
      "$stdlib/tkinter" \
      "$stdlib/turtledemo" \
      "$stdlib/ensurepip" \
      "$stdlib/lib2to3" \
      "$stdlib/venv" 2>/dev/null || true
  fi

  log "--slim: removing build tooling (pip, setuptools, wheel)"
  rm -rf -- "$VENV"/lib/python"${PY_VERSION}"/site-packages/pip* \
            "$VENV"/lib/python"${PY_VERSION}"/site-packages/setuptools* \
            "$VENV"/lib/python"${PY_VERSION}"/site-packages/wheel* 2>/dev/null || true
  if [[ -n "$rc" ]]; then
    rm -rf -- "$rc"/lib/python"${PY_VERSION}"/site-packages/pip* \
              "$rc"/lib/python"${PY_VERSION}"/site-packages/setuptools* \
              "$rc"/lib/python"${PY_VERSION}"/site-packages/wheel* \
              "$rc"/bin/pip "$rc"/bin/pip3 "$rc"/bin/pip3."${PY_VERSION#3.}" \
              "$rc"/bin/idle3 "$rc"/bin/idle3."${PY_VERSION#3.}" \
              "$rc"/bin/pydoc3 "$rc"/bin/pydoc3."${PY_VERSION#3.}" \
              "$rc"/bin/python3.14-config 2>/dev/null || true
  fi
}

write_launcher() {
  log "writing launcher $OUT/bin/hermes-sidecar"
  mkdir -p "$OUT/bin"
  cat > "$OUT/bin/hermes-sidecar" <<'SH'
#!/bin/sh
# Launcher for the bundled Hermes sidecar. Runs the hermes_plus server, which
# serves the Desktop UI from the bundled renderer and proxies the API to a stock
# Hermes dashboard child it starts itself. All paths are relative to this file,
# so the payload can be moved after building.
set -eu
OUT="$(cd -- "$(dirname -- "$0")/.." && pwd)"
VENV="$OUT/venv"
export HERMES_PLUS_HERMES_BIN="${HERMES_PLUS_HERMES_BIN:-$VENV/bin/hermes}"
export HERMES_PLUS_UI_DIR="${HERMES_PLUS_UI_DIR:-$OUT/ui_dist}"
export PATH="$VENV/bin:$PATH"
exec "$VENV/bin/python" -m hermes_plus.cli serve "$@"
SH
  chmod 0755 "$OUT/bin/hermes-sidecar"
}

write_manifest() {
  # Read versions from paths, never by importing: importing regenerates
  # bytecode caches that would re-introduce build-path references.
  local py_version plus_version
  py_version="$(basename -- "$(managed_python)" | sed 's/^python//')"
  plus_version="$(basename -- "$(ls -d "$VENV"/lib/python"${PY_VERSION}"/site-packages/hermes_plus-*.dist-info 2>/dev/null | head -1)" | sed 's/^hermes_plus-//; s/\.dist-info$//')"
  {
    printf 'python-version=%s\n' "$py_version"
    printf 'hermes-plus=%s\n' "$plus_version"
    printf 'slim=%s\n' "$SLIM"
    printf 'built=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } > "$OUT/MANIFEST"
}

main() {
  check_disk
  install_python
  local base_python
  base_python="$(managed_python)"
  create_venv "$base_python"
  install_packages
  copy_webapp
  fix_relocation
  prune_payload
  write_launcher
  write_manifest
  sanitize_paths
  log "payload ready: $OUT"
}

main "$@"

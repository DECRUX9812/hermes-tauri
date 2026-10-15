#!/usr/bin/env bash
# T16a: does the bundled Python payload still boot after Tauri's resource copy?
#
# Tauri's resource copy is what turns the staged tree into the bundle:
# tauri-utils ResourcePathsIter walks the tree with walkdir's default
# follow_links(false), tauri-bundler Settings::copy_resources copies every
# non-directory entry with fs_utils::copy_file -> std::fs::copy, and fs::copy
# follows symlinks. So the payload's relative symlinks arrive in the bundle as
# full copies ("dereferenced").
#
# This script reproduces that walker and copy semantics into a scratch tree,
# reports what the real bundler would do with each symlink, and then runs the
# real launcher from the copy exactly as the shell's bundled command does.
#
#   scripts/payload-deref-boot-test.sh [--staged DIR] [--work DIR] [--keep] [--skip-copy]
#
# --skip-copy reuses an existing --work/deref tree instead of rebuilding it (for
# a manual experiment on the copy, e.g. restoring the venv interpreter symlink).
# It only writes inside --work (default /tmp/hermes-tauri-t16a) and removes that
# tree again unless --keep is given. It never touches the staged payload.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
STAGED="$ROOT/src-tauri/payload"
WORK="/tmp/hermes-tauri-t16a"
KEEP=0
SKIP_COPY=0
PAYLOAD_SRC="${HERMES_TAURI_PAYLOAD_DIR:-$HOME/.cache/hermes-tauri/payload}"

log() { printf '[deref-test] %s\n' "$*" >&2; }
fail() { printf '[deref-test] FAIL: %s\n' "$*" >&2; exit 1; }
pass() { printf '[deref-test] PASS: %s\n' "$*" >&2; }

while (($# > 0)); do
  case "$1" in
    --staged)
      STAGED="$2"
      shift
      ;;
    --work)
      WORK="$2"
      shift
      ;;
    --keep) KEEP=1 ;;
    --skip-copy) SKIP_COPY=1 ;;
    *) fail "unknown argument: $1" ;;
  esac
  shift
done

COPY="$WORK/deref"
DATA="$WORK/data"
LOG="$WORK/launcher.log"

pgid=""
pid=""
cleanup() {
  if [[ -n "$pgid" ]] && kill -0 -- "-$pgid" 2>/dev/null; then
    kill -KILL -- "-$pgid" 2>/dev/null || true
  fi
  if ((KEEP)); then
    log "kept $WORK"
  else
    rm -rf -- "$WORK"
    log "removed $WORK"
  fi
}
trap cleanup EXIT

[[ -d "$STAGED/hermes-payload" ]] || fail "no staged payload at $STAGED (run scripts/stage-payload.sh first)"
[[ -x "$STAGED/hermes-payload/bin/hermes-sidecar" ]] || fail "staged launcher missing: $STAGED/hermes-payload/bin/hermes-sidecar"

rm -rf -- "$DATA"
if ((SKIP_COPY)); then
  [[ -d "$COPY/hermes-payload" ]] || fail "--skip-copy: no copy at $COPY"
else
  rm -rf -- "$COPY"
fi
mkdir -p -- "$COPY" "$DATA"

# ---------------------------------------------------------------- phase 1: walk
# What the real bundler does with each entry. `report` mode copies nothing.
log "phase 1: tauri-bundler walker semantics on $STAGED"
python3 - "$STAGED" <<'PY'
import os
import stat
import sys

src = sys.argv[1]

entries = 0
links_to_file = 0
links_to_dir = []
broken = []
first_error = None


def note_error(kind, path, target=None):
    global first_error
    if first_error is None:
        first_error = (kind, path, target)


def walk(spath):
    global entries, links_to_file
    st = os.lstat(spath)
    if stat.S_ISLNK(st.st_mode):
        if not os.path.exists(spath):  # resources.rs: path.exists() -> ResourcePathNotFound
            broken.append(spath)
            note_error("broken-symlink", spath, os.readlink(spath))
            return
        if os.path.isdir(spath):  # walkdir: is_dir() is false for a symlink
            links_to_dir.append((spath, os.readlink(spath)))
            note_error("not-a-file", spath, os.readlink(spath))
            return
        entries += 1
        links_to_file += 1  # fs::copy follows it -> full copy in the bundle
        return
    if stat.S_ISDIR(st.st_mode):
        for name in sorted(os.listdir(spath)):
            walk(os.path.join(spath, name))
        return
    entries += 1


walk(src)
print(f"walker: non-directory entries={entries} symlinks-to-file={links_to_file} symlinks-to-dir={len(links_to_dir)} broken={len(broken)}")
for path, target in links_to_dir:
    print(f"walker: symlink-to-directory {path} -> {target}")
for path in broken:
    print(f"walker: broken symlink {path}")
if first_error is None:
    print("walker: fs_utils::copy_file would accept every entry")
else:
    kind, path, target = first_error
    if kind == "not-a-file":
        print(f'walker: fs_utils::copy_file would stop with: "{path}" is not a file (target {target})')
    else:
        print(f"walker: resources.rs would stop with ResourcePathNotFound({path})")
PY

# ------------------------------------------------------------- phase 2: the copy
# The tree Tauri produces: symlink-to-file entries dereferenced (fs::copy keeps
# the mode bits), real directories walked. Entries the bundler cannot copy at
# all (symlinks to directories) are reported in phase 1 and skipped here, so the
# copy models a bundle whose staging dropped them.
log "phase 2: copying $STAGED -> $COPY with symlinks dereferenced"
if ((SKIP_COPY)); then
  log "  --skip-copy: reusing the existing copy"
else
  python3 - "$STAGED" "$COPY" <<'PY'
import os
import shutil
import stat
import sys

_, src, dest = sys.argv

files = 0
dereferenced = 0
skipped = 0


def walk(spath, dpath):
    global files, dereferenced, skipped
    st = os.lstat(spath)
    if stat.S_ISLNK(st.st_mode):
        if os.path.isdir(spath):
            skipped += 1  # fs_utils::copy_file rejects these (phase 1)
            return
        shutil.copy2(spath, dpath, follow_symlinks=True)
        files += 1
        dereferenced += 1
        return
    if stat.S_ISDIR(st.st_mode):
        os.makedirs(dpath, exist_ok=True)
        os.chmod(dpath, stat.S_IMODE(st.st_mode))
        for name in sorted(os.listdir(spath)):
            walk(os.path.join(spath, name), os.path.join(dpath, name))
        return
    shutil.copy2(spath, dpath, follow_symlinks=True)
    files += 1


walk(src, dest)
print(f"copy: files={files} dereferenced-symlinks={dereferenced} skipped-symlink-to-dir={skipped}")
PY
fi

# ----------------------------------------------------------- phase 3: mode bits
log "phase 3: exec bits in the copy"
for f in hermes-payload/bin/hermes-sidecar hermes-payload/venv/bin/python python/cpython-3.14.8-linux-x86_64-gnu/bin/python3.14; do
  [[ -e "$COPY/$f" ]] || fail "missing in the copy: $f"
  mode="$(stat -c %A "$COPY/$f")"
  [[ -x "$COPY/$f" ]] || fail "not executable in the copy: $f ($mode)"
  log "  $f $mode"
done
pass "launcher and interpreter are executable in the dereferenced copy"

# ------------------------------------------------- phase 4: does the venv still work?
log "phase 4: the copied venv resolves its own prefix"
# PYTHONDONTWRITEBYTECODE keeps the copy byte-identical to what the bundler
# would produce (the payload's own bytecode is already there).
if ! PREFIX_OUT="$(PYTHONDONTWRITEBYTECODE=1 "$COPY/hermes-payload/venv/bin/python" -c 'import sys; print(sys.prefix, sys.base_prefix, sys.executable)')"; then
  fail "the copied venv cannot start (see the interpreter error above)"
fi
log "  sys.prefix base_prefix executable: $PREFIX_OUT"
python3 - "$COPY" "$PREFIX_OUT" <<'PY'
import sys

copy, out = sys.argv[1], sys.argv[2]
prefix, base_prefix, executable = out.split()
for name, value in (("sys.prefix", prefix), ("sys.base_prefix", base_prefix), ("sys.executable", executable)):
    inside = value.startswith(copy + "/")
    print(f"  {name}={value} inside-copy={inside}")
    if not inside:
        sys.exit(f"FAIL: {name} points outside the copy: {value}")
PY
pass "the copied venv's prefix, base_prefix and executable all point inside the copy"

IMPORT_OUT="$(PYTHONDONTWRITEBYTECODE=1 "$COPY/hermes-payload/venv/bin/python" -c 'import hermes_plus, sys; print(hermes_plus.__file__)')" ||
  fail "the copied venv cannot import hermes_plus"
log "  import hermes_plus: $IMPORT_OUT"

log "phase 4b: absolute paths left in the copy"
for pattern in "$STAGED" "$PAYLOAD_SRC"; do
  hits="$(grep -r -l -F -- "$pattern" "$COPY" 2>/dev/null | wc -l || true)"
  log "  grep -rl -F '$pattern' <copy>: $hits file(s)"
done

# ------------------------------------------------------------------- phase 5: boot
free_port() {
  python3 - <<'PY'
import socket

for port in range(18800, 18900):
    sock = socket.socket()
    try:
        sock.bind(("127.0.0.1", port))
    except OSError:
        sock.close()
        continue
    sock.close()
    print(port)
    raise SystemExit(0)
raise SystemExit("no free port in 18800-18899")
PY
}

PORT="$(free_port)"
log "phase 5: launching $COPY/hermes-payload/bin/hermes-sidecar --host 127.0.0.1 --port $PORT"
log "  HERMES_HOME=$DATA/hermes HERMES_RUNTIME_DIR=$DATA/runtime HERMES_DISABLE_LAZY_INSTALLS=1 PYTHONDONTWRITEBYTECODE=1"
start="$SECONDS"
setsid env \
  HERMES_HOME="$DATA/hermes" \
  HERMES_RUNTIME_DIR="$DATA/runtime" \
  HERMES_DISABLE_LAZY_INSTALLS=1 \
  PYTHONDONTWRITEBYTECODE=1 \
  "$COPY/hermes-payload/bin/hermes-sidecar" --host 127.0.0.1 --port "$PORT" >"$LOG" 2>&1 &
pid=$!
sleep 1
pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')" || fail "the launcher died at once: $(tail -20 "$LOG")"
log "  leader pid=$pid pgid=$pgid"

login_deadline=$((SECONDS + 90))
while ((SECONDS < login_deadline)); do
  if grep -q -F 'Hermes Plus login:' "$LOG"; then
    break
  fi
  kill -0 "$pid" 2>/dev/null || fail "the launcher exited before the login line: $(tail -20 "$LOG")"
  sleep 1
done
grep -q -F 'Hermes Plus login:' "$LOG" || fail "no 'Hermes Plus login:' line within 90 s"
login_at="$(grep -m1 -F 'Hermes Plus login:' "$LOG" | sed -E 's/([?&]token=)[^[:space:]]*/\1<redacted>/')"
log "  login line after $((SECONDS - login_deadline + 90))s: $login_at"

status="000"
health_deadline=$((SECONDS + 90))
while ((SECONDS < health_deadline)); do
  status="$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "http://127.0.0.1:$PORT/healthz" || true)"
  if [[ "$status" == "200" ]]; then
    break
  fi
  kill -0 "$pid" 2>/dev/null || fail "the launcher exited before /healthz was 200: $(tail -20 "$LOG")"
  sleep 1
done
[[ "$status" == "200" ]] || fail "GET /healthz was never 200 (last status $status)"
pass "GET /healthz = 200 after $((SECONDS - start))s (cold start)"
log "  /api/health without the token: $(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "http://127.0.0.1:$PORT/api/health" || true)"
log "  launcher log (redacted):"
sed -E 's/([?&]token=)[^[:space:]]*/\1<redacted>/g' "$LOG" | tail -30 | sed 's/^/    /'

# ------------------------------------------------------- phase 6: stop, no leftovers
scan_home() {
  local pid_dir scan_pid
  for pid_dir in /proc/[0-9]*; do
    scan_pid="${pid_dir#/proc/}"
    [[ "$scan_pid" == "$$" ]] && continue
    if tr '\0' '\n' <"$pid_dir/environ" 2>/dev/null | grep -q -x -F "HERMES_HOME=$DATA/hermes"; then
      printf '%s\n' "$scan_pid"
    fi
  done
  return 0
}

log "phase 6: SIGTERM to process group -$pgid"
kill -TERM -- "-$pgid" || true
stop_deadline=$((SECONDS + 20))
while ((SECONDS < stop_deadline)); do
  kill -0 -- "-$pgid" 2>/dev/null || break
  sleep 1
done
if kill -0 -- "-$pgid" 2>/dev/null; then
  fail "process group -$pgid still alive 20 s after SIGTERM"
fi
pass "process group -$pgid is gone"

leftovers="$(scan_home | tr '\n' ' ')"
log "  /proc scan for HERMES_HOME=$DATA/hermes: [${leftovers% }]"
if [[ -n "${leftovers// /}" ]]; then
  read -r -a left_pids <<<"$leftovers"
  for left in "${left_pids[@]}"; do
    kill -TERM "$left" 2>/dev/null || true
  done
  sleep 3
  for left in "${left_pids[@]}"; do
    kill -KILL "$left" 2>/dev/null || true
  done
  leftovers="$(scan_home | tr '\n' ' ')"
  [[ -z "${leftovers// /}" ]] || fail "processes with HERMES_HOME survived: $leftovers"
  pass "leftover processes were killed; the scan is now empty"
else
  pass "no process with that HERMES_HOME survived"
fi

# ------------------------------------------------------------------ phase 7: sizes
log "phase 7: sizes"
log "  staged (with symlinks): $(du -sh -- "$STAGED" | cut -f1) on disk, $(du -sh --apparent-size -- "$STAGED" | cut -f1) apparent"
log "  dereferenced copy:      $(du -sh -- "$COPY" | cut -f1) on disk, $(du -sh --apparent-size -- "$COPY" | cut -f1) apparent"
log "  staged: regular files=$(find "$STAGED" -type f | wc -l) symlinks=$(find "$STAGED" -type l | wc -l)"
log "  copy:   regular files=$(find "$COPY" -type f | wc -l) symlinks=$(find "$COPY" -type l | wc -l)"

pass "the dereferenced payload boots, serves /healthz and stops cleanly"

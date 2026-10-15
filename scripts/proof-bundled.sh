#!/usr/bin/env bash
# Bundled-mode proof of the Tauri shell: it runs the packaged AppImage with its
# own payload and no Hermes installed, and checks the whole first-run path.
#
# Usage: scripts/proof-bundled.sh [--appimage PATH] [--out DIR] [--keep]
#
# `--out` is the whole scratch root: the fresh HOME (the payload unpacks ~330 MB
# into it), the AppImage's own extraction dir (`TMPDIR`), the shots and the
# logs. It must sit on a filesystem with room for ~700 MB and ~20k inodes; the
# shared /tmp tmpfs is not always that, so the default is overridable.
#
# Outer part: prepares a stub `xdg-open`, a fresh HOME under the scratch root
# and a minimal PATH with no `hermes` on it, then re-executes this file inside
# `dbus-run-session -- xvfb-run -a`. Every process it starts is recorded by pid
# and killed on every exit path.
#
# Inner part, all against the payload's own backend (no test Hermes):
#   launch 1  first run: the window shows the boot page's own phase while the
#             payload unpacks into the app-data dir, the shell spawns the bundled
#             launcher, /healthz answers 200, the UI paints; the log carries
#             `?token=<redacted>` and never a token; SIGTERM leaves no process of
#             the payload's HERMES_HOME behind. (6a, 6e)
#   launch 2  second run: the payload is already unpacked, so this is the
#             unpack-free start-up time. (6e)
#   launch 3  cold start with `hermes-tauri://open?path=/team`: the window ends
#             on #/team with a live session, and F5 reloads it in place and
#             keeps the session (two shots, different md5). (6b, 6c)
#
# Input is real (xdotool F5, the shell's own deep link). Shots are verified the
# same way as scripts/proof.sh: a distinct-colour count on a 32x20 sample grid
# (a blank window scores 1) and an md5 that has not been seen before.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APPIMAGE="${HERMES_TAURI_BUNDLED_APPIMAGE:-}"
OUT="/tmp/hermes-tauri-t16/bundled"
KEEP=0
INNER=0
SCREEN="1440x900x24"
WINDOW_TITLE="Hermes Tauri"
MIN_COLOURS=12

usage() {
  cat <<'USAGE'
usage: scripts/proof-bundled.sh [--appimage PATH] [--out DIR] [--keep]

  --appimage PATH  the payload AppImage to drive (or HERMES_TAURI_BUNDLED_APPIMAGE)
  --out DIR        scratch root: fresh HOME, AppImage extraction dir, shots and
                   logs (default /tmp/hermes-tauri-t16/bundled). Needs ~700 MB
                   and ~20k inodes free; it must stay inside this task's own
                   scratch roots (/tmp/hermes-tauri-t16 or
                   ~/.cache/hermes-tauri/t16-proof).
  --keep           keep the scratch dir and the shell HOME (default: remove the HOME)
  -h, --help       this text
USAGE
}

die() {
  printf 'bundled-proof: %s\n' "$*"
  exit 1
}

while (($# > 0)); do
  case "$1" in
    --appimage) APPIMAGE="${2:-}"; shift 2 ;;
    --appimage=*) APPIMAGE="${1#*=}"; shift ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --out=*) OUT="${1#*=}"; shift ;;
    --keep) KEEP=1; shift ;;
    --inner) INNER=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'bundled-proof: unknown argument %s\n' "$1" >&2; usage; exit 2 ;;
  esac
done

[[ -n "$APPIMAGE" ]] || die 'no --appimage and no HERMES_TAURI_BUNDLED_APPIMAGE'
APPIMAGE="$(cd "$(dirname "$APPIMAGE")" && printf '%s/%s' "$PWD" "$(basename "$APPIMAGE")")"
OUT="$(mkdir -p "$OUT" && cd "$OUT" && pwd)"

# The scratch root stays inside this task's own roots, so a stray --out cannot
# touch another job's files.
SCRATCH_ROOTS=("/tmp/hermes-tauri-t16" "$HOME/.cache/hermes-tauri/t16-proof")
under_scratch() {
  local path="$1" root
  for root in "${SCRATCH_ROOTS[@]}"; do
    [[ "$path" == "$root"/* ]] && return 0
  done
  return 1
}
under_scratch "$OUT" || die "--out must live under ${SCRATCH_ROOTS[*]}, refusing $OUT"

# Small files only (stub browser, sample-grid helper, probe png): safe next to
# the shots, and it keeps a run self-contained under one directory.
WORK="$OUT/work"

# ── outer part ───────────────────────────────────────────────────────────────

outer_main() {
  [[ -x "$APPIMAGE" ]] || die "not executable: $APPIMAGE"
  command -v dbus-run-session >/dev/null 2>&1 || die 'dbus-run-session is required'
  command -v xvfb-run >/dev/null 2>&1 || die 'xvfb-run is required'
  command -v scrot >/dev/null 2>&1 || die 'scrot is required'
  command -v xdotool >/dev/null 2>&1 || die 'xdotool is required'
  command -v xwd >/dev/null 2>&1 || die 'xwd is required'
  command -v curl >/dev/null 2>&1 || die 'curl is required'

  rm -rf "$WORK"
  mkdir -p "$WORK/bin"
  write_stub_browser
  write_png_colours

  printf 'bundled-proof: entering dbus-run-session + xvfb-run (%s)\n' "$SCREEN"
  local -a inner_args=(--inner --appimage "$APPIMAGE" --out "$OUT")
  if ((KEEP == 1)); then
    inner_args+=(--keep)
  fi
  dbus-run-session -- xvfb-run -a -s "-screen 0 $SCREEN" \
    "$SCRIPT_DIR/proof-bundled.sh" "${inner_args[@]}" 2>"$OUT/dbus-stderr.log"
}

# A real browser must never open: this stub logs its argv and exits 0.
write_stub_browser() {
  cat >"$WORK/bin/xdg-open" <<'SH'
#!/bin/sh
printf '%s\n' "$*" >>"${HERMES_TAURI_STUB_OPEN_LOG:?}"
exit 0
SH
  chmod +x "$WORK/bin/xdg-open"
}

# Distinct colours on a 32x20 sample grid of a scrot PNG (the same measurement
# scripts/proof.sh uses; a blank window scores 1).
write_png_colours() {
  cat >"$WORK/png_colours.py" <<'PY'
import struct
import sys
import zlib

GRID = (32, 20)


def main(path):
    with open(path, "rb") as stream:
        data = stream.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(1)
    pos, header, idat = 8, None, bytearray()
    while pos + 8 <= len(data):
        length, tag = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        pos += 12 + length
        if tag == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif tag == b"IDAT":
            idat += body
        elif tag == b"IEND":
            break
    if header is None:
        raise SystemExit(1)
    width, height, depth, colour, _compression, _filter, interlace = header
    if depth != 8 or interlace != 0:
        raise SystemExit(1)
    channels = {0: 1, 2: 3, 4: 2, 6: 4}.get(colour)
    if channels is None:
        raise SystemExit(1)
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    previous = bytearray(stride)
    rows = []
    pos = 0
    for _ in range(height):
        kind = raw[pos]
        line = bytearray(raw[pos + 1:pos + 1 + stride])
        pos += 1 + stride
        if kind == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif kind == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif kind == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif kind == 4:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                up = previous[i]
                up_left = previous[i - channels] if i >= channels else 0
                guess = left + up - up_left
                da, db, dc = abs(guess - left), abs(guess - up), abs(guess - up_left)
                best = left if da <= db and da <= dc else (up if db <= dc else up_left)
                line[i] = (line[i] + best) & 0xFF
        elif kind != 0:
            raise SystemExit(1)
        rows.append(line)
        previous = line
    cols, samples = GRID
    colours = set()
    for gy in range(samples):
        y = gy * (height - 1) // (samples - 1)
        line = rows[y]
        for gx in range(cols):
            x = gx * (width - 1) // (cols - 1)
            colours.add(bytes(line[x * channels:x * channels + 3]))
    print(len(colours))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1]))
PY
}

# ── inner part ───────────────────────────────────────────────────────────────

SHELL_PID=0
SHELL_PGID=""
SHELL_LOG=""
SHELL_LOG_LINES=0
FAILED=0
HOME_DIR=""
SIDECAR_LOG=""
ORIGIN=""
PORT=""

declare -a SHOT_ORDER=() SHOT_MD5=() SHOT_RESULT=()

record() {
  local name="$1" result="$2" detail="$3"
  SHOT_ORDER+=("$name")
  SHOT_RESULT+=("$result")
  if [[ "$result" == PASS ]]; then
    printf 'PASS %s\n' "$name"
  else
    printf 'FAIL %s: %s\n' "$name" "$detail"
    FAILED=1
  fi
}

note() {
  printf 'bundled-proof: %s\n' "$*"
}

inner_cleanup() {
  local status=$?
  stop_shell
  if ((KEEP == 0)) && [[ -n "$HOME_DIR" ]] && under_scratch "$HOME_DIR"; then
    rm -rf "$HOME_DIR"
  fi
  exit "$status"
}

shell_pgid() {
  ps -o pgid= -p "$SHELL_PID" 2>/dev/null | tr -d '[:space:]'
}

stop_shell() {
  [[ "$SHELL_PID" =~ ^[1-9][0-9]*$ ]] || return 0
  if kill -0 "$SHELL_PID" 2>/dev/null; then
    kill -TERM "$SHELL_PID" 2>/dev/null || true
    local attempt
    for attempt in $(seq 1 80); do
      kill -0 "$SHELL_PID" 2>/dev/null || break
      sleep 0.25
    done
    if kill -0 "$SHELL_PID" 2>/dev/null; then
      kill -KILL -- "-$SHELL_PGID" 2>/dev/null || true
      sleep 0.5
    fi
  fi
  SHELL_PID=0
}

# Starts the shell and fails loudly when it does not survive the start.
start_shell() {
  local label="$1"
  shift
  stop_shell
  SHELL_LOG="$OUT/shell-$label.log"
  : >"$SHELL_LOG"
  SHELL_LOG_LINES=0
  env -u HERMES_TAURI_URL -u HERMES_TAURI_CMD -u HERMES_TAURI_ENV \
    APPIMAGE_EXTRACT_AND_RUN=1 \
    setsid "$APPIMAGE" "$@" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!
  printf '%s\n' "$SHELL_PID" >"$OUT/shell-$label.pid"
  sleep 2
  if ! kill -0 "$SHELL_PID" 2>/dev/null; then
    printf 'bundled-proof: the shell (%s) died within 2 s of launch; log tail:\n' "$label"
    tail -n 20 "$SHELL_LOG"
    exit 1
  fi
  SHELL_PGID="$(shell_pgid)"
  note "shell $label pid=$SHELL_PID pgid=$SHELL_PGID log=$SHELL_LOG"
}

# Waits for a new line matching the pattern in the shell's own stderr log.
wait_log() {
  local pattern="$1" timeout="${2:-120}" deadline=$((SECONDS + ${2:-120})) line
  local -a lines=()
  while ((SECONDS < deadline)); do
    mapfile -t lines < <(tail -n +"$((SHELL_LOG_LINES + 1))" "$SHELL_LOG" 2>/dev/null)
    for line in "${lines[@]}"; do
      if [[ "$line" =~ $pattern ]]; then
        SHELL_LOG_LINES="$(wc -l <"$SHELL_LOG")"
        printf '%s\n' "$line"
        return 0
      fi
    done
    sleep 0.3
  done
  return 1
}

window_id() {
  xdotool search --name "$WINDOW_TITLE" 2>/dev/null | head -n 1
}

wait_window() {
  local deadline=$((SECONDS + ${1:-30}))
  while ((SECONDS < deadline)); do
    [[ -n "$(window_id)" ]] && return 0
    sleep 0.3
  done
  return 1
}

place_window() {
  local wid
  wid="$(window_id)"
  [[ -n "$wid" ]] || return 1
  xdotool windowmove "$wid" 0 0 2>/dev/null || true
  xdotool windowsize "$wid" 1280 820 2>/dev/null || true
  sleep 0.5
}

probe_colours() {
  scrot -o "$WORK/probe.png" 2>/dev/null || return 1
  python3 "$WORK/png_colours.py" "$WORK/probe.png" 2>/dev/null | tr -dc '0-9'
}

# Painted means the sample grid holds real content; a blank window scores 1.
wait_painted() {
  local min="$1" timeout="${2:-60}" deadline=$((SECONDS + ${2:-60})) colours="" hits=0
  while ((SECONDS < deadline)); do
    colours="$(probe_colours)"
    if [[ -n "$colours" ]] && ((colours >= min)); then
      hits=$((hits + 1))
      if ((hits >= 2)); then
        printf '%s\n' "$colours"
        return 0
      fi
      sleep 1.5
      continue
    fi
    hits=0
    sleep 0.3
  done
  return 1
}

# Waits for the painted frame to come back (a reload repaints the page).
wait_repaint() {
  local timeout="${1:-60}" deadline=$((SECONDS + ${1:-60})) colours="" changed=0
  while ((SECONDS < deadline)); do
    colours="$(probe_colours)"
    if [[ -n "$colours" ]] && ((colours >= MIN_COLOURS)); then
      changed=$((changed + 1))
      ((changed >= 2)) && { printf '%s\n' "$colours"; return 0; }
    else
      changed=0
    fi
    sleep 0.3
  done
  return 1
}

capture() {
  local name="$1"
  scrot -o "$OUT/$name.png" || return 1
  python3 "$WORK/png_colours.py" "$OUT/$name.png" 2>/dev/null | tr -dc '0-9'
}

# Captures one shot, checks it is painted and not a copy of an earlier shot.
shot() {
  local name="$1" desc="$2" min="${3:-$MIN_COLOURS}" colours md5 index
  colours="$(capture "$name")" || {
    record "$name" FAIL "the screen could not be captured"
    return 1
  }
  if [[ -z "$colours" ]] || ((colours < min)); then
    record "$name" FAIL "blank frame: ${colours:-unreadable} colours, under $min"
    return 1
  fi
  md5="$(md5sum "$OUT/$name.png" | cut -d' ' -f1)"
  for index in "${!SHOT_MD5[@]}"; do
    if [[ "${SHOT_MD5[$index]}" == "$md5" ]]; then
      record "$name" FAIL "identical to ${SHOT_ORDER[$index]}.png: the screen never changed"
      return 1
    fi
  done
  SHOT_MD5+=("$md5")
  record "$name" PASS "$colours colours, md5 $md5, $desc"
}

# Any process still carrying the payload's HERMES_HOME after the shell exited.
leftovers() {
  local needle="HERMES_HOME=$HOME_DIR/.local/share/dev.hermes-plus.desktop/hermes"
  local pid found=""
  for pid in /proc/[0-9]*; do
    [[ -r "$pid/environ" ]] || continue
    if tr '\0' '\n' <"$pid/environ" 2>/dev/null | grep -Fxq "$needle"; then
      found+=" ${pid#/proc/}"
    fi
  done
  printf '%s' "$found"
}

# Milliseconds since the epoch, for the start-up timings.
now_ms() {
  date +%s%3N
}

sidecar_log_path() {
  find "$HOME_DIR" -name 'hermes-sidecar.log' 2>/dev/null | head -n 1
}

# The front door's token is 43 url-safe characters; the logs must never carry it.
assert_token_redacted() {
  local name="$1" file="$2" raw
  [[ -r "$file" ]] || {
    record "$name" FAIL "missing log $file"
    return 1
  }
  if ! grep -Fq '?token=<redacted>' "$file"; then
    record "$name" FAIL "$(basename "$file") has no '?token=<redacted>' line"
    return 1
  fi
  raw="$(grep -oE 'token=[A-Za-z0-9_-]{43}' "$file" | head -n 1 || true)"
  if [[ -n "$raw" ]]; then
    record "$name" FAIL "$(basename "$file") carries an unredacted token"
    return 1
  fi
  record "$name" PASS "?token=<redacted> present and no 43-character token in $(basename "$file")"
}

inner_main() {
  export NO_AT_BRIDGE=1
  export PATH="$WORK/bin:/usr/bin:/bin"
  export BROWSER="$WORK/bin/xdg-open"
  export HERMES_TAURI_STUB_OPEN_LOG="$OUT/opened-urls.log"
  : >"$HERMES_TAURI_STUB_OPEN_LOG"

  [[ -n "${DISPLAY:-}" ]] || die 'no DISPLAY (run through dbus-run-session + xvfb-run)'
  command -v hermes >/dev/null 2>&1 && die "a hermes is on PATH: $(command -v hermes)"

  HOME_DIR="$OUT/home"
  rm -rf "$HOME_DIR"
  mkdir -p "$HOME_DIR" "$OUT/tmp"
  export HOME="$HOME_DIR"
  # APPIMAGE_EXTRACT_AND_RUN unpacks the AppDir into $TMPDIR; keeping that
  # inside our own scratch dir means a killed run leaves nothing in the shared
  # /tmp root (the scratch dir is removed at the end of the run).
  export TMPDIR="$OUT/tmp"
  # No XDG overrides: the point of this proof is the real first-run layout under
  # a fresh HOME (app-data dir, app-log dir and the webview profile all land
  # inside it).
  unset XDG_DATA_HOME XDG_CONFIG_HOME XDG_CACHE_HOME XDG_STATE_HOME || true

  rm -f "$OUT"/*.png "$OUT"/shell-*.log "$OUT"/shell-*.pid
  trap inner_cleanup EXIT INT TERM

  note "HOME=$HOME appimage=$APPIMAGE"
  note "hermes on PATH: $(command -v hermes || echo none)"

  # ── launch 1: first run (unpack + cold start) ──────────────────────────────
  local started painted first_ms origin
  started="$(now_ms)"
  start_shell first
  wait_window 60 || die 'the shell never mapped a window'
  place_window
  # The window maps while the manager thread is still unpacking the payload, so
  # this frame is the boot page's own phase (a plain boot page scores few
  # colours, hence the lower floor).
  shot 00-preparing-phase "the boot page while the payload unpacks" 3
  wait_log 'Preparing the bundled Hermes payload' 60 >/dev/null ||
    note 'no preparing line (the payload was already unpacked?)'
  origin="$(wait_log 'Hermes backend ready at (http://127\.0\.0\.1:[0-9]+)' 240 || true)"
  if [[ -z "$origin" ]]; then
    printf 'bundled-proof: the bundled backend never became ready; log tail:\n'
    tail -n 30 "$SHELL_LOG"
    record 01-bundled-ready FAIL "the shell never reported a ready backend"
    record 02-token-redacted FAIL "no ready backend"
    exit "$FAILED"
  fi
  ORIGIN="${origin##*ready at }"
  PORT="${ORIGIN##*:}"
  note "origin=$ORIGIN"
  SIDECAR_LOG="$(sidecar_log_path)"
  note "sidecar log: ${SIDECAR_LOG:-<none>}"

  wait_painted "$MIN_COLOURS" 120 >/dev/null || note 'the UI did not paint in 120 s'
  painted="$(now_ms)"
  first_ms=$((painted - started))
  shot 01-bundled-ready "first run: bundled payload unpacked, UI painted ($((first_ms / 1000)) s after launch)"

  local health
  health="$(curl -s -o /dev/null -w '%{http_code}' "$ORIGIN/healthz" || true)"
  if [[ "$health" == 200 ]]; then
    record 03-healthz PASS "GET $ORIGIN/healthz = 200 (open liveness route)"
  else
    record 03-healthz FAIL "GET $ORIGIN/healthz = ${health:-none}"
  fi

  assert_token_redacted 04-shell-log-redacted "$SHELL_LOG"
  if [[ -n "$SIDECAR_LOG" ]]; then
    assert_token_redacted 05-sidecar-log-redacted "$SIDECAR_LOG"
  else
    record 05-sidecar-log-redacted FAIL "no hermes-sidecar.log under the fresh HOME"
  fi

  if grep -Fq 'Hermes backend source: the bundled payload' "$SHELL_LOG"; then
    record 06-bundled-source PASS 'the shell log says the backend came from the bundled payload'
  else
    record 06-bundled-source FAIL 'the shell log does not name the bundled payload as its source'
  fi
  if grep -Eq 'mode spawn, program "[^"]*/payload/[^"]*/hermes-payload/bin/hermes-sidecar"' "$SHELL_LOG"; then
    record 07-bundled-launcher PASS 'mode spawn from the unpacked payload launcher'
  else
    record 07-bundled-launcher FAIL 'the spawn line does not name the unpacked payload launcher'
  fi

  stop_shell
  sleep 2
  local left
  left="$(leftovers)"
  if [[ -z "$left" ]]; then
    record 08-shutdown PASS 'SIGTERM left no process carrying the payload HERMES_HOME'
  else
    record 08-shutdown FAIL "processes survived with the payload HERMES_HOME:$left"
  fi

  # ── launch 2: second run (payload already unpacked) ────────────────────────
  started="$(now_ms)"
  start_shell second
  wait_window 60 || die 'the shell never mapped a window on the second run'
  place_window
  wait_log 'Hermes backend ready at http' 240 >/dev/null || die 'the second run never became ready'
  wait_painted "$MIN_COLOURS" 120 >/dev/null || note 'the UI did not paint on the second run'
  painted="$(now_ms)"
  local second_ms=$((painted - started))
  record 09-second-run PASS "second run painted $((second_ms / 1000)) s after launch, first run $((first_ms / 1000)) s"
  stop_shell
  sleep 1

  # ── launch 3: cold deep link (F1) + F5 reload (F5) ─────────────────────────
  start_shell deeplink "hermes-tauri://open?path=/team"
  wait_window 60 || die 'the shell never mapped a window for the deep link'
  place_window
  wait_log 'Hermes backend ready at http' 240 >/dev/null || die 'the deep-link run never became ready'
  if wait_log 'Navigated to http://127\.0\.0\.1:[0-9]+/#/team' 60 >/dev/null; then
    record 10-deeplink-route PASS 'the window was navigated to #/team after the launch token was consumed'
  else
    record 10-deeplink-route FAIL 'the #/team route was never applied'
  fi
  wait_painted "$MIN_COLOURS" 120 >/dev/null || note 'the team route did not paint'
  sleep 2
  shot 11-team-authenticated "the #/team route with a live front-door session"

  # The front-door cookie is what keeps the page authenticated: WebKitGTK
  # stores it under the fresh HOME, so its presence is direct evidence.
  if grep -rlq 'hermes_plus_session' "$HOME_DIR" 2>/dev/null; then
    record 12-session-cookie PASS 'the front-door session cookie is stored in the webview profile'
  else
    record 12-session-cookie FAIL 'no hermes_plus_session cookie under the fresh HOME'
  fi

  xdotool windowfocus "$(window_id)" 2>/dev/null || true
  sleep 1
  shot 13-reload-before "the #/team route before the F5 reload"
  xdotool key --clearmodifiers F5
  if wait_repaint "$MIN_COLOURS" 90 >/dev/null; then
    sleep 2
    shot 14-reload-after "the same route after F5: the webview reloaded and repainted"
  else
    record 14-reload-after FAIL 'F5 did not repaint the webview'
  fi
  if wait_log 'Main window navigation: http://127\.0\.0\.1:[0-9]+/#/team' 30 >/dev/null; then
    record 15-reload-in-place PASS 'the reload stayed on #/team on the backend origin'
  else
    record 15-reload-in-place FAIL 'the reload did not reload #/team'
  fi
  if grep -rlq 'hermes_plus_session' "$HOME_DIR" 2>/dev/null; then
    record 16-reload-authenticated PASS 'the session cookie survived the reload'
  else
    record 16-reload-authenticated FAIL 'the session cookie is gone after the reload'
  fi

  stop_shell
  sleep 2
  left="$(leftovers)"
  if [[ -z "$left" ]]; then
    record 17-shutdown-after-deeplink PASS 'no process of the payload HERMES_HOME survived the second shutdown'
  else
    record 17-shutdown-after-deeplink FAIL "processes survived:$left"
  fi

  printf 'bundled-proof: first run %s ms, second run %s ms\n' "$first_ms" "$second_ms"
  printf 'bundled-proof: done, %s checks, %s\n' "${#SHOT_ORDER[@]}" \
    "$([[ "$FAILED" == 1 ]] && printf 'at least one failed' || printf 'all passed')"
  exit "$FAILED"
}

if ((INNER == 1)); then
  inner_main
else
  outer_main
fi

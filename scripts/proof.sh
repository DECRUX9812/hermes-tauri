#!/usr/bin/env bash
# Reproducible Xvfb proof of the Tauri shell: one script, the same shots every run.
#
# Usage: scripts/proof.sh [--bin PATH] [--out DIR] [--port-dir DIR] [--spawn-shutdown]
#
# Outer part: starts an isolated test Hermes on a free 18800-18899 port, then
# re-executes this same file inside `dbus-run-session -- xvfb-run -a`. A trap
# stops the test Hermes and every shell this run launched on each exit path.
#
# Inner part: launches the shell binary, drives it with real input only
# (xdotool keys and clicks, the shell's own deep links) and writes one PNG per
# shot. Every shot is verified (painted frame, checksum seen for the first
# time) and reported as a single PASS or FAIL line; the run exits non-zero when
# any shot failed.
#
# The shell's own XDG dirs are redirected under the output dir and it runs from
# a scratch cwd, so each run starts from a fresh webview profile, nothing is
# written to the real home, and no local path or git branch leaks into a shot.
set -uo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TEST_HERMES="$SCRIPT_DIR/test-hermes.sh"

DEFAULT_BIN="$HOME/.cache/hermes-tauri/target/release/hermes-tauri"
DEFAULT_OUT=/tmp/hermes-tauri-t6g/shots
DEFAULT_PORT_DIR=/tmp/hermes-tauri-test-proof
WORK=/tmp/hermes-tauri-proof-work

SCREEN="1440x900x24"
WINDOW_TITLE="Hermes Tauri"
WINDOW_SIZE="1280x820"
BOTS_TAB="190,48"                  # sidebar Sessions|Bots toggle, window at 0,0
PARK="600,300"                     # neutral pointer parking spot
DEEPLINK_SHOT_ROUTE="/studio"      # 08-deeplink: a route no earlier shot used
MIN_COLOURS=12                     # painted app pages score well above this
# The /studio shot gets its own bar: a blank studio (the 3D scene never built)
# still shows the app chrome plus the "building the studio…" text and scored
# 10-12 on the sample grid (T16's blank run: 19), while the rendered 3D scene
# scored 199-215. 100 sits far above any chrome-and-loading-state frame, so a
# studio that never built can no longer pass. See docs/platforms.md §Linux.
STUDIO_MIN_COLOURS=100

BIN="$DEFAULT_BIN"
OUT="$DEFAULT_OUT"
PORT_DIR="$DEFAULT_PORT_DIR"
INNER=0
SPAWN_MODE=0

usage() {
  cat >&2 <<'USAGE'
usage: scripts/proof.sh [--bin PATH] [--out DIR] [--port-dir DIR] [--spawn-shutdown]

  --bin PATH        shell binary to drive (default: the shared release build)
  --out DIR         screenshot output dir (default: /tmp/hermes-tauri-t6p/shots)
  --port-dir DIR    test Hermes state dir; must live under /tmp/hermes-tauri-test*
  --spawn-shutdown  spawn-mode check: let the shell start its own Hermes, then
                    SIGTERM it and prove no spawned backend survived. Skips the
                    screenshot list.
USAGE
  exit 2
}

abspath() {
  local path="$1"
  if [[ "$path" == /* ]]; then
    printf '%s\n' "$path"
  else
    printf '%s/%s\n' "$PWD" "$path"
  fi
}

while (($# > 0)); do
  case "$1" in
    --bin) BIN="${2:-}"; shift 2 ;;
    --bin=*) BIN="${1#*=}"; shift ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --out=*) OUT="${1#*=}"; shift ;;
    --port-dir) PORT_DIR="${2:-}"; shift 2 ;;
    --port-dir=*) PORT_DIR="${1#*=}"; shift ;;
    --spawn-shutdown) SPAWN_MODE=1; shift ;;
    --inner) INNER=1; shift ;;
    -h|--help) usage ;;
    *) printf 'proof: unknown argument %s\n' "$1" >&2; usage ;;
  esac
done
BIN="$(abspath "$BIN")"
OUT="$(abspath "$OUT")"
PORT_DIR="$(abspath "$PORT_DIR")"
SPAWN_DIR="$PORT_DIR-spawn"

# The proof only ever talks to a disposable test Hermes, never to the real one.
case "$PORT_DIR" in
  /tmp/hermes-tauri-test*) ;;
  *)
    printf 'proof: --port-dir must live under /tmp/hermes-tauri-test*, refusing %s\n' \
      "$PORT_DIR" >&2
    exit 2
    ;;
esac

die() {
  printf 'proof: %s\n' "$*"
  exit 1
}

# ── outer part ───────────────────────────────────────────────────────────────

OUTER_TEST_HERMES=0
OUTER_DBUS_LOG=""

# shellcheck disable=SC2329  # runs from the EXIT/INT/TERM trap below
outer_cleanup() {
  local status=$? pidfile pid
  if [[ "$OUTER_TEST_HERMES" == 1 ]]; then
    HERMES_TAURI_TEST_DIR="$PORT_DIR" "$TEST_HERMES" stop >/dev/null 2>&1 || true
  fi
  for pidfile in "$OUT"/shell-*.pid; do
    [[ -f "$pidfile" ]] || continue
    IFS= read -r pid <"$pidfile" || continue
    [[ "$pid" =~ ^[1-9][0-9]*$ ]] || continue
    kill -TERM -- "-$pid" 2>/dev/null || true
  done
  rm -f "$OUT"/shell-*.pid 2>/dev/null || true
  exit "$status"
}

outer_main() {
  [[ -x "$TEST_HERMES" ]] || die "missing $TEST_HERMES"
  [[ -x "$BIN" ]] || die "not executable: $BIN (pass --bin)"
  command -v dbus-run-session >/dev/null 2>&1 || die 'dbus-run-session is required'
  command -v xvfb-run >/dev/null 2>&1 || die 'xvfb-run is required'
  mkdir -p "$OUT" || die "cannot create $OUT"
  if ! rm -rf "$WORK" || ! mkdir -p "$WORK"; then
    die "cannot create $WORK"
  fi
  write_helpers
  # Everything below runs from this scratch dir: neither the test Hermes backend
  # nor the shell can pick up a repository path or a git branch name from here,
  # and the backend is what reports the session's project to the UI.
  cd "$WORK" || die "cannot enter $WORK"

  if [[ "$SPAWN_MODE" == 1 ]]; then
    # The shell starts its own Hermes from HERMES_TAURI_CMD; give it a second,
    # seeded state dir so this mode never disturbs the attach run.
    printf 'proof: spawn mode, the shell starts its own Hermes under %s\n' "$SPAWN_DIR"
    HERMES_TAURI_TEST_DIR="$SPAWN_DIR" "$TEST_HERMES" seed >/dev/null || \
      die 'could not seed the spawn state dir'
  else
    printf 'proof: starting the isolated test Hermes under %s\n' "$PORT_DIR"
    HERMES_TAURI_TEST_DIR="$PORT_DIR" "$TEST_HERMES" start >/dev/null || \
      die "test Hermes did not start (state dir $PORT_DIR)"
    OUTER_TEST_HERMES=1
  fi
  trap outer_cleanup EXIT INT TERM

  local inner_args=(--inner --bin "$BIN" --out "$OUT" --port-dir "$PORT_DIR") status
  [[ "$SPAWN_MODE" == 1 ]] && inner_args+=(--spawn-shutdown)
  # Session bus and xdg-portal chatter would drown the run log, so it goes to
  # its own file next to the shots.
  OUTER_DBUS_LOG="$OUT/dbus-stderr.log"
  printf 'proof: entering dbus-run-session + xvfb-run (%s)\n' "$SCREEN"
  dbus-run-session -- xvfb-run -a -s "-screen 0 $SCREEN" \
    "$SCRIPT_DIR/proof.sh" "${inner_args[@]}" 2>"$OUTER_DBUS_LOG"
  status=$?
  if ((status != 0)); then
    printf 'proof: inner run exited %s. Session bus and portal stderr is in %s; its last lines:\n' \
      "$status" "$OUTER_DBUS_LOG"
    grep -v '^[[:space:]]*$' "$OUTER_DBUS_LOG" | tail -n 15
  fi
  exit "$status"
}

# ── inner part ───────────────────────────────────────────────────────────────

SHELL_PID=0
SHELL_LOG=""
SHELL_LOG_LINES=0
STATE_DIR=""
SHOT_FAILED=0
reload_mark=0
declare -a SHOT_ORDER=() SHOT_RESULT=() SHOT_COLOURS=() SHOT_MD5=() SHOT_DESC=()
declare -a CHECK_ORDER=() CHECK_RESULT=() CHECK_DESC=()

# shellcheck disable=SC2329  # runs from the EXIT/INT/TERM trap at the end of the file
inner_cleanup() {
  local status=$?
  stop_black_hole
  # The retry check starts a second test Hermes; stop it by its own pid file.
  # `stop` is a no-op when that instance never started.
  HERMES_TAURI_TEST_DIR="$PORT_DIR-retry" "$TEST_HERMES" stop >/dev/null 2>&1 || true
  stop_shell
  rm -f "$OUT"/shell-*.pid 2>/dev/null || true
  # A run that dies early still leaves a truthful index of the shots it wrote.
  if ((${#SHOT_ORDER[@]} > 0)) && [[ -n "$STATE_DIR" ]]; then
    write_readme
  fi
  exit "$status"
}

shell_pgid() {
  ps -o pgid= -p "$SHELL_PID" 2>/dev/null | tr -d '[:space:]'
}

stop_shell() {
  local pgid attempt
  [[ "$SHELL_PID" =~ ^[1-9][0-9]*$ ]] || return 0
  if kill -0 "$SHELL_PID" 2>/dev/null; then
    pgid="$(shell_pgid)"
    [[ -n "$pgid" ]] && kill -TERM -- "-$pgid" 2>/dev/null
    for attempt in $(seq 1 40); do
      kill -0 "$SHELL_PID" 2>/dev/null || break
      sleep 0.25
    done
    if kill -0 "$SHELL_PID" 2>/dev/null && [[ -n "$pgid" ]]; then
      kill -KILL -- "-$pgid" 2>/dev/null
      sleep 0.5
    fi
  fi
  SHELL_PID=0
}

# Starts a shell instance and fails loudly when it does not survive the start.
start_shell() {
  local label="$1"
  shift
  stop_shell
  SHELL_LOG="$OUT/shell-$label.log"
  : >"$SHELL_LOG"
  # The log is truncated here, so every line in it belongs to this instance and
  # wait_log can start scanning from the top.
  SHELL_LOG_LINES=0
  setsid "$BIN" "$@" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!
  printf '%s\n' "$SHELL_PID" >"$OUT/shell-$label.pid"
  sleep 2
  if ! kill -0 "$SHELL_PID" 2>/dev/null; then
    printf 'proof: the shell (%s) died within 2 s of launch; log tail:\n' "$label"
    tail -n 20 "$SHELL_LOG"
    exit 1
  fi
  printf 'proof: shell %s pid=%s pgid=%s log=%s\n' \
    "$label" "$SHELL_PID" "$(shell_pgid)" "$SHELL_LOG"
}

window_id() {
  xdotool search --name "$WINDOW_TITLE" 2>/dev/null | head -n 1
}

window_count() {
  xdotool search --name "$WINDOW_TITLE" 2>/dev/null | wc -l
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
  local wid placed=0
  # The splash and the main window share the title, so every match is placed.
  while read -r wid; do
    xdotool windowmove "$wid" 0 0 2>/dev/null || true
    xdotool windowsize "$wid" "${WINDOW_SIZE%x*}" "${WINDOW_SIZE#*x}" 2>/dev/null || true
    placed=1
  done < <(xdotool search --name "$WINDOW_TITLE" 2>/dev/null)
  ((placed == 1)) || return 1
  sleep 0.5
}

# 01b-splash: the boot page stays in front of the window while the remote app
# loads. Taken after the launch navigation and before the app's first frame,
# and only while the splash window is still mapped, so a PASS here means the
# user really saw the boot page and not a blank window.
#
# The splash is a second webview, so its own boot page needs a moment to paint
# its first frame; under load that moment is seconds, and a shot taken before it
# is a picture of a white window. Sample until the boot page is really on
# screen, and re-check after each capture that the splash is still the window in
# front and the app has not painted: the app's first frame dismisses the splash,
# so either signal means this shot's moment has passed.
splash_shot() {
  local windows deadline
  if ! wait_log 'Main window navigation' 45 >/dev/null; then
    record 01b-splash FAIL "-" "-" "the launch navigation was never logged"
    return 1
  fi
  deadline=$((SECONDS + 45))
  while :; do
    if grep -q 'First frame of the Hermes app painted' "$SHELL_LOG"; then
      record 01b-splash FAIL "-" "-" "the app painted before the splash could be captured"
      return 1
    fi
    windows="$(window_count)"
    if ((windows != 2)); then
      record 01b-splash FAIL "-" "-" \
        "$windows window(s) carry the app title, so the splash was already gone"
      return 1
    fi
    if capture 01b-splash; then
      if ((CAP_COLOURS >= 4)) && ! capture_repeats &&
        ! grep -q 'First frame of the Hermes app painted' "$SHELL_LOG"; then
        record 01b-splash PASS "$CAP_COLOURS" "$CAP_MD5" \
          "the splash (the boot page) still in front of the window while the Hermes app loads, before its first frame"
        return 0
      fi
    fi
    if ((SECONDS >= deadline)); then
      record 01b-splash FAIL "${CAP_COLOURS:--}" "${CAP_MD5:--}" \
        "blank frame: ${CAP_COLOURS:-unreadable} distinct colours on the sample grid, under 4"
      return 1
    fi
    sleep 0.5
  done
}

# Fast probe of what is on screen (xwd, ~0.2 s), used only to decide when to
# take a shot. The shots themselves are verified from the PNG scrot writes.
probe_colours() {
  xwd -root -silent >"$WORK/probe.xwd" 2>/dev/null || return 1
  python3 "$WORK/frame_colours.py" "$WORK/probe.xwd" 2>/dev/null | tr -dc '0-9'
}

# Painted means the sample grid holds real content: a blank window scores 1 and
# the shell's own boot page (the splash) scores 9 and up.
wait_painted() {
  local min="$1" timeout="${2:-45}" deadline=$((SECONDS + ${2:-45})) colours="" hits=0
  while ((SECONDS < deadline)); do
    colours="$(probe_colours)"
    if [[ -n "$colours" ]] && ((colours >= min)); then
      hits=$((hits + 1))
      if ((hits >= 2)); then
        return 0
      fi
      sleep 1.5
      continue
    fi
    hits=0
    sleep 0.3
  done
  printf 'proof: the frame stayed below %s colours (last: %s) after %ss\n' \
    "$min" "${colours:-unreadable}" "$timeout"
  return 1
}

# After a navigation the webview loads the page again, so wait for the loading
# state and then for two consecutive painted frames. When no loading state ever
# shows up, the route changed in place and a painted frame this late is fine.
wait_repaint() {
  local min="$1" timeout="${2:-45}" grace="${3:-12}"
  local start=$SECONDS deadline=$((SECONDS + ${2:-45})) dipped=0 hits=0 colours=""
  while ((SECONDS < deadline)); do
    colours="$(probe_colours)"
    if [[ -n "$colours" ]] && ((colours >= min)); then
      if ((dipped == 1)); then
        hits=$((hits + 1))
        if ((hits >= 2)); then
          return 0
        fi
        sleep 1.5
        continue
      fi
      if ((SECONDS - start >= grace)); then
        return 0
      fi
    elif [[ -n "$colours" ]]; then
      dipped=1
      hits=0
    fi
    sleep 0.3
  done
  printf 'proof: the frame never came back painted (last: %s) within %ss\n' \
    "${colours:-unreadable}" "$timeout"
  return 1
}

# Waits for a new line matching the pattern in the shell's own stderr log, so
# waits are driven by what the shell really did rather than by a fixed sleep.
# The marker advances to the matched line only: lines the shell wrote after it
# (the app paints while this function is returning) belong to the next wait, or
# a later wait would miss them.
wait_log() {
  local pattern="$1" timeout="${2:-25}" deadline=$((SECONDS + ${2:-25})) line
  local -a lines=()
  local consumed=0 index
  while ((SECONDS < deadline)); do
    mapfile -t lines < <(tail -n +"$((SHELL_LOG_LINES + 1))" "$SHELL_LOG" 2>/dev/null)
    consumed=$SHELL_LOG_LINES
    for ((index = 0; index < ${#lines[@]}; index++)); do
      line="${lines[$index]}"
      consumed=$((consumed + 1))
      if [[ "$line" =~ $pattern ]]; then
        SHELL_LOG_LINES=$consumed
        printf '%s\n' "$line"
        return 0
      fi
    done
    SHELL_LOG_LINES=$consumed
    if ! kill -0 "$SHELL_PID" 2>/dev/null; then
      printf 'proof: the shell died while waiting for /%s/\n' "$pattern"
      tail -n 20 "$SHELL_LOG"
      exit 1
    fi
    sleep 0.4
  done
  printf 'proof: /%s/ was not logged within %ss\n' "$pattern" "$timeout"
  return 1
}

# The shell's own deep link: a second invocation hands the route to the first
# window through the single-instance plugin.
deeplink() {
  local route="$1" pattern="$2" status windows
  ( cd /tmp && timeout 20 "$BIN" "hermes-tauri://open?path=$route" \
      >"$WORK/deeplink.log" 2>&1 )
  status=$?
  if ((status != 0)); then
    printf 'proof: the second invocation for %s exited %s\n' "$route" "$status"
    return 1
  fi
  windows="$(window_count)"
  if ((windows != 1)); then
    printf 'proof: %s windows after the second invocation, so single-instance did not hand over\n' \
      "$windows"
    return 1
  fi
  wait_log "$pattern" 25
}

record() {
  local name="$1" result="$2" colours="$3" md5="$4" desc="$5"
  SHOT_ORDER+=("$name")
  SHOT_RESULT+=("$result")
  SHOT_COLOURS+=("$colours")
  SHOT_MD5+=("$md5")
  SHOT_DESC+=("$desc")
  if [[ "$result" == PASS ]]; then
    printf 'PASS %s\n' "$name"
  else
    printf 'FAIL %s: %s\n' "$name" "$desc"
    SHOT_FAILED=1
  fi
}

CAP_COLOURS=""
CAP_MD5=""
# A check is a proof that reads the shell's own log and the window list instead
# of a screenshot: it has no frame to show, but it must still be reported.
record_check() {
  local name="$1" result="$2" desc="$3"
  CHECK_ORDER+=("$name")
  CHECK_RESULT+=("$result")
  CHECK_DESC+=("$desc")
  if [[ "$result" == PASS ]]; then
    printf 'PASS %s\n' "$name"
  else
    printf 'FAIL %s: %s\n' "$name" "$desc"
    SHOT_FAILED=1
  fi
}

capture() {
  local name="$1"
  local file="$OUT/$name.png"
  CAP_COLOURS=""
  CAP_MD5=""
  scrot -o "$file" || return 1
  CAP_COLOURS="$(python3 "$WORK/png_colours.py" "$file" 2>/dev/null | tr -dc '0-9')"
  [[ -n "$CAP_COLOURS" ]] || return 1
  CAP_MD5="$(md5sum "$file" | cut -d' ' -f1)"
  return 0
}

# True when the frame just captured is byte-identical to an earlier shot: the
# screen never changed, so the shot would prove nothing. Sets CAP_REPEAT to the
# name of the shot it repeats.
capture_repeats() {
  local index
  CAP_REPEAT=""
  for index in "${!SHOT_MD5[@]}"; do
    if [[ "${SHOT_MD5[$index]}" == "$CAP_MD5" ]]; then
      CAP_REPEAT="${SHOT_ORDER[$index]}"
      return 0
    fi
  done
  return 1
}

# Captures one shot, checks the frame is painted and not a copy of an earlier
# shot, and prints its PASS or FAIL line. `timeout` is how long the frame is
# given to reach `min` colours: a route that paints a loading state first (the
# Studio page prints "building the studio…" for a while) passes through a frame
# below the threshold, and the shot must be of the page, not of that state. The
# last frame is kept when the timeout runs out, so the FAIL still shows what was
# on screen.
shot() {
  local name="$1" min="$2" desc="$3" timeout="${4:-0}"
  local deadline=$((SECONDS + timeout))
  while :; do
    if ! capture "$name"; then
      record "$name" FAIL "-" "-" "the screen could not be captured and read back"
      return 1
    fi
    if ((CAP_COLOURS >= min)) || ((SECONDS >= deadline)); then
      break
    fi
    sleep 1
  done
  if ((CAP_COLOURS < min)); then
    record "$name" FAIL "$CAP_COLOURS" "$CAP_MD5" \
      "blank frame: $CAP_COLOURS distinct colours on the sample grid, under $min"
    return 1
  fi
  if capture_repeats; then
    record "$name" FAIL "$CAP_COLOURS" "$CAP_MD5" \
      "identical to $CAP_REPEAT.png: the screen never changed"
    return 1
  fi
  record "$name" PASS "$CAP_COLOURS" "$CAP_MD5" "$desc"
}

# True when the app's own webview state mentions the given text.
webview_state_mentions() {
  grep -rla -- "$1" "$STATE_DIR" >/dev/null 2>&1
}

inner_main() {
  printf 'proof: DISPLAY=%s shell=%s\n' "${DISPLAY:-<unset>}" "$BIN"
  command -v xdpyinfo >/dev/null 2>&1 || die 'xdpyinfo is required'
  if ! xdpyinfo >/dev/null 2>&1; then
    printf 'proof: no usable X display (DISPLAY=%s)\n' "${DISPLAY:-<unset>}"
    exit 2
  fi
  if [[ ! -x "$BIN" ]]; then
    printf 'proof: not executable: %s\n' "$BIN"
    exit 3
  fi
  command -v scrot >/dev/null 2>&1 || die 'scrot is required (import from ImageMagick is not installed here)'
  command -v xdotool >/dev/null 2>&1 || die 'xdotool is required'
  command -v xwd >/dev/null 2>&1 || die 'xwd is required'

  mkdir -p "$OUT" || die "cannot create $OUT"
  rm -f "$OUT"/*.png "$OUT"/shell-*.log "$OUT"/shell-*.pid 2>/dev/null
  local probe="$OUT/.writable"
  : >"$probe" || die "cannot write in $OUT"
  rm -f "$probe"
  write_helpers

  STATE_DIR="$OUT/shell-state"
  rm -rf "$STATE_DIR"
  mkdir -p "$STATE_DIR"
  export XDG_DATA_HOME="$STATE_DIR/data" XDG_CONFIG_HOME="$STATE_DIR/config"
  export XDG_CACHE_HOME="$STATE_DIR/cache" XDG_STATE_HOME="$STATE_DIR/state"
  export NO_AT_BRIDGE=1

  if [[ "$SPAWN_MODE" == 1 ]]; then
    spawn_shutdown_check
    write_readme
    exit "$SHOT_FAILED"
  fi

  local origin dead_port
  origin="$(HERMES_TAURI_TEST_DIR="$PORT_DIR" "$TEST_HERMES" status | awk '{print $2}')"
  [[ "$origin" == http://127.0.0.1:* ]] || die "the test Hermes is not running ($origin)"
  [[ -r "$PORT_DIR/launch-url" ]] || die "the launch link is missing in $PORT_DIR"
  dead_port="$(free_port "${origin##*:}")" || die 'no free port for the boot shot'
  printf 'proof: test Hermes at %s, closed port for the boot shot: %s\n' "$origin" "$dead_port"

  # 01-boot: point the shell at a port where nothing listens, so it stays on its
  # boot page and reports Hermes as unreachable.
  start_shell boot --url "http://127.0.0.1:$dead_port"
  wait_window 30 || die 'the shell never mapped a window'
  place_window
  if wait_log 'Hermes connection failed' 40 >/dev/null && wait_painted 4 20; then
    sleep 2
    shot 01-boot 4 \
      "boot page while Hermes is unreachable: launched against the closed port $dead_port, the page reports it and offers Retry"
  else
    record 01-boot FAIL "-" "-" "the shell never painted its boot page"
  fi
  stop_shell

  # 02-sessions: attach to the test Hermes and land on the home route.
  start_shell attach --url "$(cat "$PORT_DIR/launch-url")"
  wait_window 30 || die 'the shell never mapped a window'
  place_window
  wait_log 'Hermes backend ready at' 45 >/dev/null || die 'the shell never connected to the test Hermes'
  splash_shot
  wait_log 'First frame of the Hermes app painted' 60 >/dev/null || die 'the app never painted its first frame'
  local windows
  windows="$(window_count)"
  if ((windows != 1)); then
    record 02-sessions FAIL "-" "-" \
      "the app painted its first frame but $windows windows carry the app title: the always-on-top splash is still up"
  elif wait_painted "$MIN_COLOURS" 60; then
    shot 02-sessions "$MIN_COLOURS" \
      "home route after attaching: the seeded demo session titles in the sidebar and the seeded recent tasks on the page" 20
  else
    record 02-sessions FAIL "-" "-" "the app never painted after attaching"
  fi

  # 03-plugin-rail and 04-team: plugin pages, navigated by the shell's own deep
  # links, each waiting for the page to reload before the shot is taken.
  nav_shot 03-plugin-rail '%2Fkanban' 'Navigated to .*#/kanban' "$MIN_COLOURS" \
    "the Kanban plugin page reached by deep link, the plugin rail on the far left"
  nav_shot 04-team '%2Fteam' 'Navigated to .*#/team' "$MIN_COLOURS" \
    "the Demo Product Team page reached by deep link"

  group_chat_shot

  # 06/07: in-place reload with the shell's own F5 accelerator (the adapter
  # intercepts it and calls location.reload()). 06 shows the group chat view the
  # UI is on, which is in-app state, so the window's URL is still the #/team
  # route; 07 shows what a reload really restores, the route and its data.
  xdotool windowfocus "$(window_id)" 2>/dev/null || true
  xdotool mousemove --sync "${PARK%,*}" "${PARK#*,}" 2>/dev/null || true
  sleep 1
  shot 06-reload-before "$MIN_COLOURS" \
    "the group chat view (in-app state on the #/team route) before the reload" 20
  xdotool windowfocus "$(window_id)" 2>/dev/null || true
  reload_mark="$(wc -l <"$SHELL_LOG")"
  xdotool key --clearmodifiers F5
  # A reload must show the webview's loading state before it paints again, so
  # this wait never accepts a painted frame that never left the screen.
  if wait_repaint "$MIN_COLOURS" 45 100000; then
    shot 07-reload-after "$MIN_COLOURS" \
      "the #/team route after F5: the webview reloaded, repainted from the backend and kept the session" 20
    reload_session_check "$reload_mark"
  else
    record 07-reload-after FAIL "-" "-" \
      "F5 did not reload the webview: the frame never left its painted state and came back"
  fi

  # 08-deeplink: a second invocation hands another route to the running window.
  # The route is /studio, and its bar is STUDIO_MIN_COLOURS, not MIN_COLOURS:
  # only the built 3D scene reaches that count, so a studio stuck on its
  # loading state fails the proof instead of slipping through as painted.
  nav_shot 08-deeplink "$DEEPLINK_SHOT_ROUTE" "Navigated to .*$DEEPLINK_SHOT_ROUTE" \
    "$STUDIO_MIN_COLOURS" \
    "the same window after a second invocation handed it the ${DEEPLINK_SHOT_ROUTE} route, \
3D scene rendered"

  stop_shell
  # The retry check needs its own port: a free one in the test range that the
  # test Hermes is not on, because the first half of the check must find it
  # closed.
  local retry_port
  retry_port="$(free_port "${origin##*:}")" || die 'no free port for the retry check'
  retry_check "$retry_port"
  cap_check
  stop_shell
  write_readme
  printf 'proof: done, %s shots and %s checks, %s\n' "${#SHOT_ORDER[@]}" \
    "${#CHECK_ORDER[@]}" \
    "$([[ "$SHOT_FAILED" == 1 ]] && printf 'at least one failed' || printf 'all passed')"
  exit "$SHOT_FAILED"
}

# The F5 proof: the webview must really re-navigate to the route it was on, and
# the session must survive it. Read from the lines the shell wrote after the
# keypress (the marker), so a navigation logged before the reload cannot pass
# this. No second window and no connection error: the token was reused, not
# re-minted.
reload_session_check() {
  local mark="$1" windows lines
  windows="$(window_count)"
  lines="$(tail -n +"$((mark + 1))" "$SHELL_LOG" 2>/dev/null)"
  if ((windows != 1)); then
    record_check reload-session FAIL \
      "F5 left $windows windows carrying the app title, so a login or error window appeared"
    return 1
  fi
  if ! grep -q 'Main window navigation: .*#/team' <<<"$lines"; then
    record_check reload-session FAIL \
      "F5 produced no new navigation to the route the window was on"
    return 1
  fi
  if grep -q 'Hermes connection failed' <<<"$lines"; then
    record_check reload-session FAIL "the shell lost the backend across the reload"
    return 1
  fi
  record_check reload-session PASS \
    "F5 re-navigated the webview to the same #/team route, the app painted again from the backend, no second window and no connection error: the session survived the reload"
}

# A deep link followed by a shot: the link must be logged by the running shell
# and the page must come back painted before the screen is captured.
nav_shot() {
  local name="$1" route="$2" pattern="$3" min="$4" desc="$5"
  if ! deeplink "$route" "$pattern" >/dev/null; then
    record "$name" FAIL "-" "-" "the $route deep link was not honoured"
    return 1
  fi
  if ! wait_repaint "$min" 45; then
    record "$name" FAIL "-" "-" "the screen never came back painted after the $route deep link"
    return 1
  fi
  # A plugin page can paint its own loading state first (the Studio page prints
  # "building the studio…" for a while), so the shot is given time to settle.
  shot "$name" "$min" "$desc" 30
}

# 05-group-chat: a group chat created through the UI with demo bots and a neutral
# message submitted to the active room transcript.
group_chat_shot() {
  local wid
  wid="$(window_id)"
  xdotool windowfocus "$wid" 2>/dev/null || true

  # 1. Switch sidebar to Bots tab
  xdotool mousemove --sync 190 48 2>/dev/null || true
  xdotool click 1
  sleep 1.5

  # 2. Click '+' button to open dropdown menu
  xdotool mousemove --sync 259 85 2>/dev/null || true
  xdotool click 1
  sleep 1.5

  # 3. Click "New group chat" in dropdown menu
  xdotool mousemove --sync 180 142 2>/dev/null || true
  xdotool click 1
  sleep 2.0

  # 4. In the dialog, select the first demo bot
  xdotool mousemove --sync 550 474 2>/dev/null || true
  xdotool click 1
  sleep 1.2

  # 5. Select the second demo bot (shifted down by selection badge mounting)
  xdotool mousemove --sync 550 590 2>/dev/null || true
  xdotool click 1
  sleep 1.2

  # 6. Click "Create Group (2)" button
  xdotool mousemove --sync 785 728 2>/dev/null || true
  xdotool click 1
  sleep 3.0

  # 7. Focus composer textarea at (500, 710), clear draft, type and submit
  xdotool mousemove --sync 500 710 2>/dev/null || true
  xdotool click 1
  sleep 0.5
  xdotool key --clearmodifiers ctrl+a
  sleep 0.2
  xdotool key BackSpace
  sleep 0.2
  xdotool type --delay 15 "Demo: please summarize the launch checklist"
  sleep 1.5
  xdotool key Return
  sleep 1.0
  xdotool mousemove --sync 1156 732 2>/dev/null || true
  xdotool click 1
  sleep 3.0

  # 8. Park cursor
  xdotool mousemove --sync "${PARK%,*}" "${PARK#*,}" 2>/dev/null || true
  sleep 1.0

  shot 05-group-chat "$MIN_COLOURS" \
    "a group chat created through the UI with two demo bots and one submitted neutral message" 20
}

# The splash cap (splash.rs, 30 s) is the safety net for an app page that never
# reports a first frame. The app paints in ~6-9 s on this machine, so a real cap
# would only ever be seen as "it did not fire"; this check shortens the cap
# through the shell's own environment knob and watches the same code path: the
# always-on-top window must go away on its own while the app is still loading,
# leaving exactly one window, and the app must then paint as usual.
cap_check() {
  local cap_ms=3000 windows
  printf 'proof: splash cap check, HERMES_TAURI_SPLASH_CAP_MS=%s\n' "$cap_ms"
  export HERMES_TAURI_SPLASH_CAP_MS="$cap_ms"
  start_shell cap --url "$(cat "$PORT_DIR/launch-url")"
  unset HERMES_TAURI_SPLASH_CAP_MS
  wait_window 30 || die 'the shell never mapped a window'
  place_window
  if ! wait_log "Splash removed: the app did not paint within ${cap_ms} ms" 90 >/dev/null; then
    record_check splash-cap FAIL "the cap never removed the splash"
    return 1
  fi
  windows="$(window_count)"
  if ((windows != 1)); then
    record_check splash-cap FAIL \
      "the cap fired but $windows windows still carry the app title"
    return 1
  fi
  if ! wait_log 'First frame of the Hermes app painted' 60 >/dev/null; then
    record_check splash-cap FAIL "the app never painted after the cap removed the splash"
    return 1
  fi
  record_check splash-cap PASS \
    "the cap removed the always-on-top splash while the app was still loading (same code path as the 30 s default, shortened to ${cap_ms} ms); one window was left and the app painted afterwards"
}

# Where the boot page's Retry button is, measured in the frame on screen now.
# boot/boot.css centers a 480 px panel in the window and the button is the last
# element in it, so the button is the lowest bordered box on the page; the
# helper walks the rows for that box ($WORK/retry_button_xy.py) rather than
# trusting a fixed spot, which a longer or shorter error message above the
# button would move.
retry_button_xy() {
  local frame="$WORK/retry-frame.png" xy
  scrot -o "$frame" || return 1
  xy="$(python3 "$WORK/retry_button_xy.py" "$frame" 2>/dev/null)" || return 1
  [[ "$xy" =~ ^[0-9]+\ [0-9]+$ ]] || return 1
  printf '%s\n' "$xy"
}

# A real click on the button: pointer moved onto it, button 1 pressed.
click_retry() {
  local x="$1" y="$2"
  xdotool windowfocus "$(window_id)" 2>/dev/null || true
  xdotool mousemove --sync "$x" "$y" 2>/dev/null || return 1
  sleep 0.3
  xdotool click 1
}

# A listener that accepts connections and never answers. The shell's health
# probe then runs into its 3 s timeout instead of the instant refusal a closed
# port gives, which is what keeps the splash on screen long enough between the
# click and the failure to be observed. Held and stopped by pid.
BLACK_HOLE_PID=0

start_black_hole() {
  local port="$1"
  python3 - "$port" >/dev/null 2>&1 <<'PY' &
import socket
import sys

port = int(sys.argv[1])
server = socket.socket()
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", port))
server.listen(16)
held = []
while True:
    try:
        connection, _ = server.accept()
    except OSError:
        break
    held.append(connection)  # accepted, deliberately never answered
PY
  BLACK_HOLE_PID=$!
  sleep 0.5
  kill -0 "$BLACK_HOLE_PID" 2>/dev/null
}

stop_black_hole() {
  if [[ "$BLACK_HOLE_PID" =~ ^[1-9][0-9]*$ ]]; then
    kill -TERM "$BLACK_HOLE_PID" 2>/dev/null || true
    wait "$BLACK_HOLE_PID" 2>/dev/null || true
  fi
  BLACK_HOLE_PID=0
}

# The retry check (R8 finding 1: the earlier proof only ever photographed the
# failure page). A real click on the boot page's Retry must start a new attempt,
# put the always-on-top splash back up for it and lose it again when the attempt
# fails; a second click, with the test Hermes started on that same port, must
# end with the app on screen.
#
# The shell runs from its config.toml here, which is the one source an attempt
# re-reads from scratch: it first names the port where nothing listens (the
# 01-boot state), and the second click happens after the file was pointed at the
# launch link of the test Hermes that was started on that port (the link carries
# the session token, which a bare `--url` retry would not have).
retry_check() {
  local port="$1"
  local config_dir="$STATE_DIR/config/dev.hermes-plus.desktop"
  local config_file="$config_dir/config.toml"
  local retry_dir="$PORT_DIR-retry"
  local mark windows max_windows deadline launch_url xy attempts lines

  printf 'proof: retry check, nothing listening on %s yet\n' "$port"
  mkdir -p "$config_dir" || die "cannot create $config_dir"

  # 1. Start against the closed port: the same state as 01-boot.
  {
    printf '# Written by scripts/proof.sh for the retry check.\n'
    printf 'url = "http://127.0.0.1:%s"\n' "$port"
    printf 'spawn_if_unreachable = false\n'
  } >"$config_file"
  start_shell retry
  wait_window 30 || die 'the shell never mapped a window'
  place_window
  if ! wait_log 'Hermes connection failed' 40 >/dev/null || ! wait_painted 4 20; then
    record_check retry FAIL "the shell never showed the boot page with Retry"
    stop_shell
    return 1
  fi
  sleep 2
  if ! xy="$(retry_button_xy)"; then
    record_check retry FAIL "the Retry button could not be located in the boot page's frame"
    stop_shell
    return 1
  fi
  printf 'proof: Retry button at %s\n' "$xy"

  # 2. First click: the port accepts but never answers, so the next attempt
  #    keeps its splash up for the probe's 3 s timeout.
  if ! start_black_hole "$port"; then
    record_check retry FAIL "could not hold port $port with a silent listener"
    stop_shell
    return 1
  fi
  mark="$(wc -l <"$SHELL_LOG")"
  if ! click_retry $xy; then
    stop_black_hole
    record_check retry FAIL "the Retry click could not be delivered"
    stop_shell
    return 1
  fi

  # 3. Sample the window list while that attempt runs: the splash comes back
  #    (two windows carry the app title) and goes away again on the failure
  #    (one window). The loop runs until the shell logged that dismissal, so
  #    the counts below are read while and after the attempt really happened.
  max_windows=0
  attempts=0
  deadline=$((SECONDS + 45))
  while ((SECONDS < deadline)); do
    windows="$(window_count)"
    ((windows > max_windows)) && max_windows="$windows"
    lines="$(tail -n +"$((mark + 1))" "$SHELL_LOG" 2>/dev/null)"
    grep -q 'Connecting to Hermes' <<<"$lines" && attempts=1
    if ((attempts == 1)) && grep -q 'Splash removed' <<<"$lines"; then
      break
    fi
    sleep 0.1
  done
  # This loop read the log itself, so the shared marker moves past it.
  SHELL_LOG_LINES="$(wc -l <"$SHELL_LOG")"
  stop_black_hole
  lines="$(tail -n +"$((mark + 1))" "$SHELL_LOG" 2>/dev/null)"
  windows="$(window_count)"

  if ((attempts == 0)); then
    record_check retry FAIL "the Retry click started no new connection attempt"
    stop_shell
    return 1
  fi
  if ((max_windows != 2)); then
    record_check retry FAIL \
      "the retry attempt never had two windows on screen (highest count $max_windows), so the splash did not come back"
    stop_shell
    return 1
  fi
  if ! grep -q 'Splash removed' <<<"$lines"; then
    record_check retry FAIL "the failure did not dismiss the splash it had put back"
    stop_shell
    return 1
  fi
  if ((windows != 1)); then
    record_check retry FAIL \
      "after the failed retry $windows windows carry the app title, expected one"
    stop_shell
    return 1
  fi
  if ! xy="$(retry_button_xy)"; then
    record_check retry FAIL "the Retry button could not be located after the failed retry"
    stop_shell
    return 1
  fi

  # 4. Second click: the backend answers on that port now (the attempt re-reads
  #    the config file, which points at its launch link), so the app must end up
  #    on screen with the splash gone.
  printf 'proof: starting the test Hermes on %s for the retry\n' "$port"
  if ! HERMES_TAURI_TEST_DIR="$retry_dir" "$TEST_HERMES" start "$port" >/dev/null; then
    record_check retry FAIL "the test Hermes did not start on port $port"
    stop_shell
    return 1
  fi
  launch_url="$(cat "$retry_dir/launch-url" 2>/dev/null)"
  if [[ "$launch_url" != "http://127.0.0.1:$port"* ]]; then
    record_check retry FAIL "the test Hermes on $port wrote no usable launch link"
    stop_shell
    return 1
  fi
  {
    printf '# Written by scripts/proof.sh for the retry check.\n'
    printf 'url = "%s"\n' "$launch_url"
    printf 'spawn_if_unreachable = false\n'
  } >"$config_file"
  if ! click_retry $xy; then
    record_check retry FAIL "the second Retry click could not be delivered"
    stop_shell
    return 1
  fi
  if ! wait_log "Hermes backend ready at http://127.0.0.1:$port" 90 >/dev/null; then
    record_check retry FAIL "the retry did not connect to the test Hermes on port $port"
    stop_shell
    return 1
  fi
  if ! wait_log 'First frame of the Hermes app painted' 90 >/dev/null; then
    record_check retry FAIL "the app never painted after the retry connected"
    stop_shell
    return 1
  fi
  windows="$(window_count)"
  if ((windows != 1)); then
    record_check retry FAIL \
      "the app painted but $windows windows carry the app title: the splash stayed up"
    stop_shell
    return 1
  fi
  if ! wait_painted "$MIN_COLOURS" 60; then
    record_check retry FAIL "no painted app frame came back after the retry"
    stop_shell
    return 1
  fi

  record_check retry PASS \
    "a real click on the boot page's Retry started a new attempt (the shell logged it), put the splash back up (two windows carried the app title while the attempt ran) and lost it again when that attempt failed (Splash removed, one window left); a second click, after the test Hermes was started on that port and the shell's config.toml was pointed at its launch link, connected, painted the app and left one window"
  stop_shell
}

# ── spawn mode ───────────────────────────────────────────────────────────────

spawn_shutdown_check() {
  printf 'proof: spawn-shutdown check, state dir %s\n' "$SPAWN_DIR"
  local spawn_json spawn_port
  local -a pids=() pgids=()
  spawn_json="$(HERMES_TAURI_TEST_DIR="$SPAWN_DIR" "$TEST_HERMES" cmd)" || die 'could not build the spawn command'
  python3 -c 'import json, sys; arr = json.loads(sys.argv[1]); sys.exit(0 if isinstance(arr, list) and all(isinstance(x, str) for x in arr) else 1)' "$spawn_json" 2>/dev/null || \
    die 'the spawn command is not a JSON list of strings'
  spawn_port="$(python3 -c 'import json, sys; arr = json.loads(sys.argv[1]); i = arr.index("--port"); print(arr[i + 1])' "$spawn_json" 2>/dev/null)" || \
    die 'the spawn command carries no port'
  [[ -n "$spawn_port" ]] || die 'the spawn command carries no port'

  export HERMES_TAURI_CMD="$spawn_json"
  start_shell spawn
  wait_window 30 || die 'the shell never mapped a window'
  place_window
  wait_log 'Hermes backend ready at' 90 >/dev/null || die 'the shell never brought its own Hermes up'
  wait_log 'First frame of the Hermes app painted' 60 >/dev/null || die 'the app never painted in spawn mode'

  local pid pgid attempt survived
  while IFS= read -r pid; do
    [[ "$pid" =~ ^[1-9][0-9]*$ ]] || continue
    pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
    [[ -n "$pgid" ]] || continue
    pids+=("$pid")
    pgids+=("$pgid")
  done < <(pgrep -f "hermes webapp --isolated.*--port $spawn_port" 2>/dev/null)
  if ((${#pids[@]} == 0)); then
    record spawn-shutdown FAIL "-" "-" "no spawned Hermes process was found on port $spawn_port"
    return
  fi
  printf 'proof: spawned Hermes pid=%s pgid=%s (port %s)\n' "${pids[*]}" "${pgids[*]}" "$spawn_port"

  stop_shell

  survived=""
  attempt=0
  for attempt in $(seq 1 40); do
    survived=""
    for pid in "${pids[@]}"; do
      kill -0 "$pid" 2>/dev/null && survived="$survived pid:$pid"
    done
    for pgid in "${pgids[@]}"; do
      pgrep -g "$pgid" >/dev/null 2>&1 && survived="$survived pgid:$pgid"
    done
    [[ -z "$survived" ]] && break
    sleep 0.5
  done

  if [[ -z "$survived" ]]; then
    record spawn-shutdown PASS "-" "-" \
      "SIGTERM to the shell stopped its spawned Hermes (pid ${pids[*]}, pgid ${pgids[*]}) within $((attempt / 2 + 1))s"
  else
    record spawn-shutdown FAIL "-" "-" "a spawned Hermes process outlived the shell:$survived"
  fi
}

# ── helpers written at run time ──────────────────────────────────────────────

write_helpers() {
  mkdir -p "$WORK" || die "cannot create $WORK"
  cat >"$WORK/frame_colours.py" <<'PY'
"""Distinct colours on a fixed sample grid of an XWD root capture."""
import struct
import sys

GRID = (32, 20)


def main(path):
    with open(path, "rb") as stream:
        data = stream.read()
    if len(data) < 100:
        raise SystemExit(1)
    fields = struct.unpack(">25I", data[:100])
    header_size, width, height = fields[0], fields[4], fields[5]
    byte_order, bits, stride, ncolors = fields[7], fields[11], fields[12], fields[19]
    if width < 1 or height < 1 or stride < width:
        raise SystemExit(1)
    # Xvfb reports the root as 32bpp with no window mapped and 24bpp with one,
    # both stored in 4-byte groups, so read the first three bytes of each group.
    pixel_bytes = stride // width if stride % width == 0 else (bits + 7) // 8
    if pixel_bytes < 3:
        raise SystemExit(1)
    start = header_size + ncolors * 12
    image = data[start:start + height * stride]
    if len(image) < height * stride:
        raise SystemExit(1)
    cols, rows = GRID
    colours = set()
    for gy in range(rows):
        y = gy * (height - 1) // (rows - 1)
        for gx in range(cols):
            x = gx * (width - 1) // (cols - 1)
            index = y * stride + x * pixel_bytes
            # byte_order 0 is LSBFirst, 1 is MSBFirst: the first three bytes of
            # the group always hold the red, green and blue sample.
            group = image[index:index + 3]
            colours.add(bytes(reversed(group)) if byte_order == 0 else bytes(group))
    print(len(colours))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1]))
PY
  cat >"$WORK/png_colours.py" <<'PY'
"""PNG reader and the distinct-colour count of a scrot capture.

`read_png` is shared with retry_button_xy.py, which needs the pixels.
"""
import struct
import sys
import zlib

GRID = (32, 20)


def read_png(path):
    """(width, height, channels, rows) of an 8-bit, non-interlaced PNG."""
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
                if da <= db and da <= dc:
                    best = left
                elif db <= dc:
                    best = up
                else:
                    best = up_left
                line[i] = (line[i] + best) & 0xFF
        elif kind != 0:
            raise SystemExit(1)
        rows.append(line)
        previous = line
    return width, height, channels, rows


def main(path):
    width, height, channels, rows = read_png(path)
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
  cat >"$WORK/retry_button_xy.py" <<'PY'
"""Where the boot page's Retry button is, in pixels of a scrot capture.

boot/boot.css centers a panel of at most 480 px in the window (`body` is a
grid with `place-items: center` and the panel is its only child) and the
button is the panel's last element, so the button is the lowest bordered box
on the page. How far down it sits depends on how many lines the error message
above it wrapped to, which is why this walks the frame for the box instead of
a fixed spot: a row whose pixels differ from the row above across one
contiguous run of 40..200 px is a border (glyphs give scattered runs, and the
panel's own border and its shadow span the whole panel, so neither
qualifies). The lowest such row is the button's bottom border, the nearest
one 20..60 px above it with an overlapping span is its top border, and the
middle of the two is where a click lands on the button.

Prints "x y" and exits 0; exits 1 when the frame does not hold such a button.
"""
import sys

from png_colours import read_png

MIN_WIDTH = 40
MAX_WIDTH = 200
MIN_HEIGHT = 20
MAX_HEIGHT = 60
# Antialiasing leaves one-row gaps inside a border run.
MAX_GAP = 3


def runs(row, above, width, channels):
    """Contiguous runs where this row differs from the row above it."""
    found = []
    start = last = None
    for x in range(1, width):
        index = x * channels
        if row[index:index + 3] == above[index:index + 3]:
            continue
        if start is None:
            start = last = x
        elif x - last <= MAX_GAP:
            last = x
        else:
            found.append((start, last))
            start = last = x
    if start is not None:
        found.append((start, last))
    return [(a, b) for a, b in found if MIN_WIDTH <= b - a <= MAX_WIDTH]


def main(path):
    width, height, channels, rows = read_png(path)
    candidates = []
    for y in range(1, height):
        for a, b in runs(rows[y], rows[y - 1], width, channels):
            candidates.append((y, a, b))
    if not candidates:
        return 1
    bottom_y, bottom_a, bottom_b = candidates[-1]
    tops = [
        candidate
        for candidate in candidates
        if MIN_HEIGHT <= bottom_y - candidate[0] <= MAX_HEIGHT
        and min(bottom_b, candidate[2]) - max(bottom_a, candidate[1]) >= MIN_WIDTH
    ]
    if not tops:
        return 1
    top_y = tops[-1][0]
    print((bottom_a + bottom_b) // 2, (top_y + bottom_y) // 2)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1]))
PY
}

# A port in the test range with nothing listening on it, found by binding.
free_port() {
  python3 - "$1" <<'PY'
import socket
import sys

skip = int(sys.argv[1]) if len(sys.argv) > 1 else 0
for port in range(18800, 18900):
    if port == skip:
        continue
    with socket.socket() as probe:
        try:
            probe.bind(("127.0.0.1", port))
        except OSError:
            continue
    print(port)
    break
PY
}

write_readme() {
  local index label target="$OUT/README.md"
  # Spawn mode writes no shots and must not clobber the index of the run that
  # did, so it gets its own file next to it.
  [[ "$SPAWN_MODE" == 1 ]] && target="$OUT/README-spawn-shutdown.md"
  {
    printf '# Proof run — %s\n\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf "Shell binary: \`%s\`\n\n" "$BIN"
    printf 'Screen %s, window at 0,0 sized %s; the black margin is the bare Xvfb root.\n\n' \
      "$SCREEN" "$WINDOW_SIZE"
    printf 'Input is real: xdotool keys and clicks plus the shell'"'"'s own deep links.\n'
    printf "The shell's XDG dirs live under \`%s\`, so every run starts from a fresh\n" "$STATE_DIR"
    printf 'webview profile and nothing is written to the real home. Both the shell and\n'
    printf 'the backend run from a scratch working directory, so no local path or git\n'
    printf 'branch name is on screen.\n\n'
    if [[ "$SPAWN_MODE" != 1 ]]; then
      printf '01-boot is the boot page of the main window, with the splash already gone because\n'
      printf 'the start-up failed. 01b-splash is the splash window itself while the app loads:\n'
      printf 'the shell keeps its own boot page in front of the window until the app reports its\n'
      printf 'first painted frame, so there is no blank window. 05-group-chat is a group chat\n'
      printf 'created through the UI with one typed neutral message, not the seeded "Demo\n'
      printf 'Product Room": the seeded room does not hydrate in a fresh webview profile (a UI\n'
      printf 'boot race in the hermes-bots plugin, also in Chromium; see docs/test-hermes.md).\n'
      printf 'The retry check drives the boot page'"'"'s Retry button with two real clicks (see the\n'
      printf 'check table): it locates the button in a live frame, watches the splash come back and\n'
      printf 'go again on a failed attempt, and ends with the app on screen after the test Hermes\n'
      printf 'was started on the port that attempt used.\n\n'
    fi
    printf '| shot | result | colours | md5 | what it shows |\n|---|---|---|---|---|\n'
    for index in "${!SHOT_ORDER[@]}"; do
      label="${SHOT_ORDER[$index]}"
      [[ "${SHOT_COLOURS[$index]}" == "-" ]] || label="$label.png"
      printf '| %s | %s | %s | %s | %s |\n' \
        "$label" "${SHOT_RESULT[$index]}" "${SHOT_COLOURS[$index]}" \
        "${SHOT_MD5[$index]}" "${SHOT_DESC[$index]}"
    done
    if ((${#CHECK_ORDER[@]} > 0)); then
      printf '\n| check | result | what it proves |\n|---|---|---|\n'
      for index in "${!CHECK_ORDER[@]}"; do
        printf '| %s | %s | %s |\n' \
          "${CHECK_ORDER[$index]}" "${CHECK_RESULT[$index]}" "${CHECK_DESC[$index]}"
      done
    fi
    printf "\n\`colours\` is the number of distinct colours on a 32x20 sample grid of each PNG:\n"
    printf 'a blank window scores 1 and the boot page (the splash) 9 and up, so every row\n'
    printf 'with a colour count shows real content. A FAIL line means the shot was written\n'
    printf 'and then rejected, never that a stale file was left behind.\n'
  } >"$target"
}

if [[ "$INNER" == 1 ]]; then
  trap inner_cleanup EXIT INT TERM
  inner_main
else
  outer_main
fi

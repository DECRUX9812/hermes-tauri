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
DEFAULT_OUT=/tmp/hermes-tauri-t6p/shots
DEFAULT_PORT_DIR=/tmp/hermes-tauri-test-proof
WORK=/tmp/hermes-tauri-proof-work

SCREEN="1440x900x24"
WINDOW_TITLE="Hermes Tauri"
WINDOW_SIZE="1280x820"
BOTS_TAB="168,37"                  # sidebar Sessions|Bots toggle, window at 0,0
PARK="600,300"                     # neutral pointer parking spot
DEEPLINK_SHOT_ROUTE="/studio"      # 08-deeplink: a route no earlier shot used
MIN_COLOURS=12                     # painted app pages score well above this

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
declare -a SHOT_ORDER=() SHOT_RESULT=() SHOT_COLOURS=() SHOT_MD5=() SHOT_DESC=()

# shellcheck disable=SC2329  # runs from the EXIT/INT/TERM trap at the end of the file
inner_cleanup() {
  local status=$?
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
  local wid
  wid="$(window_id)"
  [[ -n "$wid" ]] || return 1
  xdotool windowmove "$wid" 0 0 2>/dev/null || true
  xdotool windowsize "$wid" "${WINDOW_SIZE%x*}" "${WINDOW_SIZE#*x}" 2>/dev/null || true
  sleep 0.5
}

# Fast probe of what is on screen (xwd, ~0.2 s), used only to decide when to
# take a shot. The shots themselves are verified from the PNG scrot writes.
probe_colours() {
  xwd -root -silent >"$WORK/probe.xwd" 2>/dev/null || return 1
  python3 "$WORK/frame_colours.py" "$WORK/probe.xwd" 2>/dev/null | tr -dc '0-9'
}

# Painted means the sample grid holds real content: a blank window scores 1 and
# the shell's own loading splash 3.
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
wait_log() {
  local pattern="$1" timeout="${2:-25}" deadline=$((SECONDS + ${2:-25})) line
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

# Captures one shot, checks the frame is painted and not a copy of an earlier
# shot, and prints its PASS or FAIL line.
shot() {
  local name="$1" min="$2" desc="$3" index earlier
  if ! capture "$name"; then
    record "$name" FAIL "-" "-" "the screen could not be captured and read back"
    return 1
  fi
  if ((CAP_COLOURS < min)); then
    record "$name" FAIL "$CAP_COLOURS" "$CAP_MD5" \
      "blank frame: $CAP_COLOURS distinct colours on the sample grid, under $min"
    return 1
  fi
  for index in "${!SHOT_MD5[@]}"; do
    earlier="${SHOT_MD5[$index]}"
    if [[ "$earlier" == "$CAP_MD5" ]]; then
      record "$name" FAIL "$CAP_COLOURS" "$CAP_MD5" \
        "identical to ${SHOT_ORDER[$index]}.png: the screen never changed"
      return 1
    fi
  done
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
  if wait_painted "$MIN_COLOURS" 60; then
    shot 02-sessions "$MIN_COLOURS" \
      "home route after attaching: the seeded demo session titles in the sidebar and the seeded recent tasks on the page"
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

  # 06/07: in-place reload with the shell's own F5 accelerator. The shot after
  # the reload must differ from the one before it, which is what the F5 case is
  # supposed to show.
  xdotool windowfocus "$(window_id)" 2>/dev/null || true
  xdotool mousemove --sync "${PARK%,*}" "${PARK#*,}" 2>/dev/null || true
  sleep 1
  shot 06-reload-before "$MIN_COLOURS" \
    "the /team route before the reload"
  xdotool windowfocus "$(window_id)" 2>/dev/null || true
  xdotool key --clearmodifiers F5
  # A reload must show the webview's loading state before it paints again, so
  # this wait never accepts a painted frame that never left the screen.
  if wait_repaint "$MIN_COLOURS" 45 100000; then
    shot 07-reload-after "$MIN_COLOURS" \
      "the same route after F5: the webview reloaded and repainted, still attached"
  else
    record 07-reload-after FAIL "-" "-" \
      "F5 did not reload the webview: the frame never left its painted state and came back"
  fi

  # 08-deeplink: a second invocation hands another route to the running window.
  nav_shot 08-deeplink "$DEEPLINK_SHOT_ROUTE" "Navigated to .*$DEEPLINK_SHOT_ROUTE" \
    "$MIN_COLOURS" \
    "the same window after a second invocation handed it the ${DEEPLINK_SHOT_ROUTE} route"

  stop_shell
  write_readme
  printf 'proof: done, %s shots, %s\n' "${#SHOT_ORDER[@]}" \
    "$([[ "$SHOT_FAILED" == 1 ]] && printf 'at least one failed' || printf 'all passed')"
  exit "$SHOT_FAILED"
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
  shot "$name" "$min" "$desc"
}

# 05-group-chat: the seeded room. The app pulls the room projection once, at
# plugin registration, before its gateway exists (docs/test-hermes.md), so a
# fresh profile has no room row to open. The shot reports that honestly instead
# of building a lookalike room and pretending it was the seeded one.
group_chat_shot() {
  local reason
  xdotool windowfocus "$(window_id)" 2>/dev/null || true
  xdotool mousemove --sync "${BOTS_TAB%,*}" "${BOTS_TAB#*,}" 2>/dev/null || true
  xdotool click 1
  sleep 3
  reason="the seeded \"Demo Product Room\" never reaches the UI (docs/test-hermes.md: the room projection is pulled before the gateway exists), so no real click can open it"
  if webview_state_mentions demo-product-room; then
    reason="the seeded room is in this profile's room cache, but this driver has no verified click target for a room row"
  fi
  if capture 05-group-chat; then
    record 05-group-chat FAIL "$CAP_COLOURS" "$CAP_MD5" "$reason"
  else
    record 05-group-chat FAIL "-" "-" "$reason (the screen could not be captured)"
  fi
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
  wait_painted "$MIN_COLOURS" 60 || die 'the app never painted in spawn mode'

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
"""Distinct colours on the same grid, read back from a scrot PNG."""
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
  local index label
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
    printf '| shot | result | colours | md5 | what it shows |\n|---|---|---|---|---|\n'
    for index in "${!SHOT_ORDER[@]}"; do
      label="${SHOT_ORDER[$index]}"
      [[ "${SHOT_COLOURS[$index]}" == "-" ]] || label="$label.png"
      printf '| %s | %s | %s | %s | %s |\n' \
        "$label" "${SHOT_RESULT[$index]}" "${SHOT_COLOURS[$index]}" \
        "${SHOT_MD5[$index]}" "${SHOT_DESC[$index]}"
    done
    printf "\n\`colours\` is the number of distinct colours on a 32x20 sample grid of each PNG:\n"
    printf 'a blank window scores 1 and the shell'"'"'s own loading splash 3, so every row with\n'
    printf 'a colour count shows real content. A FAIL line means the shot was written and\n'
    printf 'then rejected, never that a stale file was left behind.\n'
  } >"$OUT/README.md"
}

if [[ "$INNER" == 1 ]]; then
  trap inner_cleanup EXIT INT TERM
  inner_main
else
  outer_main
fi

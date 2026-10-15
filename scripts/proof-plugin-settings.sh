#!/usr/bin/env bash
# Optional T18 proof: Settings > Plugins, driven with real input in the Tauri shell.
#
# Usage: scripts/proof-plugin-settings.sh [--bin PATH] [--out DIR] [--port-dir DIR]
#
# Outer part: starts an isolated test Hermes whose home already carries the neutral
# `demo-settings` fixture plugin (HERMES_TAURI_DEMO_SETTINGS_PLUGIN=1), then re-executes this
# file inside `dbus-run-session -- xvfb-run -a`. A trap stops the test Hermes and every shell
# this run launched on each exit path.
#
# Inner part: launches the shell, walks Capabilities > Plugins, opens the demo plugin's settings
# entry through the gear, edits one value of every non-secret schema type plus a fake secret,
# saves, and proves where each value landed by reading the test home's config.yaml and .env.
# Every step is real input (xdotool clicks and keys); nothing is injected into the page, no
# devtools and no WebDriver are used, and no local storage is edited.
#
# This is a separate script rather than a step in proof.sh: proof.sh's shot list (01-08) is a
# fixed, published contract other proofs read, and this flow needs its own state dir with the
# fixture plugin installed.
set -uo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TEST_HERMES="$SCRIPT_DIR/test-hermes.sh"
CACHE="$HOME/.cache/hermes-tauri"

# The newest master-equivalent build, in the order the project's briefs prescribe.
default_bin() {
  local candidate
  for candidate in "$CACHE/target-T16" "$CACHE/target-T6" "$CACHE/target-T5" "$CACHE/target-t16"; do
    [[ -x "$candidate/release/hermes-tauri" ]] && { printf '%s\n' "$candidate/release/hermes-tauri"; return 0; }
  done
  printf '%s\n' "$CACHE/target-t16/release/hermes-tauri"
}

DEFAULT_BIN="$(default_bin)"
DEFAULT_OUT=/tmp/hermes-tauri-t18/shots
DEFAULT_PORT_DIR=/tmp/hermes-tauri-test-t18-proof
WORK=/tmp/hermes-tauri-t18-proof-work

SCREEN="1440x900x24"
WINDOW_TITLE="Hermes Tauri"
WINDOW_SIZE="1280x820"
MIN_COLOURS=12

# Real screen coordinates inside the window placed at 0,0 sized 1280x820. The rail is icon-only,
# so the page icon for Capabilities is the second entry of the first page group (measured at
# y=211 on this build); the candidate list absorbs a small vertical shift in the layout.
RAIL_CAPABILITIES_CANDIDATES=(24,211 24,207 24,215 24,203 24,219 24,199 24,223)
TABS_PLUGINS="890,58"
ROW_GEAR="896,199"
FIELD_API_URL="1003,273"
FIELD_MODE="1058,338"
MODE_OPTION_CAREFUL="1063,387"
FIELD_RETRIES="1003,399"
FIELD_TIMEOUT="1003,461"
FIELD_VERBOSE="1077,527"
SCROLL_ANCHOR="900,421"
EDITOR_TAGS="633,342"
EDITOR_LIMITS="633,492"
FIELD_SECRET="1008,632"
BUTTON_SAVE="1052,193"

FAKE_SECRET="demo-not-a-real-key"

BIN="$DEFAULT_BIN"
OUT="$DEFAULT_OUT"
PORT_DIR="$DEFAULT_PORT_DIR"
INNER=0

usage() {
  cat >&2 <<'USAGE'
usage: scripts/proof-plugin-settings.sh [--bin PATH] [--out DIR] [--port-dir DIR]

  --bin PATH       shell binary to drive (default: the shared T16 release build)
  --out DIR        shot and log output dir
  --port-dir DIR   test Hermes state dir; must live under /tmp/hermes-tauri-test*
USAGE
  exit 2
}

abspath() {
  local path="$1"
  if [[ "$path" == /* ]]; then printf '%s\n' "$path"; else printf '%s/%s\n' "$PWD" "$path"; fi
}

while (($# > 0)); do
  case "$1" in
    --bin) BIN="${2:-}"; shift 2 ;;
    --bin=*) BIN="${1#*=}"; shift ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --out=*) OUT="${1#*=}"; shift ;;
    --port-dir) PORT_DIR="${2:-}"; shift 2 ;;
    --port-dir=*) PORT_DIR="${1#*=}"; shift ;;
    --inner) INNER=1; shift ;;
    -h|--help) usage ;;
    *) printf 'proof-plugin-settings: unknown argument %s\n' "$1" >&2; usage ;;
  esac
done
BIN="$(abspath "$BIN")"
OUT="$(abspath "$OUT")"
PORT_DIR="$(abspath "$PORT_DIR")"

case "$PORT_DIR" in
  /tmp/hermes-tauri-test*) ;;
  *)
    printf 'proof-plugin-settings: --port-dir must live under /tmp/hermes-tauri-test*, refusing %s\n' \
      "$PORT_DIR" >&2
    exit 2
    ;;
esac

die() { printf 'proof-plugin-settings: %s\n' "$*"; exit 1; }

# ── outer part ───────────────────────────────────────────────────────────────

OUTER_TEST_HERMES=0

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
  if ! rm -rf "$WORK" || ! mkdir -p "$WORK"; then die "cannot create $WORK"; fi

  printf 'proof-plugin-settings: starting the isolated test Hermes (with the demo-settings fixture)\n'
  HERMES_TAURI_DEMO_SETTINGS_PLUGIN=1 HERMES_TAURI_TEST_DIR="$PORT_DIR" \
    "$TEST_HERMES" start >/dev/null || die "test Hermes did not start (state dir $PORT_DIR)"
  OUTER_TEST_HERMES=1
  trap outer_cleanup EXIT INT TERM

  printf 'proof-plugin-settings: entering dbus-run-session + xvfb-run (%s)\n' "$SCREEN"
  dbus-run-session -- xvfb-run -a -s "-screen 0 $SCREEN" \
    "$SCRIPT_DIR/proof-plugin-settings.sh" --inner --bin "$BIN" --out "$OUT" --port-dir "$PORT_DIR" \
    2>"$OUT/dbus-stderr.log"
  local status=$?
  if ((status != 0)); then
    printf 'proof-plugin-settings: inner run exited %s; session stderr tail:\n' "$status"
    grep -v '^[[:space:]]*$' "$OUT/dbus-stderr.log" | tail -n 15
  fi
  exit "$status"
}

# ── inner part ───────────────────────────────────────────────────────────────

SHELL_PID=0
SHELL_LOG=""
SHELL_LOG_LINES=0
STATE_DIR=""
declare -a CHECK_NAME=() CHECK_RESULT=() CHECK_DETAIL=()
FAILED=0

# shellcheck disable=SC2329  # runs from the EXIT/INT/TERM trap at the end of the file
inner_cleanup() {
  local status=$?
  stop_shell
  rm -f "$OUT"/shell-*.pid 2>/dev/null || true
  exit "$status"
}

record() {
  local name="$1" result="$2" detail="$3"
  CHECK_NAME+=("$name"); CHECK_RESULT+=("$result"); CHECK_DETAIL+=("$detail")
  if [[ "$result" == PASS ]]; then
    printf 'PASS %s — %s\n' "$name" "$detail"
  else
    printf 'FAIL %s — %s\n' "$name" "$detail"
    FAILED=1
  fi
}

shell_pgid() { ps -o pgid= -p "$SHELL_PID" 2>/dev/null | tr -d '[:space:]'; }

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

start_shell() {
  local label="$1"; shift
  stop_shell
  SHELL_LOG="$OUT/shell-$label.log"
  : >"$SHELL_LOG"
  SHELL_LOG_LINES=0
  setsid "$BIN" "$@" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!
  printf '%s\n' "$SHELL_PID" >"$OUT/shell-$label.pid"
  sleep 2
  if ! kill -0 "$SHELL_PID" 2>/dev/null; then
    printf 'proof-plugin-settings: the shell (%s) died within 2 s; log tail:\n' "$label"
    tail -n 20 "$SHELL_LOG"
    exit 1
  fi
  printf 'proof-plugin-settings: shell %s pid=%s pgid=%s log=%s\n' \
    "$label" "$SHELL_PID" "$(shell_pgid)" "$SHELL_LOG"
}

window_id() { xdotool search --name "$WINDOW_TITLE" 2>/dev/null | head -n 1; }

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

probe_colours() {
  xwd -root -silent >"$WORK/probe.xwd" 2>/dev/null || return 1
  python3 "$WORK/frame_colours.py" "$WORK/probe.xwd" 2>/dev/null | tr -dc '0-9'
}

wait_painted() {
  local min="$1" timeout="${2:-45}" deadline=$((SECONDS + ${2:-45})) colours="" hits=0
  while ((SECONDS < deadline)); do
    colours="$(probe_colours)"
    if [[ -n "$colours" ]] && ((colours >= min)); then
      hits=$((hits + 1))
      ((hits >= 2)) && return 0
      sleep 1.5
      continue
    fi
    hits=0
    sleep 0.3
  done
  printf 'proof-plugin-settings: frame stayed below %s colours (last: %s) after %ss\n' \
    "$min" "${colours:-unreadable}" "$timeout"
  return 1
}

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
    kill -0 "$SHELL_PID" 2>/dev/null || { printf 'proof-plugin-settings: the shell died\n'; tail -n 20 "$SHELL_LOG"; exit 1; }
    sleep 0.4
  done
  return 1
}

focus_window() { xdotool windowfocus "$(window_id)" 2>/dev/null || true; }

# Geometry of the driven window, so a misplaced window shows up in the log.
log_geometry() {
  local wid
  wid="$(window_id)"
  [[ -n "$wid" ]] || { printf 'proof-plugin-settings: no window\n'; return; }
  xdotool getwindowgeometry "$wid" 2>/dev/null | tr '\n' ' '
  printf '\n'
}

# Debug captures (T18_PROOF_DEBUG=1) land next to the shots as debug-<name>.png.
debug_shot() {
  [[ "${T18_PROOF_DEBUG:-0}" == 1 ]] || return 0
  sleep 0.6
  scrot -o "$OUT/debug-$1.png" 2>/dev/null || true
  printf 'proof-plugin-settings: debug shot %s\n' "$1"
}

# click <x,y> [settle]
click() {
  local point="$1" settle="${2:-0.8}"
  focus_window
  xdotool mousemove --sync "${point%,*}" "${point#*,}" 2>/dev/null || true
  sleep 0.35
  xdotool click 1
  sleep "$settle"
}

hover() {
  focus_window
  xdotool mousemove --sync "${1%,*}" "${1#*,}" 2>/dev/null || true
  sleep "${2:-1.2}"
}

scroll_down() { focus_window; xdotool mousemove --sync "${SCROLL_ANCHOR%,*}" "${SCROLL_ANCHOR#*,}"; sleep 0.3; xdotool click --repeat "${1:-5}" 5; sleep 1.5; }
scroll_up() { focus_window; xdotool mousemove --sync "${SCROLL_ANCHOR%,*}" "${SCROLL_ANCHOR#*,}"; sleep 0.3; xdotool click --repeat "${1:-6}" 4; sleep 1.5; }

# replace_field <x,y> <text> — focus, select all, replace, type slowly (WebKitGTK drops fast keys)
replace_field() {
  local point="$1" text="$2"
  click "$point" 0.6
  xdotool key --clearmodifiers ctrl+a
  sleep 0.35
  xdotool key --clearmodifiers BackSpace
  sleep 0.5
  xdotool type --delay 300 "$text"
  sleep 1.0
}

shot() {
  local name="$1" min="${2:-$MIN_COLOURS}" desc="$3"
  local file="$OUT/$name.png" colours md5
  if ! scrot -o "$file"; then record "$name" FAIL 'the screen could not be captured'; return 1; fi
  colours="$(python3 "$WORK/png_colours.py" "$file" 2>/dev/null | tr -dc '0-9')"
  md5="$(md5sum "$file" | cut -d' ' -f1)"
  if [[ -z "$colours" ]] || ((colours < min)); then
    record "$name" FAIL "blank frame (${colours:-unreadable} colours): $desc"
    return 1
  fi
  record "$name" PASS "$desc (colours=$colours md5=$md5)"
}

# Screen regions OCR'd to confirm a step. Each is a tight crop of one control or one row: OCR cost
# grows with the pixel count, so a full-window crop turns every wait into minutes on a loaded
# machine, while these stay fast.
REGION_TABS="650,42,300,32"       # the Skills / Tools / Connectors / Plugins tab strip
REGION_INSTALL="1050,92,160,30"   # the Plugins tab's "Install from Git" button
REGION_ROW="312,178,320,30"       # the demo-settings row title
REGION_SAVE="980,178,180,34"      # the form's "Save settings" button
REGION_API_URL="905,258,270,26"   # the form's API URL input
REGION_SECRET="1100,610,320,45"   # the masked secret input

# OCR text of a "x,y,w,h" region.
ocr_at() {
  local x y w h
  IFS=, read -r x y w h <<<"$1"
  scrot -o "$WORK/ocr.png" 2>/dev/null || return 1
  python3 "$WORK/crop.py" "$WORK/ocr.png" "$WORK/ocr-crop.png" "$x" "$y" "$w" "$h" 2 >/dev/null || return 1
  tesseract "$WORK/ocr-crop.png" - --psm 7 2>/dev/null | tr '\n' ' '
}

# wait_text <region> <needle> [min_samples] [timeout] — true once the region's OCR contains the
# needle. The wait polls until the deadline, but never decides on fewer than `min_samples` reads:
# one OCR of a region takes as long as the machine is loaded, so a single early sample taken
# before the page repaints would otherwise fail a step that is actually working.
wait_text() {
  local region="$1" needle="$2" min_samples="${3:-2}" timeout="${4:-20}"
  local deadline=$((SECONDS + timeout)) samples=0 text=""
  while ((SECONDS < deadline || samples < min_samples)); do
    text="$(ocr_at "$region")"
    samples=$((samples + 1))
    if printf '%s' "$text" | grep -qi -- "$needle"; then return 0; fi
    sleep 1
  done
  printf 'proof-plugin-settings: never saw %q in region %s after %s reads (last OCR: %s)\n' \
    "$needle" "$region" "$samples" "${text:0:120}"
  return 1
}

# The shell paints a dark boot page first and only then the Hermes UI, so a rail click before the
# app renders lands on the boot page and does nothing. The icon rail's column separates the two
# states cleanly: the boot page is dark edge to edge, while the rendered rail is light with dark
# glyphs (measured 6400 dark pixels on the boot page against ~230 on the app).
write_rail_dark() {
  cat >"$WORK/rail_dark.py" <<'PY'
"""Dark pixels in the icon rail's column of a scrot PNG."""
import sys

from PIL import Image

X0, X1, Y0, Y1, LIMIT = 16, 32, 140, 540, 140


def main(path):
    px = Image.open(path).convert("RGB").load()
    print(sum(1 for y in range(Y0, Y1) for x in range(X0, X1) if max(px[x, y]) < LIMIT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1]))
PY
}

rail_dark() {
  scrot -o "$WORK/rail.png" 2>/dev/null || return 1
  python3 "$WORK/rail_dark.py" "$WORK/rail.png" 2>/dev/null | tr -dc '0-9'
}

# Waits until the app (not the boot page) is on screen.
wait_app_ready() {
  local timeout="${1:-90}" deadline=$((SECONDS + ${1:-90})) dark=""
  while ((SECONDS < deadline)); do
    dark="$(rail_dark)"
    if [[ -n "$dark" ]] && ((dark < 2000)); then return 0; fi
    sleep 1
  done
  printf 'proof-plugin-settings: the app never left its boot page (rail dark: %s)\n' "${dark:-unreadable}"
  return 1
}

# The rail is icon-only, so the page is confirmed by OCR rather than assumed; the candidate list
# absorbs a small vertical shift in this build's rail layout, and each candidate is clicked twice
# because the first click can land while the page is still repainting.
goto_capabilities() {
  local point attempt
  for point in "${RAIL_CAPABILITIES_CANDIDATES[@]}"; do
    for attempt in 1 2; do
      click "$point" 3
      wait_painted "$MIN_COLOURS" 30 || true
      if wait_text "$REGION_TABS" 'Connectors' 1 12; then
        printf 'proof-plugin-settings: the rail icon at %s opened Capabilities\n' "$point"
        return 0
      fi
    done
  done
  return 1
}

open_plugins_tab() {
  local attempt
  for attempt in 1 2 3; do
    click "$TABS_PLUGINS" 3
    wait_text "$REGION_INSTALL" 'Install' 1 20 && return 0
  done
  return 1
}

inner_main() {
  printf 'proof-plugin-settings: DISPLAY=%s shell=%s\n' "${DISPLAY:-<unset>}" "$BIN"
  command -v xdpyinfo >/dev/null 2>&1 || die 'xdpyinfo is required'
  xdpyinfo >/dev/null 2>&1 || die "no usable X display (DISPLAY=${DISPLAY:-<unset>})"
  [[ -x "$BIN" ]] || die "not executable: $BIN"
  for tool in scrot xdotool xwd tesseract python3; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
  done

  mkdir -p "$OUT" || die "cannot create $OUT"
  rm -f "$OUT"/*.png "$OUT"/shell-*.log "$OUT"/shell-*.pid 2>/dev/null
  write_helpers

  STATE_DIR="$OUT/shell-state"
  rm -rf "$STATE_DIR"
  mkdir -p "$STATE_DIR"
  export XDG_DATA_HOME="$STATE_DIR/data" XDG_CONFIG_HOME="$STATE_DIR/config"
  export XDG_CACHE_HOME="$STATE_DIR/cache" XDG_STATE_HOME="$STATE_DIR/state"
  export NO_AT_BRIDGE=1
  # A stub xdg-open first on PATH: the shell must never reach a real browser.
  mkdir -p "$WORK/bin"
  cat >"$WORK/bin/xdg-open" <<'SH'
#!/bin/sh
printf '%s\n' "$*" >> "${XDG_OPEN_LOG:-/dev/null}"
exit 0
SH
  chmod +x "$WORK/bin/xdg-open"
  export PATH="$WORK/bin:$PATH"
  export BROWSER="$WORK/bin/xdg-open" XDG_OPEN_LOG="$OUT/opened-urls.log"
  : >"$XDG_OPEN_LOG"

  local origin
  origin="$(HERMES_TAURI_TEST_DIR="$PORT_DIR" "$TEST_HERMES" status | awk '{print $2}')"
  [[ "$origin" == http://127.0.0.1:* ]] || die "the test Hermes is not running ($origin)"
  [[ -r "$PORT_DIR/launch-url" ]] || die "the launch link is missing in $PORT_DIR"
  printf 'proof-plugin-settings: test Hermes at %s\n' "$origin"

  local home="$PORT_DIR/hermes"
  [[ -f "$home/plugins/demo-settings/plugin.yaml" ]] || die 'the demo-settings fixture is not installed in the test home'

  start_shell main --url "$(cat "$PORT_DIR/launch-url")"
  wait_window 30 || die 'the shell never mapped a window'
  place_window
  log_geometry
  wait_log 'Hermes backend ready at' 45 >/dev/null || die 'the shell never connected to the test Hermes'
  wait_painted "$MIN_COLOURS" 60 || die 'the app never painted after attaching'
  wait_app_ready 90 || true
  debug_shot 00-attached

  # 1. Capabilities > Plugins: the demo plugin row and its gear. The desktop rows paint from the
  #    app's own records, but the agent rows come from `plugins.manage` and only land once the
  #    gateway socket has settled (the shell reconnects for a while after attaching), so the row
  #    wait is generous and the debug shots trace what the screen showed meanwhile.
  goto_capabilities || record 08-plugin-settings-entry FAIL 'the rail never opened Capabilities'
  debug_shot 01-capabilities
  open_plugins_tab || record 08-plugin-settings-entry FAIL 'the Plugins tab never opened'
  debug_shot 02-plugins-tab
  if wait_text "$REGION_ROW" 'demo-settings' 3 120; then
    hover "$ROW_GEAR" 2.0
    shot 08-plugin-settings-entry "$MIN_COLOURS" \
      "Capabilities > Plugins: the demo-settings row with its gear (tooltip 'Settings: demo-settings')"
  else
    record 08-plugin-settings-entry FAIL 'the demo-settings row never appeared on Capabilities > Plugins'
  fi

  # 2. The gear opens Settings > Plugins on the plugin's own entry.
  click "$ROW_GEAR" 5
  if wait_text "$REGION_SAVE" 'Save' 2 40; then
    shot 09-plugin-settings-form "$MIN_COLOURS" \
      "Settings > Plugins > demo-settings: the schema form with every field type"
  else
    record 09-plugin-settings-form FAIL 'the settings form never opened'
  fi

  # 3. Change one value of each non-secret type and save.
  replace_field "$FIELD_API_URL" "https://api.example.com/v2"
  replace_field "$FIELD_RETRIES" "7"
  replace_field "$FIELD_TIMEOUT" "4.5"
  click "$FIELD_MODE" 1.2
  click "$MODE_OPTION_CAREFUL" 1.5
  click "$FIELD_VERBOSE" 1.2
  scroll_down 5
  replace_field "$EDITOR_TAGS" '["demo", "launch"]'
  replace_field "$EDITOR_LIMITS" '{"rps": 9, "burst": 2}'
  # 4. A fake secret goes in the masked field; it must land in .env, never config.yaml.
  replace_field "$FIELD_SECRET" "$FAKE_SECRET"
  shot 09b-plugin-settings-secret "$MIN_COLOURS" \
    "the same form scrolled down: the JSON editors and the masked secret field (dots, never plaintext)"
  assert_secret_masked
  save_settings "$home"

  assert_saved "$home"

  # 5. Relaunch with the same webview profile: the saved values must come back.
  stop_shell
  start_shell restart --url "$(cat "$PORT_DIR/launch-url")"
  wait_window 30 || die 'the shell never mapped a window after the relaunch'
  place_window
  wait_log 'Hermes backend ready at' 45 >/dev/null || die 'the shell never reconnected after the relaunch'
  wait_painted "$MIN_COLOURS" 60 || true
  wait_app_ready 90 || true
  goto_capabilities
  open_plugins_tab
  if wait_text "$REGION_ROW" 'demo-settings' 3 120; then
    click "$ROW_GEAR" 5
    if wait_text "$REGION_SAVE" 'Save' 2 40 && wait_text "$REGION_API_URL" 'api.example.com/v2' 2 20; then
      shot 10-plugin-settings-reloaded "$MIN_COLOURS" \
        "after a relaunch on the same profile: the form still shows the saved values"
    else
      record 10-plugin-settings-reloaded FAIL 'the saved values did not come back after the relaunch'
    fi
  else
    record 10-plugin-settings-reloaded FAIL 'the demo-settings row never appeared after the relaunch'
  fi

  stop_shell
  write_readme
  printf 'proof-plugin-settings: done, %s\n' \
    "$([[ "$FAILED" == 1 ]] && printf 'at least one check failed' || printf 'all checks passed')"
  exit "$FAILED"
}

# The secret input renders dots; the typed value must never appear on screen in plaintext.
assert_secret_masked() {
  local text
  text="$(ocr_at "$REGION_SECRET")"
  if printf '%s' "$text" | grep -qF -- "$FAKE_SECRET"; then
    record 'the secret field stays masked' FAIL 'the typed secret was readable on screen'
  else
    record 'the secret field stays masked' PASS 'the typed secret never appeared in plaintext (OCR found no copy of it)'
  fi
}

# The values the form is expected to have written, one per non-secret field type.
SAVED_EXPECT=('api_url: https://api.example.com/v2' 'mode: careful' 'retries: 7'
              'timeout_seconds: 4.5' 'verbose: true' 'rps: 9' 'burst: 2' 'launch')

# True once config.yaml carries every expected value.
block_complete() {
  local block="$1" expect
  for expect in "${SAVED_EXPECT[@]}"; do
    printf '%s' "$block" | grep -qF -- "$expect" || return 1
  done
  return 0
}

# Waits for the backend to flush the settings block into the test home's config.yaml.
wait_saved_block() {
  local home="$1" timeout="${2:-25}" deadline=$((SECONDS + ${2:-25})) block=""
  while ((SECONDS < deadline)); do
    block="$(saved_block "$home")"
    block_complete "$block" && return 0
    sleep 1.5
  done
  return 1
}

# Saves the form and confirms the backend wrote it. Saving again is idempotent (the same values,
# and the secret is rewritten with the same value), so a click the webview dropped — or a save
# whose two writes did not both land — is retried instead of failing the proof.
save_settings() {
  local home="$1" attempt
  for attempt in 1 2 3; do
    scroll_up 8
    click "$BUTTON_SAVE" 5
    wait_saved_block "$home" 25 && return 0
    printf 'proof-plugin-settings: save attempt %s did not reach config.yaml yet, retrying\n' "$attempt"
  done
  printf 'proof-plugin-settings: the form never saved after 3 attempts\n'
  return 1
}

# The settings block under plugins.entries.<id>.settings, or empty when it is not there yet.
saved_block() {
  python3 - "$1/config.yaml" <<'PY'
import sys
from pathlib import Path
try:
    text = Path(sys.argv[1]).read_text(encoding="utf-8")
except OSError:
    raise SystemExit(0)
lines = text.splitlines()
out, inside, indent = [], False, None
for line in lines:
    stripped = line.strip()
    if stripped.startswith("demo-settings:"):
        inside = True
        indent = len(line) - len(line.lstrip())
        out.append(line)
        continue
    if inside:
        if stripped and (len(line) - len(line.lstrip())) <= indent:
            break
        out.append(line)
print("\n".join(out).rstrip())
PY
}

# Reads the test home and proves where each saved value landed. The form's save is two writes
# (config.yaml through `plugins.manage settings`, then the secret through the credential route),
# so the block is polled rather than read once: a single read races the backend's flush.
assert_saved() {
  local home="$1" block="" env_line expect
  local expected=(api_url: 'https://api.example.com/v2' mode: careful retries: 7
                  timeout_seconds: 4.5 verbose: true rps: 9 burst: 2 launch)
  local deadline=$((SECONDS + 45)) missing=0
  while ((SECONDS < deadline)); do
    block="$(saved_block "$home")"
    missing=0
    for expect in "${expected[@]}"; do
      printf '%s' "$block" | grep -qF -- "$expect" || missing=1
    done
    ((missing == 0)) && break
    sleep 1.5
  done
  printf '\n--- config.yaml plugins.entries.demo-settings block ---\n%s\n---\n' "$block"
  for expect in 'api_url: https://api.example.com/v2' 'mode: careful' 'retries: 7' \
                'timeout_seconds: 4.5' 'verbose: true' 'rps: 9' 'burst: 2' 'launch'; do
    if printf '%s' "$block" | grep -qF -- "$expect"; then
      record "config.yaml has '$expect'" PASS 'written by the Desktop form through plugins.manage settings'
    else
      record "config.yaml has '$expect'" FAIL "missing from the saved block: $expect"
    fi
  done
  if printf '%s' "$block" | grep -qi 'api_key'; then
    record 'config.yaml has no secret' FAIL 'the secret key appeared in config.yaml'
  else
    record 'config.yaml has no secret' PASS 'no api_key key under plugins.entries.demo-settings.settings'
  fi

  env_line=""
  deadline=$((SECONDS + 30))
  while ((SECONDS < deadline)); do
    env_line="$(grep -m1 '^DEMO_SETTINGS_API_KEY=' "$home/.env" 2>/dev/null || true)"
    [[ -n "$env_line" ]] && break
    sleep 1.5
  done
  if [[ -n "$env_line" ]]; then
    record '.env carries the secret' PASS "DEMO_SETTINGS_API_KEY=${FAKE_SECRET:0:9}… (value redacted past its prefix)"
  else
    record '.env carries the secret' FAIL 'DEMO_SETTINGS_API_KEY is missing from the test home .env'
  fi
}

# ── helpers written at run time ──────────────────────────────────────────────

write_helpers() {
  mkdir -p "$WORK" || die "cannot create $WORK"
  write_frame_colours
  write_png_colours
  write_crop
  write_rail_dark
}

write_frame_colours() {
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
            group = image[index:index + 3]
            colours.add(bytes(reversed(group)) if byte_order == 0 else bytes(group))
    print(len(colours))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1]))
PY
}

write_png_colours() {
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

write_crop() {
  cat >"$WORK/crop.py" <<'PY'
"""Crop and upscale a region of a PNG so tesseract reads a small control."""
import sys

from PIL import Image


def main() -> int:
    src, dst, x, y, w, h = sys.argv[1:7]
    scale = int(sys.argv[7]) if len(sys.argv) > 7 else 2
    image = Image.open(src).convert("RGB")
    box = image.crop((int(x), int(y), int(x) + int(w), int(y) + int(h)))
    box = box.resize((box.width * scale, box.height * scale), Image.NEAREST)
    box.save(dst)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
PY
}

write_readme() {
  local index
  {
    printf '# Plugin settings proof — %s\n\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'Shell binary: `%s`\n\n' "$BIN"
    printf 'Test Hermes state dir: `%s` (isolated; never the real home)\n\n' "$PORT_DIR"
    printf 'Screen %s, window at 0,0 sized %s. Input is real xdotool clicks and keys; no page\n' \
      "$SCREEN" "$WINDOW_SIZE"
    printf 'injection, no devtools, no WebDriver, no local-storage edits.\n\n'
    printf '| check | result | detail |\n|---|---|---|\n'
    for index in "${!CHECK_NAME[@]}"; do
      printf '| %s | %s | %s |\n' "${CHECK_NAME[$index]}" "${CHECK_RESULT[$index]}" "${CHECK_DETAIL[$index]}"
    done
  } >"$OUT/README.md"
}

if [[ "$INNER" == 1 ]]; then
  trap inner_cleanup EXIT INT TERM
  inner_main
else
  outer_main
fi

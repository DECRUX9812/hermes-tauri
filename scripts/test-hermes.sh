#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
PIN="${HERMES_TAURI_PIN:-$REPO_ROOT/../_hermes-pin}"
DEFAULT_STATE_DIR=/tmp/hermes-tauri-test
STATE_DIR="${HERMES_TAURI_TEST_DIR:-$DEFAULT_STATE_DIR}"

fail() {
  printf 'test-hermes: %s\n' "$*" >&2
  exit 1
}

resolve_state_dir() {
  STATE_DIR="$(realpath -m -- "$STATE_DIR")" || fail 'could not resolve the test state directory'
  case "$STATE_DIR" in
    "$DEFAULT_STATE_DIR"|"$DEFAULT_STATE_DIR"-*|"$DEFAULT_STATE_DIR"/*) ;;
    *) fail "state directory must be $DEFAULT_STATE_DIR or a sibling beginning with ${DEFAULT_STATE_DIR##*/}-" ;;
  esac
  SERVER_LOG="$STATE_DIR/logs/server.log"
  PID_FILE="$STATE_DIR/pid"
  RUN_ID_FILE="$STATE_DIR/run-id"
  PORT_FILE="$STATE_DIR/port"
  LAUNCH_URL_FILE="$STATE_DIR/launch-url"
}

resolve_state_dir

require_pin() {
  [[ -d "$PIN" ]] || fail "pinned Hermes checkout not found at $PIN; set HERMES_TAURI_PIN to an absolute path"
  [[ -f "$PIN/hermes" ]] || fail 'pinned Hermes checkout is missing'
  [[ -f "$PIN/apps/desktop/dist-webapp/index.html" ]] || fail 'pinned webapp build is missing'
}

resolve_python() {
  local launcher install_root candidate system_python

  if [[ -n "${HERMES_TAURI_PYTHON:-}" ]]; then
    [[ -x "$HERMES_TAURI_PYTHON" ]] || fail 'HERMES_TAURI_PYTHON is not executable'
    printf '%s\n' "$HERMES_TAURI_PYTHON"
    return
  fi

  launcher="$(command -v hermes || true)"
  [[ -n "$launcher" && -r "$launcher" ]] || fail 'Hermes launcher not found; set HERMES_TAURI_PYTHON'
  system_python="$(command -v python3 || true)"
  [[ -n "$system_python" ]] || fail 'python3 is required to inspect the existing Hermes environment'

  install_root="$(
    "$system_python" - "$launcher" <<'PY'
import shlex
import sys
from pathlib import Path

launcher = Path(sys.argv[1]).resolve()
try:
    lines = launcher.read_text(encoding="utf-8", errors="replace").splitlines()
except OSError:
    raise SystemExit(1)

for line in lines:
    stripped = line.strip()
    if not stripped.startswith("exec "):
        continue
    try:
        words = shlex.split(stripped)
    except ValueError:
        continue
    if len(words) < 2:
        continue
    target = Path(words[1])
    if target.name == "hermes" and target.parent.name == "bin" and target.parent.parent.name == ".hermes":
        print(target.parents[2])
        raise SystemExit(0)
raise SystemExit(1)
PY
  )" || fail 'could not locate the existing Hermes install; set HERMES_TAURI_PYTHON'

  candidate="$(
    PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$PIN" "$system_python" - "$install_root" <<'PY'
import os
import sys
from pathlib import Path

os.environ.pop("HERMES_RUNTIME_DIR", None)
sys.path.insert(0, os.environ["PYTHONPATH"].split(os.pathsep)[0])
from pm.environments import selected_venv

print(selected_venv(Path(sys.argv[1])) / "bin" / "python")
PY
  )" || fail 'could not resolve the existing Hermes Python environment'
  [[ -x "$candidate" ]] || fail 'the selected Hermes Python interpreter is unavailable'
  printf '%s\n' "$candidate"
}

server_env() {
  local python_path="$1"
  SERVER_ENV=(
    "HOME=$STATE_DIR/home"
    "HERMES_HOME=$STATE_DIR/hermes"
    "HERMES_RUNTIME_DIR=$STATE_DIR/runtime"
    "HERMES_DISABLE_LAZY_INSTALLS=1"
    "XDG_CACHE_HOME=$STATE_DIR/cache"
    "XDG_CONFIG_HOME=$STATE_DIR/config"
    "XDG_DATA_HOME=$STATE_DIR/data"
    "XDG_STATE_HOME=$STATE_DIR/state"
    "XDG_RUNTIME_DIR=$STATE_DIR/xdg-runtime"
    "TMPDIR=$STATE_DIR/tmp"
    "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    "PYTHONPATH=$PIN"
    "PYTHONNOUSERSITE=1"
    "PYTHONDONTWRITEBYTECODE=1"
    "PYTHONUTF8=1"
  )
  PYTHON="$python_path"
}

ensure_state() {
  umask 077
  mkdir -p \
    "$STATE_DIR/home" "$STATE_DIR/hermes" "$STATE_DIR/runtime" \
    "$STATE_DIR/cache" "$STATE_DIR/config" "$STATE_DIR/data" \
    "$STATE_DIR/state" "$STATE_DIR/xdg-runtime" "$STATE_DIR/tmp" \
    "$STATE_DIR/logs"
  chmod 700 "$STATE_DIR" "$STATE_DIR/home" "$STATE_DIR/hermes" "$STATE_DIR/runtime" \
    "$STATE_DIR/cache" "$STATE_DIR/config" "$STATE_DIR/data" "$STATE_DIR/state" \
    "$STATE_DIR/xdg-runtime" "$STATE_DIR/tmp" "$STATE_DIR/logs"
}

write_test_config() {
  cat > "$STATE_DIR/hermes/config.yaml" <<'YAML'
onboarding:
  run: false
model:
  provider: custom
  base_url: http://127.0.0.1:18899/v1
  default: test-model
  api_key: no-key-required
YAML
  chmod 600 "$STATE_DIR/hermes/config.yaml"
}

port_is_free() {
  local port="$1"
  ! ss -H -ltn | awk -v port="$port" '$4 ~ (":" port "$") { found = 1 } END { exit !found }'
}

choose_port() {
  local port
  for port in $(seq 18800 18899); do
    if port_is_free "$port"; then
      printf '%s\n' "$port"
      return
    fi
  done
  fail 'no free test port in 18800-18899'
}

validate_port() {
  local port="$1"
  [[ "$port" =~ ^[0-9]{1,5}$ ]] || fail 'port must be an integer from 18800 through 18899'
  (( port >= 18800 && port <= 18899 )) || fail 'port must be in 18800-18899'
}

make_server_command() {
  local port="$1" run_id="$2"
  SERVER_CMD=(
    env -i
    "${SERVER_ENV[@]}"
    "HERMES_TAURI_TEST_RUN_ID=$run_id"
    "$PYTHON" "$PIN/hermes" webapp
    --isolated --host 127.0.0.1 --port "$port" --skip-build --no-open
  )
}

run_seed() {
  env -i "${SERVER_ENV[@]}" "$PYTHON" "$SCRIPT_DIR/seed-demo.py"
}

read_pid() {
  local pid
  [[ -r "$PID_FILE" ]] || return 1
  IFS= read -r pid < "$PID_FILE" || return 1
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 1
  printf '%s\n' "$pid"
}

owns_server_group() {
  local pid="$1" run_id pgid
  [[ -r "$RUN_ID_FILE" && -r "/proc/$pid/environ" ]] || return 1
  IFS= read -r run_id < "$RUN_ID_FILE" || return 1
  [[ "$run_id" =~ ^[0-9a-f]{32}$ ]] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
  [[ "$pgid" == "$pid" ]] || return 1
  tr '\0' '\n' < "/proc/$pid/environ" | grep -Fqx -- "HERMES_TAURI_TEST_RUN_ID=$run_id"
}

clear_runtime_files() {
  rm -f -- "$PID_FILE" "$RUN_ID_FILE" "$PORT_FILE" "$LAUNCH_URL_FILE"
}

stop_server() {
  local pid attempt
  if ! pid="$(read_pid)"; then
    [[ ! -e "$PID_FILE" ]] || fail 'pid file is invalid; refusing to signal a process'
    clear_runtime_files
    printf 'stopped\n'
    return
  fi

  if ! kill -0 "$pid" 2>/dev/null; then
    clear_runtime_files
    printf 'stopped\n'
    return
  fi
  owns_server_group "$pid" || fail 'pid does not identify this test server; refusing to signal it'

  kill -TERM -- "-$pid"
  for attempt in $(seq 1 40); do
    if ! kill -0 -- "-$pid" 2>/dev/null; then
      clear_runtime_files
      printf 'stopped\n'
      return
    fi
    sleep 0.25
  done

  kill -KILL -- "-$pid" 2>/dev/null || true
  for attempt in $(seq 1 20); do
    if ! kill -0 -- "-$pid" 2>/dev/null; then
      clear_runtime_files
      printf 'stopped\n'
      return
    fi
    sleep 0.25
  done
  fail 'server process group did not exit after SIGKILL'
}

extract_launch_url() {
  local port="$1"
  grep -Em1 -o "http://127\\.0\\.0\\.1:${port}/?#hermes-session=[A-Za-z0-9._~-]+" "$SERVER_LOG" || true
}

start_server() {
  local requested_port="${1:-}" port python run_id pid origin launch_url attempt
  [[ ! -e "$PID_FILE" ]] || {
    if pid="$(read_pid)" && owns_server_group "$pid"; then
      fail 'test Hermes is already running; use status or stop first'
    fi
    fail 'stale or invalid pid file exists; inspect or reset the test directory'
  }

  require_pin
  python="$(resolve_python)"
  server_env "$python"
  ensure_state
  if [[ -n "$requested_port" ]]; then
    validate_port "$requested_port"
    port="$requested_port"
  else
    port="$(choose_port)"
  fi
  port_is_free "$port" || fail "test port $port is already in use"

  write_test_config
  run_seed >/dev/null
  run_id="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
  make_server_command "$port" "$run_id"
  : > "$SERVER_LOG"
  chmod 600 "$SERVER_LOG"
  env -i "${SERVER_ENV[@]}" "$PYTHON" - "$PID_FILE" "$SERVER_LOG" "$RUN_ID_FILE" "$run_id" "${SERVER_CMD[@]}" <<'PY'
import os
import subprocess
import sys

pid_path, log_path, run_id_path, run_id, *command = sys.argv[1:]
with open(log_path, "ab", buffering=0) as log:
    process = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=log,
        stderr=subprocess.STDOUT,
        close_fds=True,
        start_new_session=True,
        env=os.environ.copy(),
    )
for path, value in ((pid_path, f"{process.pid}\n"), (run_id_path, f"{run_id}\n")):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="ascii") as stream:
        stream.write(value)
PY
  pid="$(read_pid)" || fail 'server launch did not produce a pid'
  printf '%s\n' "$port" > "$PORT_FILE"
  chmod 600 "$PORT_FILE"

  origin="http://127.0.0.1:$port"
  for attempt in $(seq 1 90); do
    launch_url="$(extract_launch_url "$port")"
    if [[ -n "$launch_url" ]]; then
      printf '%s\n' "$launch_url" > "$LAUNCH_URL_FILE"
      chmod 600 "$LAUNCH_URL_FILE"
    fi
    if [[ -n "$launch_url" ]] && curl -fsS "$origin/api/health" -o /dev/null; then
      printf '%s\n' "$origin"
      return
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      clear_runtime_files
      fail 'server exited before becoming healthy; see the private server log'
    fi
    sleep 1
  done

  stop_server >/dev/null
  fail 'server did not become healthy within 90 seconds; see the private server log'
}

show_status() {
  local pid port origin
  if ! pid="$(read_pid)" || ! owns_server_group "$pid"; then
    printf 'stopped\n'
    return
  fi
  if [[ ! -r "$PORT_FILE" ]]; then
    printf 'running (port unavailable)\n'
    return 1
  fi
  IFS= read -r port < "$PORT_FILE"
  origin="http://127.0.0.1:$port"
  if curl -fsS "$origin/api/health" -o /dev/null; then
    printf 'running %s\n' "$origin"
  else
    printf 'starting %s\n' "$origin"
    return 1
  fi
}

verify_isolation() {
  local pid
  pid="$(read_pid)" || fail 'test server is not running'
  owns_server_group "$pid" || fail 'pid does not identify this test server process group'

  python3 - "$pid" "$STATE_DIR/home" "$STATE_DIR/hermes" "$STATE_DIR/runtime" <<'PY'
import os
import sys
from pathlib import Path

target_pgid = int(sys.argv[1])
expected = {
    "HOME": sys.argv[2],
    "HERMES_HOME": sys.argv[3],
    "HERMES_RUNTIME_DIR": sys.argv[4],
}
real_hermes_home = (Path.home() / ".hermes").resolve(strict=False)
environment_keys = tuple(expected)
processes = []
failures = []

for entry in os.listdir("/proc"):
    if not entry.isdigit():
        continue
    pid = int(entry)
    try:
        if os.getpgid(pid) == target_pgid:
            processes.append(pid)
    except (ProcessLookupError, PermissionError):
        continue

if not processes:
    print("ISOLATION_FAILURE no_processes_in_server_group")
    raise SystemExit(1)

for pid in sorted(processes):
    try:
        raw_environment = Path(f"/proc/{pid}/environ").read_bytes()
    except FileNotFoundError:
        continue
    except PermissionError:
        values = {key: "<unreadable>" for key in environment_keys}
        print(f"pid={pid} " + " ".join(f"{key}={values[key]}" for key in environment_keys))
        failures.append(f"pid={pid} environment_unreadable")
        continue

    environment = {}
    for item in raw_environment.split(b"\0"):
        if b"=" not in item:
            continue
        key, value = item.split(b"=", 1)
        decoded_key = key.decode("ascii", errors="ignore")
        if decoded_key in expected:
            environment[decoded_key] = value.decode("utf-8", errors="replace")

    print(
        f"pid={pid} "
        + " ".join(f"{key}={environment.get(key, '<unset>')}" for key in environment_keys)
    )
    for key, expected_value in expected.items():
        if environment.get(key) != expected_value:
            failures.append(f"pid={pid} {key}_not_isolated")

    fd_dir = Path(f"/proc/{pid}/fd")
    found_fds = 0
    try:
        fd_names = os.listdir(fd_dir)
    except FileNotFoundError:
        continue
    except PermissionError:
        failures.append(f"pid={pid} fd_directory_unreadable")
        continue

    for fd in sorted(fd_names, key=lambda value: int(value) if value.isdigit() else -1):
        if not fd.isdigit():
            continue
        try:
            target = os.readlink(fd_dir / fd)
        except (FileNotFoundError, PermissionError):
            continue
        if target.endswith(" (deleted)"):
            target = target[:-10]
        try:
            resolved_target = Path(target).resolve(strict=False)
            relative_target = resolved_target.relative_to(real_hermes_home)
        except (OSError, ValueError):
            continue

        try:
            fdinfo = Path(f"/proc/{pid}/fdinfo/{fd}").read_text(encoding="ascii")
        except (FileNotFoundError, PermissionError):
            failures.append(f"pid={pid} fd={fd} fdinfo_unreadable")
            continue
        flags_line = next((line for line in fdinfo.splitlines() if line.startswith("flags:")), None)
        if flags_line is None:
            failures.append(f"pid={pid} fd={fd} flags_missing")
            continue
        try:
            flags = int(flags_line.split()[1], 8)
        except (IndexError, ValueError):
            failures.append(f"pid={pid} fd={fd} flags_invalid")
            continue

        if getattr(os, "O_PATH", 0) and flags & os.O_PATH:
            mode = "path-only"
        else:
            access_mode = flags & os.O_ACCMODE
            mode = {
                os.O_RDONLY: "read-only",
                os.O_WRONLY: "write-only",
                os.O_RDWR: "read-write",
            }.get(access_mode, "unknown")
        found_fds += 1
        print(f"pid={pid} fd={fd} mode={mode} path={relative_target}")
        if mode in {"write-only", "read-write", "unknown"}:
            failures.append(f"pid={pid} fd={fd} writable_or_unknown_mode")

    if not found_fds:
        print(f"pid={pid} fds-under-real-hermes=none")

if failures:
    for failure in failures:
        print(f"ISOLATION_FAILURE {failure}")
    raise SystemExit(1)

print("isolation=pass")
PY
}

show_command() {
  local requested_port="${1:-}" port python run_id
  require_pin
  python="$(resolve_python)"
  server_env "$python"
  if [[ -n "$requested_port" ]]; then
    validate_port "$requested_port"
    port="$requested_port"
  elif [[ -r "$PORT_FILE" ]]; then
    IFS= read -r port < "$PORT_FILE"
    validate_port "$port"
  else
    port="$(choose_port)"
  fi
  run_id="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
  make_server_command "$port" "$run_id"
  env -i "${SERVER_ENV[@]}" "$python" -c 'import json, sys; print(json.dumps(sys.argv[1:]))' "${SERVER_CMD[@]}"
}

seed_demo() {
  require_pin
  local python
  python="$(resolve_python)"
  server_env "$python"
  ensure_state
  write_test_config
  run_seed
}

reset_state() {
  case "$STATE_DIR" in
    "$DEFAULT_STATE_DIR"|"$DEFAULT_STATE_DIR"-*|"$DEFAULT_STATE_DIR"/*) ;;
    *) fail 'refusing to reset an unexpected path' ;;
  esac
  if [[ -e "$PID_FILE" ]]; then
    stop_server >/dev/null
  fi
  rm -rf -- "$STATE_DIR"
  printf 'reset\n'
}

case "${1:-}" in
  start)
    (($# <= 2)) || fail 'usage: test-hermes.sh start [port]'
    shift || true
    start_server "${1:-}"
    ;;
  stop)
    (($# == 1)) || fail 'usage: test-hermes.sh stop'
    stop_server
    ;;
  status)
    (($# == 1)) || fail 'usage: test-hermes.sh status'
    show_status
    ;;
  verify-isolation)
    (($# == 1)) || fail 'usage: test-hermes.sh verify-isolation'
    verify_isolation
    ;;
  url)
    (($# == 1)) || fail 'usage: test-hermes.sh url'
    [[ -r "$LAUNCH_URL_FILE" ]] || fail 'launch link is unavailable; start the test Hermes first'
    cat -- "$LAUNCH_URL_FILE"
    ;;
  cmd)
    (($# <= 2)) || fail 'usage: test-hermes.sh cmd [port]'
    shift || true
    show_command "${1:-}"
    ;;
  seed)
    (($# == 1)) || fail 'usage: test-hermes.sh seed'
    seed_demo
    ;;
  reset)
    (($# == 1)) || fail 'usage: test-hermes.sh reset'
    reset_state
    ;;
  *)
    fail 'usage: test-hermes.sh {start [port]|stop|status|verify-isolation|url|cmd [port]|seed|reset}'
    ;;
esac

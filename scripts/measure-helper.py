#!/usr/bin/env python3
"""Process-tree RAM and window-startup measurements behind scripts/measure.sh (stdlib only).

Each measured command runs under its own Xvfb (xvfb-run) in a new session. The shell's processes
are the command's descendants plus that session's members, minus the xvfb-run wrapper and the
display server. Processes matching --pid-pattern are backend processes: reported apart, never
counted as shell.
"""

import argparse
import json
import os
import re
import shutil
import signal
import stat
import statistics
import struct
import subprocess
import sys
import tempfile
import time
import zlib

SCREEN = "1440x900x24"
SAMPLE_GRID = (32, 20)
MIN_WINDOW = 200
MIB = 1024 * 1024
DISPLAY_RE = re.compile(r"^(\S*/)?Xvfb( |$)")


def fail(message):
    sys.stderr.write(f"measure: {message}\n")
    raise SystemExit(1)


def read_env_file(path):
    env = {}
    with open(path, encoding="utf-8") as stream:
        for number, raw in enumerate(stream, 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            key, sep, value = line.partition("=")
            if not sep or not key.isidentifier():
                fail(f"malformed line {number} in env file (content withheld)")
            env[key] = value
    if "PATH" not in env:
        fail("env file must set PATH")
    return env


def process_table():
    table = {}
    for name in os.listdir("/proc"):
        if not name.isdigit():
            continue
        pid = int(name)
        try:
            with open(f"/proc/{pid}/stat", encoding="utf-8", errors="replace") as stream:
                stat = stream.read()
            fields = stat[stat.rindex(")") + 2:].split()
            if fields[0] == "Z":
                continue
            with open(f"/proc/{pid}/cmdline", "rb") as stream:
                cmd = stream.read().replace(b"\0", b" ").decode("utf-8", "replace").strip()
        except (OSError, ValueError, IndexError):
            continue
        table[pid] = {"ppid": int(fields[1]), "sid": int(fields[3]), "cmd": cmd}
    return table


def shell_pids(table, root, session):
    if root not in table:
        return set()
    children = {}
    for pid, info in table.items():
        children.setdefault(info["ppid"], []).append(pid)
    excluded = set()
    cursor = table[root]["ppid"]
    while cursor in table and cursor not in excluded:
        excluded.add(cursor)
        cursor = table[cursor]["ppid"]
    members = set()
    stack = [root]
    while stack:
        pid = stack.pop()
        if pid in members or pid not in table:
            continue
        members.add(pid)
        stack.extend(children.get(pid, ()))
    members |= {pid for pid, info in table.items() if info["sid"] == session}
    display = {pid for pid, info in table.items() if DISPLAY_RE.match(info["cmd"])}
    return members - excluded - display


def read_memory(pid):
    pss = rss = 0
    with open(f"/proc/{pid}/smaps_rollup", encoding="ascii", errors="replace") as stream:
        for line in stream:
            if line.startswith("Pss:"):
                pss = int(line.split()[1])
            elif line.startswith("Rss:"):
                rss = int(line.split()[1])
    return pss, rss


def process_kind(cmd):
    match = re.search(r"--type=(\S+)", cmd)
    if match:
        return match.group(1)
    return os.path.basename(cmd.split(" ", 1)[0]) or "main"


def socket_summary(pids):
    out = subprocess.run(["ss", "-H", "-tanp"], capture_output=True, text=True, check=False).stdout
    loopback, listening, external = {}, {}, 0
    for line in out.splitlines():
        fields = line.split()
        if len(fields) < 5:
            continue
        owners = {int(pid) for pid in re.findall(r"pid=(\d+)", line)}
        if not owners & pids:
            continue
        state, local, peer = fields[0], fields[3], fields[4]
        if state == "LISTEN":
            listening[local] = listening.get(local, 0) + 1
        elif peer.startswith(("127.0.0.1:", "[::1]:")):
            loopback[peer] = loopback.get(peer, 0) + 1
        else:
            external += 1
    return {"loopback_peers": loopback, "listening": listening, "external_connections": external}


def take_sample(root, session, pattern):
    table = process_table()
    pids = shell_pids(table, root, session)
    backend = {pid for pid in pids if pattern and pattern.search(table[pid]["cmd"])}
    sample = {"pss": 0, "rss": 0, "count": 0, "kinds": {}, "backend_pss": 0, "backend_rss": 0, "backend_count": len(backend)}
    for pid in pids - backend:
        try:
            pss, rss = read_memory(pid)
        except OSError:
            continue
        kind = process_kind(table[pid]["cmd"])
        sample["pss"] += pss
        sample["rss"] += rss
        sample["count"] += 1
        sample["kinds"][kind] = sample["kinds"].get(kind, 0) + pss
    for pid in backend:
        try:
            pss, rss = read_memory(pid)
        except OSError:
            continue
        sample["backend_pss"] += pss
        sample["backend_rss"] += rss
    sample["pids"] = pids - backend
    return sample


def mib(kib):
    return round(kib / 1024, 1)


def summarize(samples):
    kinds = sorted({kind for sample in samples for kind in sample["kinds"]})
    return {
        "samples": len(samples),
        "shell_pss_mb": mib(statistics.median(s["pss"] for s in samples)),
        "shell_rss_mb": mib(statistics.median(s["rss"] for s in samples)),
        "shell_processes": int(statistics.median(s["count"] for s in samples)),
        "by_kind_pss_mb": {
            kind: mib(statistics.median(s["kinds"].get(kind, 0) for s in samples)) for kind in kinds
        },
        "backend_processes": int(statistics.median(s["backend_count"] for s in samples)),
        "backend_pss_mb": mib(statistics.median(s["backend_pss"] for s in samples)),
        "backend_rss_mb": mib(statistics.median(s["backend_rss"] for s in samples)),
        "sample_shell_pss_mb": [mib(s["pss"]) for s in samples],
    }


def display_env(root):
    with open(f"/proc/{root}/environ", "rb") as stream:
        pairs = stream.read().split(b"\0")
    env = {}
    for item in pairs:
        key, sep, value = item.partition(b"=")
        if sep and key in (b"DISPLAY", b"XAUTHORITY"):
            env[key.decode()] = value.decode()
    if "DISPLAY" not in env:
        fail("command has no DISPLAY; it did not start under xvfb-run")
    return {**os.environ, **env}


def alive(pid):
    try:
        with open(f"/proc/{pid}/stat", encoding="utf-8", errors="replace") as stream:
            stat = stream.read()
    except OSError:
        return False
    return stat[stat.rindex(")") + 2] not in "ZX"


def start_command(command, env, log_path, pidfile):
    wrapper = [
        "xvfb-run", "-a", "-s", f"-screen 0 {SCREEN}",
        "bash", "-c", 'echo $$ > "$0"; exec "$@"', pidfile, *command,
    ]
    log = open(log_path, "ab")
    try:
        return subprocess.Popen(
            wrapper, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    finally:
        log.close()


def wait_for_root(wrapper, pidfile, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with open(pidfile, encoding="ascii") as stream:
                text = stream.read().strip()
        except OSError:
            text = ""
        if text.isdigit():
            return int(text)
        if wrapper.poll() is not None:
            fail("command wrapper exited before the command started (see the log)")
        time.sleep(0.005)
    fail("command did not start within the timeout")


def session_members(session):
    return {pid: info["cmd"] for pid, info in process_table().items() if info["sid"] == session}


def wait_until(done, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline and not done():
        time.sleep(0.1)
    return done()


def stop_tree(wrapper, session, root):
    table = process_table()
    targets = shell_pids(table, root, session) if root in table else set()
    for pid in targets:
        _signal(pid, signal.SIGTERM)
    wait_until(lambda: not any(alive(pid) for pid in targets), 5)
    for pid in targets:
        if alive(pid):
            _signal(pid, signal.SIGKILL)
    try:
        wrapper.wait(timeout=15)
    except subprocess.TimeoutExpired:
        _signal(wrapper.pid, signal.SIGKILL)
        wrapper.wait(timeout=15)
    for pid in session_members(session):
        _signal(pid, signal.SIGTERM)
    if not wait_until(lambda: not session_members(session), 5):
        for pid in session_members(session):
            _signal(pid, signal.SIGKILL)
        wait_until(lambda: not session_members(session), 5)
    return [os.path.basename(cmd.split(" ", 1)[0]) for cmd in session_members(session).values()]


def _signal(pid, sig):
    try:
        os.kill(pid, sig)
    except ProcessLookupError:
        pass


def window_search(env, pid):
    out = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--pid", str(pid)],
        env=env, capture_output=True, text=True, check=False,
    ).stdout
    return [int(wid) for wid in out.split() if wid.isdigit()]


def descendant_pids(root):
    table = process_table()
    children = {}
    for pid, info in table.items():
        children.setdefault(info["ppid"], []).append(pid)
    found = set()
    stack = [root]
    while stack:
        pid = stack.pop()
        if pid in found or pid not in table:
            continue
        found.add(pid)
        stack.extend(children.get(pid, ()))
    return found


def window_ids(env, root):
    ids = window_search(env, root)
    if ids:
        return ids
    # The measured command may be a wrapper (e.g. dbus-run-session) that forks the
    # real X11 client, so the wrapper's pid is not the window's _NET_WM_PID. Search
    # the wrapper's process tree instead. Unrelated windows (e.g. the Xvfb root) are
    # never matched.
    ids = []
    for pid in sorted(descendant_pids(root)):
        ids.extend(window_search(env, pid))
    return ids


def window_box(env, wid):
    out = subprocess.run(
        ["xdotool", "getwindowgeometry", "--shell", str(wid)],
        env=env, capture_output=True, text=True, check=True,
    ).stdout
    values = dict(line.split("=", 1) for line in out.splitlines() if "=" in line)
    return int(values["X"]), int(values["Y"]), int(values["WIDTH"]), int(values["HEIGHT"])


def main_window(env, root):
    best = None
    for wid in window_ids(env, root):
        x, y, width, height = window_box(env, wid)
        if width >= MIN_WINDOW and height >= MIN_WINDOW and (best is None or width * height > best[1]):
            best = ((x, y), width * height, wid)
    return best


def click_window(env, root, rel_x, rel_y):
    found = main_window(env, root)
    if found is None:
        fail("no visible main window to click in")
    (x, y), _area, _wid = found
    subprocess.run(
        ["xdotool", "mousemove", "--sync", str(x + rel_x), str(y + rel_y), "click", "1"],
        env=env, check=True,
    )


def xwd_bytes(env, *args):
    return subprocess.run(["xwd", "-silent", *args], env=env, capture_output=True, check=True).stdout


def parse_xwd(data):
    fields = struct.unpack(">25I", data[:100])
    header_size, width, height = fields[0], fields[4], fields[5]
    byte_order, bits_per_pixel, stride, ncolors = fields[7], fields[11], fields[12], fields[19]
    # Xvfb's root capture reports 24 bits per pixel while any window is mapped
    # (and 32 while none is), but it stores four bytes per pixel either way:
    # bytes_per_line == width * 4. The header's depth is therefore not the
    # stride, and both depths decode with the same code below.
    if bits_per_pixel not in (24, 32) or stride != width * 4:
        fail(f"unsupported XWD format ({bits_per_pixel} bits per pixel, stride {stride} for width {width})")
    start = header_size + ncolors * 12
    order = "little" if byte_order == 0 else "big"
    masks = (fields[14], fields[15], fields[16])
    return width, height, stride, data[start:start + height * stride], order, masks


def is_painted(xwd):
    width, height, stride, image, order, _masks = parse_xwd(xwd)
    colors = set()
    cols, rows = SAMPLE_GRID
    for gy in range(rows):
        y = gy * (height - 1) // (rows - 1)
        for gx in range(cols):
            x = gx * (width - 1) // (cols - 1)
            index = y * stride + x * 4
            colors.add(int.from_bytes(image[index:index + 4], order) & 0xFFFFFF)
            if len(colors) >= 2:
                return True
    return False


def xwd_to_png(xwd, out_path):
    width, height, stride, image, order, masks = parse_xwd(xwd)
    shifts = [(mask & -mask).bit_length() - 1 for mask in masks]
    raw = bytearray()
    for y in range(height):
        row = image[y * stride:y * stride + width * 4]
        rgb = bytearray(width * 3)
        for channel, (mask, shift) in enumerate(zip(masks, shifts)):
            component = bytearray(width)
            for x in range(width):
                component[x] = (int.from_bytes(row[x * 4:x * 4 + 4], order) & mask) >> shift
            rgb[channel::3] = component
        raw.append(0)
        raw += rgb

    def chunk(tag, body):
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)

    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 6))
        + chunk(b"IEND", b"")
    )
    with open(out_path, "wb") as stream:
        stream.write(png)


def run_measured(args, measure):
    env = read_env_file(args.env_file) if args.env_file else dict(os.environ)
    log_path = args.log or tempfile.mkstemp(prefix="measure-", suffix=".log")[1]
    workdir = tempfile.mkdtemp(prefix="measure-run-")
    pidfile = os.path.join(workdir, "pid")
    wrapper = start_command(args.command, env, log_path, pidfile)
    try:
        root = wait_for_root(wrapper, pidfile, 60)
        measure(wrapper, root)
    finally:
        leftovers = stop_tree(wrapper, wrapper.pid, _root_or_none(workdir))
        shutil.rmtree(workdir, ignore_errors=True)
    return log_path, leftovers


def _root_or_none(workdir):
    try:
        with open(os.path.join(workdir, "pid"), encoding="ascii") as stream:
            return int(stream.read().strip())
    except (OSError, ValueError):
        return -1


def ram(args):
    pattern = re.compile(args.pid_pattern) if args.pid_pattern else None
    results = []

    def measure(wrapper, root):
        time.sleep(args.settle)
        if not alive(root):
            fail("command exited during the settle time (see the log)")
        env = display_env(root)
        samples = []
        for index in range(args.samples):
            samples.append(take_sample(root, wrapper.pid, pattern))
            if index + 1 < args.samples:
                time.sleep(args.interval)
        sockets = socket_summary(samples[-1]["pids"])
        results.append(("idle", samples, sockets))
        if args.click_window:
            for spec in args.click_window:
                rel_x, rel_y = (int(v) for v in spec.split(","))
                click_window(env, root, rel_x, rel_y)
                time.sleep(1)
            time.sleep(args.settle)
            samples = [take_sample(root, wrapper.pid, pattern) for _ in range(args.samples)]
            results.append(("after_clicks", samples, socket_summary(samples[-1]["pids"])))
        if args.screenshot:
            xwd_to_png(xwd_bytes(env, "-root"), args.screenshot)

    log_path, leftovers = run_measured(args, measure)
    for phase, samples, sockets in results:
        summary = summarize(samples)
        record = {
            "measure": "ram", "label": args.label, "phase": phase, "settle_s": args.settle,
            "interval_s": args.interval, "screen": SCREEN, "sockets": sockets, **summary,
        }
        print(json.dumps(record, sort_keys=True))
        sys.stderr.write(
            f"{args.label} RAM {phase}: shell PSS {summary['shell_pss_mb']} MiB "
            f"(RSS {summary['shell_rss_mb']} MiB, {summary['shell_processes']} processes); "
            f"backend {summary['backend_pss_mb']} MiB excluded\n"
        )
    sys.stderr.write(f"log: {log_path}\n")
    if leftovers:
        fail(f"{len(leftovers)} process(es) survived the stop: {', '.join(leftovers)}")


def log_matches(path, offset, pattern):
    """True when the log grew past `offset` and holds `pattern` after it."""
    try:
        with open(path, "rb") as stream:
            stream.seek(offset)
            text = stream.read().decode("utf-8", "replace")
    except OSError:
        return False
    return re.search(pattern, text) is not None


def startup_run(args, index):
    if args.fresh_dir:
        shutil.rmtree(args.fresh_dir, ignore_errors=True)
        os.makedirs(args.fresh_dir, mode=0o700)
    env = read_env_file(args.env_file) if args.env_file else dict(os.environ)
    log_path = args.log or tempfile.mkstemp(prefix="measure-startup-", suffix=".log")[1]
    workdir = tempfile.mkdtemp(prefix="measure-run-")
    pidfile = os.path.join(workdir, "pid")
    # A fixed --log is appended to across runs, so a stop pattern must only count
    # lines this run wrote.
    log_offset = os.path.getsize(log_path) if args.stop_log and os.path.exists(log_path) else 0
    wrapper = start_command(args.command, env, log_path, pidfile)
    elapsed = None
    try:
        root = wait_for_root(wrapper, pidfile, args.timeout)
        started = time.monotonic()
        deadline = started + args.timeout
        env_display = None
        while time.monotonic() < deadline:
            if not alive(root):
                fail(f"run {index}: command exited before a window was painted (see the log)")
            if args.stop_log:
                # The command reports its own milestone; the window check cannot
                # see the difference between the splash and the app behind it.
                if log_matches(log_path, log_offset, args.stop_log):
                    elapsed = (time.monotonic() - started) * 1000
                    break
            else:
                if env_display is None:
                    env_display = display_env(root)
                for wid in window_ids(env_display, root):
                    _x, _y, width, height = window_box(env_display, wid)
                    if width < MIN_WINDOW or height < MIN_WINDOW:
                        continue
                    if is_painted(xwd_bytes(env_display, "-id", str(wid))):
                        elapsed = (time.monotonic() - started) * 1000
                        break
                if elapsed is not None:
                    break
            time.sleep(0.05)
    finally:
        leftovers = stop_tree(wrapper, wrapper.pid, _root_or_none(workdir))
        shutil.rmtree(workdir, ignore_errors=True)
    if elapsed is None:
        fail(f"run {index}: no painted window within {args.timeout}s (see the log)")
    if leftovers:
        fail(f"run {index}: {len(leftovers)} process(es) survived the stop: {', '.join(leftovers)}")
    return elapsed, log_path


def startup(args):
    runs, log_path = [], None
    for index in range(1, args.runs + 1):
        elapsed, log_path = startup_run(args, index)
        runs.append(round(elapsed, 1))
        sys.stderr.write(f"{args.label} startup run {index}: {elapsed:.0f} ms\n")
    record = {
        "measure": "startup", "label": args.label, "runs": args.runs, "run_ms": runs,
        "median_ms": round(statistics.median(runs), 1), "min_ms": min(runs), "max_ms": max(runs),
        "clock": "first observed command pid (written just before exec) to first painted window",
        "painted": f"visible window >= {MIN_WINDOW}x{MIN_WINDOW} whose XWD grid has >= 2 colors",
        "screen": SCREEN,
    }
    if args.stop_log:
        record["clock"] = (
            "first observed command pid (written just before exec) to the first line the "
            "command wrote matching --stop-log"
        )
        record["painted"] = f"the command's own log line matching {args.stop_log!r}"
    print(json.dumps(record, sort_keys=True))
    sys.stderr.write(f"{args.label} startup median {record['median_ms']} ms over {args.runs} runs\n")
    sys.stderr.write(f"log: {log_path}\n")


def split_command(argv):
    if "--" not in argv:
        return argv, []
    split = argv.index("--")
    return argv[:split], argv[split + 1:]


def tree_size(top):
    entries = [top]
    if os.path.isdir(top) and not os.path.islink(top):
        entries = [
            os.path.join(dirpath, name)
            for dirpath, dirnames, filenames in os.walk(top)
            for name in dirnames + filenames
        ]
    apparent = ondisk = files = 0
    for entry in entries:
        st = os.lstat(entry)
        if stat.S_ISDIR(st.st_mode):
            continue
        apparent += st.st_size
        ondisk += st.st_blocks * 512
        files += 1
    return apparent, ondisk, files


def size(args):
    for path in args.paths:
        if not os.path.exists(path):
            fail(f"no such path: {os.path.basename(path)}")
        apparent, ondisk, files = tree_size(path)
        label = args.label or os.path.basename(os.path.normpath(path))
        record = {
            "measure": "size", "label": label, "files": files,
            "apparent_bytes": apparent, "ondisk_bytes": ondisk,
            "apparent_mb": round(apparent / MIB, 1), "ondisk_mb": round(ondisk / MIB, 1),
            "unit": "MB = MiB (1048576 bytes)",
        }
        print(json.dumps(record, sort_keys=True))
        sys.stderr.write(f"{label} size: {record['apparent_mb']} MB apparent, {record['ondisk_mb']} MB on disk, {files} files\n")


def build_parser():
    parser = argparse.ArgumentParser(prog="measure.sh")
    commands = parser.add_subparsers(dest="mode", required=True)

    size_parser = commands.add_parser("size")
    size_parser.add_argument("paths", nargs="+")
    size_parser.add_argument("--label")

    shared = argparse.ArgumentParser(add_help=False)
    shared.add_argument("--label", help="name in the output (default: basename of the command or path)")
    shared.add_argument("--env-file", help="KEY=VALUE lines; the command gets exactly these (plus Xvfb's DISPLAY)")
    shared.add_argument("--log", help="file for the command's stdout and stderr (default: a new file in TMPDIR)")

    ram_parser = commands.add_parser("ram", parents=[shared])
    ram_parser.add_argument("--pid-pattern", help="regex matching backend processes to report apart")
    ram_parser.add_argument("--settle", type=float, default=20.0)
    ram_parser.add_argument("--samples", type=int, default=5)
    ram_parser.add_argument("--interval", type=float, default=2.0)
    ram_parser.add_argument("--click-window", action="append", metavar="X,Y",
                            help="click at X,Y inside the main window, then measure again (repeatable)")
    ram_parser.add_argument("--screenshot", metavar="PNG")

    startup_parser = commands.add_parser("startup", parents=[shared])
    startup_parser.add_argument("--runs", type=int, default=3)
    startup_parser.add_argument("--timeout", type=float, default=120.0)
    startup_parser.add_argument("--fresh-dir", help="directory wiped and recreated before each run (must be under /tmp)")
    startup_parser.add_argument("--stop-log", metavar="REGEX",
                                help="stop at the first line the command writes to --log that matches REGEX "
                                     "(a shell that reports its own first frame) instead of at the first painted window")
    return parser


def main(argv):
    options, command = split_command(argv)
    args = build_parser().parse_args(options)
    if args.mode == "size":
        size(args)
        return
    if not command:
        fail("no command after --")
    args.command = command
    args.label = args.label or os.path.basename(command[0])
    if args.mode == "ram":
        if args.samples < 1 or args.settle < 0 or args.interval < 0:
            fail("samples must be >= 1 and times must be >= 0")
        ram(args)
    else:
        if args.runs < 1:
            fail("runs must be >= 1")
        if args.fresh_dir and not os.path.abspath(args.fresh_dir).startswith("/tmp/"):
            fail("--fresh-dir must be under /tmp")
        startup(args)


if __name__ == "__main__":
    main(sys.argv[1:])

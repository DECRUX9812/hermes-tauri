#!/usr/bin/env python3
"""Test fixture that imitates only the hermes webapp stdout/health contract.

It prints a launch link on stdout, answers GET /api/health with 503 until
--delay-ready seconds have passed and 200 afterwards. It is not Hermes; it lets
the sidecar client be tested against real processes and real sockets.

Diagnostics go to stderr as "[webapp_fixture] pid=N" and, with
--spawn-grandchild, "[webapp_fixture] grandchild pid=N".
"""

import argparse
import http.server
import os
import re
import secrets
import signal
import subprocess
import sys
import time

TOKEN_PATTERN = re.compile(r"^[A-Za-z0-9_-]{43}$")


def diag(message):
    print(f"[webapp_fixture] {message}", file=sys.stderr, flush=True)


def make_handler(ready_at, base_path):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path.split("?")[0] != f"{base_path}/api/health":
                self.send_response(404)
                self.end_headers()
                return
            ready = time.monotonic() >= ready_at
            self.send_response(200 if ready else 503)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def log_message(self, format, *args):
            pass

    return Handler


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--token")
    parser.add_argument("--delay-ready", type=float, default=0.0)
    parser.add_argument("--exit-early", action="store_true")
    parser.add_argument("--ignore-sigterm", action="store_true")
    parser.add_argument("--spawn-grandchild", action="store_true")
    parser.add_argument("--base-path", default="")
    parser.add_argument("--print-env", action="store_true")
    args = parser.parse_args()

    token = args.token or secrets.token_urlsafe(32)
    if not TOKEN_PATTERN.match(token):
        sys.exit("--token must be 43 characters of [A-Za-z0-9_-]")
    base_path = "/" + args.base_path.strip("/") if args.base_path.strip("/") else ""

    diag(f"pid={os.getpid()}")
    if args.print_env:
        for name in ("PYTHONHOME", "PYTHONPATH", "HERMES_HOME"):
            diag(f"{name}={os.environ.get(name, '<unset>')}")

    if args.ignore_sigterm:
        signal.signal(signal.SIGTERM, signal.SIG_IGN)

    if args.spawn_grandchild:
        grandchild = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(3600)"]
        )
        diag(f"grandchild pid={grandchild.pid}")

    if args.exit_early:
        diag("boot step 1 of 2")
        diag("boot step 2 of 2: failing on purpose")
        print(f"http://{args.host}:{args.port or 0}{base_path}/#hermes-session={token}", flush=True)
        sys.exit(3)

    handler = make_handler(time.monotonic() + args.delay_ready, base_path)
    server = http.server.ThreadingHTTPServer((args.host, args.port), handler)
    port = server.server_address[1]
    print(f"http://{args.host}:{port}{base_path}/#hermes-session={token}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()

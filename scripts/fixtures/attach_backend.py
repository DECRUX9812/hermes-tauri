#!/usr/bin/env python3
"""Fixture HTTP server for exercising the Tauri attach-only adapter."""

from __future__ import annotations

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


PAGE = b"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Hermes Tauri attach fixture</title>
  <style>
    :root { color-scheme: light dark; font: 16px system-ui, sans-serif; }
    body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: #f4f3f0; color: #282824; }
    main { width: min(640px, calc(100% - 48px)); padding: 36px; border: 1px solid #dedcd5; border-radius: 16px; background: white; }
    h1 { margin-top: 0; font-size: 24px; }
    p { line-height: 1.6; }
    code { color: #514a3d; }
    @media (prefers-color-scheme: dark) {
      body { background: #191a19; color: #e9e8e2; }
      main { border-color: #383934; background: #222321; }
      code { color: #d1c5a8; }
    }
  </style>
</head>
<body>
  <main>
    <h1>Hermes Tauri attach fixture</h1>
    <p>This fixture reproduces the browser bridge assignment path.</p>
    <div id="result">Installing browser bridge…</div>
  </main>
  <script>
    const fixtureBrowserNotify = async () => false;
    const fixtureBrowserOpenExternal = async () => undefined;
    if (!window.hermesDesktop) {
      const bridge = {
        notify: fixtureBrowserNotify,
        openExternal: fixtureBrowserOpenExternal
      };
      window.hermesDesktop = bridge;
      document.documentElement.dataset.hermesDesktopHost = 'browser';
    }
    const bridge = window.hermesDesktop;
    document.querySelector('#result').innerHTML = [
      `<p>typeof window.hermesDesktop.openExternal: <code>${typeof bridge.openExternal}</code></p>`,
      `<p>Native host: <code>${document.documentElement.dataset.hermesNativeHost || 'missing'}</code></p>`,
      `<p>Notify overlay native: <code>${bridge.notify !== fixtureBrowserNotify ? 'yes' : 'no'}</code></p>`
    ].join('');
  </script>
</body>
</html>
"""


class FixtureHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path == "/api/health":
            body = b'{"status":"ok","fixture":true}'
            content_type = "application/json"
        elif self.path.startswith("/"):
            body = PAGE
            content_type = "text/html; charset=utf-8"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), FixtureHandler)
    print(f"fixture listening at http://127.0.0.1:{args.port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()

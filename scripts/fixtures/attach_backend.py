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
    <div id="result">Installing browser bridge...</div>
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
      `<p>Notify overlay native: <code>${bridge.notify !== fixtureBrowserNotify ? 'yes' : 'no'}</code></p>`,
      `<p>selectPaths overlay: <code>${typeof bridge.selectPaths}</code></p>`,
      `<p>requestIdleCallback: <code>${typeof requestIdleCallback === 'function' ? 'function' : typeof requestIdleCallback}</code></p>`,
      `<p>cancelIdleCallback: <code>${typeof cancelIdleCallback === 'function' ? 'function' : typeof cancelIdleCallback}</code></p>`,
      `<p>notify() result: <code id="notify-result">pending</code></p>`,
      `<p>openExternal() result: <code id="open-external-result">pending</code></p>`,
      `<p>selectPaths() result: <code id="select-paths-result">pending</code></p>`,
      `<p>popup: <code id="popup-result">pending</code></p>`,
      `<p>srcdoc iframe loaded: <code id="srcdoc-state">pending</code></p>`,
      `<p>same-origin iframe loaded: <code id="same-origin-state">pending</code></p>`
    ].join('');
    // Exercises the shell notification path end to end: the adapter overlays
    // hermesDesktop.notify, the command calls the notification plugin, and
    // the boolean the bridge returns is displayed here.
    bridge.notify({ title: 'Hermes Tauri', body: 'Fixture notification' })
      .then(result => {
        document.querySelector('#notify-result').textContent = String(result);
      })
      .catch(error => {
        document.querySelector('#notify-result').textContent = `error: ${error}`;
      });
    // Exercises the native open-external command. The command round-trip is
    // the point: whatever the OS does with the URL comes back as a result or
    // a rejection and is displayed.
    bridge.openExternal('https://example.com/')
      .then(() => {
        document.querySelector('#open-external-result').textContent = 'resolved';
      })
      .catch(error => {
        document.querySelector('#open-external-result').textContent = `rejected: ${error}`;
      });
    // Exercises the native file-dialog command; the test closes the dialog
    // with Escape and the cancel result is displayed.
    bridge.selectPaths({ title: 'Fixture picker' })
      .then(paths => {
        document.querySelector('#select-paths-result').textContent = JSON.stringify(paths);
      })
      .catch(error => {
        document.querySelector('#select-paths-result').textContent = `error: ${error}`;
      });
    // Exercises the guarded-popup path: a blob: preview on this origin must
    // open as a shell preview window, and a foreign link inside it must not
    // navigate that window.
    const previewSource = `<!doctype html><html><body style="font: 32px sans-serif; display: grid; place-items: center; min-height: 100vh; margin: 0">
      <a href="https://example.com/" style="padding: 48px; border: 4px solid #555">PREVIEW BLOB - go foreign</a>
    </body></html>`;
    const blobUrl = URL.createObjectURL(new Blob([previewSource], { type: 'text/html' }));
    window.open(blobUrl, '_blank', 'noopener,noreferrer');
    document.querySelector('#popup-result').textContent = `requested ${blobUrl.slice(0, 24)}…`;
    const probeFrame = (stateId, configure) => {
      const frame = document.createElement('iframe');
      frame.hidden = true;
      configure(frame);
      frame.addEventListener('load', () => {
        document.querySelector(`#${stateId}`).textContent = 'yes';
      });
      document.body.append(frame);
    };
    probeFrame('srcdoc-state', frame => { frame.srcdoc = '<p>frame</p>'; });
    probeFrame('same-origin-state', frame => { frame.src = '/api/health'; });
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

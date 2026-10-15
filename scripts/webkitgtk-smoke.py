#!/usr/bin/env python3
"""Drive the host WebKitGTK webview and capture Hermes route evidence.

The launch URL is read from WEBKIT_SMOKE_URL and is never printed.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2

ROUTES = (
    ("home", "#/"),
    ("team", "#/team"),
    ("kanban", "#/kanban"),
    ("studio", "#/studio"),
    ("settings", "#/settings"),
)
ROUTE_WAIT_MS = {"studio": 6000}

launch_url = os.environ.get("WEBKIT_SMOKE_URL", "")
if not launch_url:
    print("Set WEBKIT_SMOKE_URL to the private Hermes launch link.", file=sys.stderr)
    raise SystemExit(2)

output_dir = Path(os.environ.get("WEBKIT_SMOKE_OUTPUT", "/tmp/hermes-tauri-t8"))
output_dir.mkdir(parents=True, exist_ok=True)
os.environ.setdefault("WEBKIT_DISABLE_DMABUF_RENDERER", "1")

summary = {
    "runner": "host WebKitGTK",
    "version": "{}.{}.{}".format(
        WebKit2.get_major_version(),
        WebKit2.get_minor_version(),
        WebKit2.get_micro_version(),
    ),
    "screenshots": str(output_dir),
    "routes": {name: {"console": [], "page": []} for name, _hash in ROUTES},
    "boot": {"console": [], "page": []},
}
current_route = "boot"


def redact(value: object) -> str:
    text = str(value)
    text = re.sub(r"hermes-session=[A-Za-z0-9._~-]+", "hermes-session=<redacted>", text)
    return re.sub(r"ticket=[^&\s]+", "ticket=<redacted>", text)


def on_script_message(_manager: WebKit2.UserContentManager, result: object) -> None:
    try:
        payload = json.loads(result.get_js_value().to_string())
        kind = "console" if payload.get("kind") == "console" else "page"
        summary["routes"].setdefault(current_route, summary[current_route])[kind].append(
            redact(payload.get("text", ""))
        )
    except Exception as exc:  # The smoke result must not depend on message parsing.
        summary["boot"]["page"].append(f"smoke message parse failed: {type(exc).__name__}")


view = WebKit2.WebView()
window = Gtk.Window(title="Hermes WebKit smoke")
window.set_default_size(1440, 960)
window.add(view)
window.show_all()

content = view.get_user_content_manager()
content.register_script_message_handler("hermesSmoke")
content.connect("script-message-received::hermesSmoke", on_script_message)
content.add_script(
    WebKit2.UserScript(
        """
        (() => {
          const send = (kind, text) => {
            try {
              window.webkit.messageHandlers.hermesSmoke.postMessage(JSON.stringify({ kind, text: String(text) }));
            } catch (_error) {}
          };
          const originalError = console.error.bind(console);
          console.error = (...args) => {
            send('console', args.map(String).join(' '));
            originalError(...args);
          };
          window.addEventListener('error', event => {
            send('page', `${event.message} @ ${event.filename}:${event.lineno}:${event.colno}`);
          });
          window.addEventListener('unhandledrejection', event => {
            send('page', `unhandledrejection: ${event.reason}`);
          });
        })();
        """,
        WebKit2.UserContentInjectedFrames.TOP_FRAME,
        WebKit2.UserScriptInjectionTime.START,
    )
)


def screenshot(route: str) -> None:
    target = output_dir / f"webkit-{route}.png"
    subprocess.run(["scrot", "-z", "-o", str(target)], check=True)
    summary["routes"][route]["screenshot"] = target.name


def run_javascript(source: str, done) -> None:
    def finished(_view: WebKit2.WebView, result: object) -> None:
        value = None
        try:
            js_value = _view.run_javascript_finish(result).get_js_value()
            value = js_value.to_string() if not js_value.is_undefined() else None
        except Exception as exc:
            summary["routes"].setdefault(current_route, summary[current_route])["page"].append(
                f"smoke javascript failed: {type(exc).__name__}: {redact(exc)}"
            )
        done(value)

    view.run_javascript(source, None, finished)


def route_step(index: int = 0) -> bool:
    global current_route
    if index >= len(ROUTES):
        Gtk.main_quit()
        return False

    current_route, route_hash = ROUTES[index]
    summary["routes"].setdefault(current_route, {"console": [], "page": []})

    def after_hash(_value: str | None = None) -> None:
        GLib.timeout_add(ROUTE_WAIT_MS.get(current_route, 1500), lambda: capture_step(index))

    run_javascript(f"window.location.hash = {json.dumps(route_hash)}", after_hash)
    return False


def finish_capture(index: int) -> None:
    try:
        screenshot(current_route)
    except Exception as exc:
        summary["routes"][current_route]["page"].append(
            f"screenshot failed: {type(exc).__name__}: {redact(exc)}"
        )
    route_step(index + 1)


def capture_step(index: int) -> bool:
    if current_route != "studio":
        finish_capture(index)
        return False

    def store_diagnostics(value: str | None) -> None:
        try:
            summary["routes"][current_route]["diagnostics"] = json.loads(value) if value else {}
        except json.JSONDecodeError:
            summary["routes"][current_route]["diagnostics"] = {"error": "result was not JSON"}
        finish_capture(index)

    run_javascript(
        """
        JSON.stringify((() => {
          const webglCanvas = document.createElement('canvas');
          const images = [...document.images];
          return {
            bootVisible: Boolean(document.querySelector('.ac-boot')),
            bootText: document.querySelector('.ac-boot')?.textContent ?? null,
            errorText: document.querySelector('.ac-error')?.textContent ?? null,
            webgl: Boolean(webglCanvas.getContext('webgl')),
            webgl2: Boolean(document.createElement('canvas').getContext('webgl2')),
            imageCount: images.length,
            incompleteImageCount: images.filter(image => !image.complete).length,
            resourceCount: performance.getEntriesByType('resource').length,
          };
        })());
        """,
        store_diagnostics,
    )
    return False


def after_onboarding_click(value: str | None = None) -> None:
    try:
        summary["onboarding"] = json.loads(value) if value else {"found": 0, "visible": 0, "clicked": False}
    except json.JSONDecodeError:
        summary["onboarding"] = {"error": "result was not JSON"}

    GLib.timeout_add(1000, route_step, 0)


def initial_page_ready() -> bool:
    run_javascript(
        """
        JSON.stringify((() => {
          const matches = [...document.querySelectorAll('button')]
            .filter(item => item.textContent.trim() === "I'll choose a provider later");
          const visible = matches.filter(item => item.getClientRects().length > 0);
          const button = visible.at(-1) ?? matches.at(-1);
          if (button) { button.click(); }
          return { found: matches.length, visible: visible.length, clicked: Boolean(button) };
        })());
        """,
        after_onboarding_click,
    )
    return False


boot_started = False


def on_load_changed(_view: WebKit2.WebView, event: WebKit2.LoadEvent) -> None:
    global boot_started
    if event == WebKit2.LoadEvent.FINISHED and not boot_started:
        boot_started = True
        GLib.timeout_add(8000, initial_page_ready)


view.connect("load-changed", on_load_changed)
view.load_uri(launch_url)
GLib.timeout_add_seconds(90, Gtk.main_quit)
Gtk.main()
print(json.dumps(summary, indent=2))

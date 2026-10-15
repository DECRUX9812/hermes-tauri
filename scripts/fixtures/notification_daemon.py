#!/usr/bin/env python3
"""Minimal D-Bus notification daemon fixture (org.freedesktop.Notifications).

Implements the parts of the freedesktop notifications spec the shell uses, so
tests under Xvfb + dbus-run-session can prove the shell's notifications really
reach a daemon. Every Notify() call is printed to stdout as JSON. This is a
fixture daemon: it replaces a desktop environment's daemon, not the plugin.
"""

from __future__ import annotations

import json

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

IFACE = "org.freedesktop.Notifications"
PATH = "/org/freedesktop/Notifications"


class FixtureDaemon(dbus.service.Object):
    def __init__(self, bus):
        super().__init__(bus, PATH)

    @dbus.service.method(IFACE, in_signature="susssasa{sv}i", out_signature="u")
    def Notify(self, app_name, replaces_id, app_icon, summary, body, actions, hints, timeout):
        print(
            json.dumps({
                "app_name": str(app_name),
                "summary": str(summary),
                "body": str(body),
            }),
            flush=True,
        )
        return dbus.UInt32(1)

    @dbus.service.method(IFACE, in_signature="u")
    def CloseNotification(self, notification_id):
        pass

    @dbus.service.method(IFACE, out_signature="as")
    def GetCapabilities(self):
        return dbus.Array(["body"], signature="s")

    @dbus.service.method(IFACE, in_signature="", out_signature="ssu")
    def GetServerInformation(self):
        return ("fixture-daemon", "fixture", 1)


def main() -> None:
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    # Keep the name object alive: if it is garbage collected, dbus-python
    # releases the well-known name and clients trigger dbus activation of the
    # desktop's (absent) notification daemon instead of reaching this fixture.
    name = dbus.service.BusName(IFACE, bus=bus)
    FixtureDaemon(bus)
    print("fixture notification daemon ready", flush=True)
    try:
        GLib.MainLoop().run()
    finally:
        del name


if __name__ == "__main__":
    main()

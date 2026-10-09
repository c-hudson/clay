#!/usr/bin/env python3
"""Linux counterpart of webview-snapshot.swift: renders Clay's web client in WebKitGTK -
the engine the Linux GUI uses - and saves a PNG. Run under a headless broadway display so
no window appears on the desktop:
    broadwayd :21 &
    GDK_BACKEND=broadway BROADWAY_DISPLAY=:21 GDK_SCALE=2 \\
        python3 webview-snapshot-gtk.py <url> <out.png> <password-sha256> [width height delay zoom]
width/height are CSS pixels; zoom 2 renders them at twice the resolution (broadway ignores
GDK_SCALE), matching a Retina capture or the native GUI's 2x offscreen snapshot.
Logs in as the GUI does: AUTO_PASSWORD (the password's SHA-256; loopback only).
Optional, for photographing a popup or an editor page:
    CLAY_WEB_SETUP  script run before the page's own (e.g. the GUI's POPUP_MODE or NOTE_MODE)
    CLAY_WEB_AFTER  script run halfway through the delay (type a command, click, log in)"""
import os
import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

url, out, pw_hash = sys.argv[1], sys.argv[2], sys.argv[3]
width = int(sys.argv[4]) if len(sys.argv) > 4 else 800
height = int(sys.argv[5]) if len(sys.argv) > 5 else 600
delay = float(sys.argv[6]) if len(sys.argv) > 6 else 8
zoom = float(sys.argv[7]) if len(sys.argv) > 7 else 1

# Broadway has no DPI of its own: without this every CSS length comes out wrong.
Gtk.Settings.get_default().props.gtk_xft_dpi = 96 * 1024

setup = (
    "window.WEBVIEW_MODE = true; window.WEBVIEW_DEVICE_OVERRIDE = 'desktop';"
    f"window.WS_PROTOCOL = 'ws'; window.AUTO_PASSWORD = '{pw_hash}';"
) + os.environ.get("CLAY_WEB_SETUP", "")
after = os.environ.get("CLAY_WEB_AFTER", "")
manager = WebKit2.UserContentManager()
manager.add_script(WebKit2.UserScript.new(
    setup, WebKit2.UserContentInjectedFrames.TOP_FRAME,
    WebKit2.UserScriptInjectionTime.START, None, None))
view = WebKit2.WebView.new_with_user_content_manager(manager)
window = Gtk.Window()
window.set_default_size(int(width * zoom), int(height * zoom))
view.set_zoom_level(zoom)
window.add(view)
window.show_all()
view.load_uri(url)


def snapshot():
    def done(webview, result):
        surface = webview.get_snapshot_finish(result)
        surface.write_to_png(out)
        print(f"webview-snapshot: {out} ({surface.get_width()}x{surface.get_height()})")
        Gtk.main_quit()
    view.get_snapshot(WebKit2.SnapshotRegion.VISIBLE, WebKit2.SnapshotOptions.NONE, None, done)
    return False


if after:
    GLib.timeout_add(int(delay * 500), lambda: view.evaluate_javascript(after, -1, None, None, None, None, None) and False)
GLib.timeout_add(int(delay * 1000), snapshot)
GLib.timeout_add(int((delay + 15) * 1000), lambda: (print("timed out", file=sys.stderr), sys.exit(1)))
Gtk.main()

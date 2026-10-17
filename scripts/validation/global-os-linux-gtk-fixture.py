#!/usr/bin/env python3
"""Disposable GTK native-window oracle for the canonical Linux Computer driver.

Only the Agent interacts with the window. The driver reads this fixture's own
state and native identity; it never edits the widget state or injects UI input.
"""
import argparse
import json
import os
from pathlib import Path
import re
import sys
import uuid

# This fixture exclusively targets the driver's private X11 server. GTK may
# otherwise prefer a default wayland-0 socket even without WAYLAND_DISPLAY.
os.environ["GDK_BACKEND"] = "x11"
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("GdkX11", "3.0")
from gi.repository import Gdk, GLib, Gtk

parser = argparse.ArgumentParser()
parser.add_argument("--root", required=True)
parser.add_argument("--token", required=True)
arguments = parser.parse_args()
token = str(uuid.UUID(arguments.token))
root = Path(arguments.root)
if not root.is_absolute() or root.name != "desktop" or not re.fullmatch(r"nomifun-global-os-product-[A-Za-z0-9-]+", root.parent.name):
    raise SystemExit("The native fixture requires its driver's unique /tmp/nomifun-global-os-product-*/desktop root")
if root.parent.parent != Path("/tmp") or root.exists() or root.is_symlink():
    raise SystemExit("The desktop fixture root must be absent under the driver's allocated /tmp root")
root.mkdir(mode=0o700)
state_path = root / "state.json"
title = f"NomiFun Linux Computer fixture {token}"
state = {"schema": "nomifun-linux-computer-fixture-v1", "token": token, "pid": os.getpid(),
         "title": title, "xid": None, "click_count": 0, "entry_text": "", "enter_count": 0,
         "scroll_value": 0.0, "closed": False, "widgets": {}, "sink_text": "",
         "clipboard_initial_empty": False, "clipboard_empty": False, "clipboard_matches_ax_value": False,
         "clipboard_restore_count": 0}

window = Gtk.Window(title=title)
clipboard = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
state["clipboard_initial_empty"] = clipboard.wait_for_text() is None
state["clipboard_empty"] = state["clipboard_initial_empty"]
ax_text = "NOMIFUN_AX_" + token.replace("-", "") + "_中文✓"
window.set_default_size(600, 480)
window.set_position(Gtk.WindowPosition.CENTER)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
box.set_border_width(24)
window.add(box)
heading = Gtk.Label(label="NomiFun native Computer acceptance fixture")
box.pack_start(heading, False, False, 0)
entry = Gtk.Entry()
entry.set_placeholder_text("Type the test marker here")
entry.get_accessible().set_name(f"NomiFun text {token}")
box.pack_start(entry, False, False, 0)
sink = Gtk.Entry()
sink.set_placeholder_text("Paste the copied private test marker here")
sink.get_accessible().set_name(f"NomiFun paste sink {token}")
box.pack_start(sink, False, False, 0)
button = Gtk.Button(label="NomiFun fixture action")
button.get_accessible().set_name(f"NomiFun action {token}")
box.pack_start(button, False, False, 0)
clear_clipboard = Gtk.Button(label="Restore the initial empty private clipboard")
clear_clipboard.get_accessible().set_name(f"NomiFun clear clipboard {token}")
box.pack_start(clear_clipboard, False, False, 0)
scrolled = Gtk.ScrolledWindow()
scrolled.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.ALWAYS)
scrolled.set_min_content_height(200)
scrolled.get_accessible().set_name("NomiFun fixture scroll area")
rows = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
for index in range(80):
    rows.pack_start(Gtk.Label(label=f"Controlled fixture row {index + 1}"), False, False, 0)
scrolled.add(rows)
box.pack_start(scrolled, True, True, 0)
status = Gtk.Label(label="Only this fixture accepts test input. Ctrl+Q closes it.")
box.pack_start(status, False, False, 0)

def widget_bounds(widget):
    native = window.get_window()
    if native is None:
        return None
    origin = native.get_origin()
    origin_x, origin_y = origin[-2:]
    point = widget.translate_coordinates(window, 0, 0)
    if point is None:
        return None
    allocation = widget.get_allocation()
    return {"x": int(origin_x + point[0]), "y": int(origin_y + point[1]),
            "width": int(allocation.width), "height": int(allocation.height)}

def publish(*_):
    native = window.get_window()
    if native is not None:
        try:
            gi.require_version("GdkX11", "3.0")
            from gi.repository import GdkX11
            state["xid"] = int(GdkX11.X11Window.get_xid(native))
        except (ValueError, TypeError, AttributeError):
            state["xid"] = None
        state["widgets"] = {name: widget_bounds(widget) for name, widget in
                            (("entry", entry), ("sink", sink), ("button", button), ("scroll", scrolled))}
    temporary = root / f".state-{os.getpid()}.tmp"
    descriptor = os.open(temporary, os.O_CREAT | os.O_WRONLY | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        json.dump(state, output, ensure_ascii=False)
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, state_path)
    return False

def changed(*_):
    state["entry_text"] = entry.get_text()
    publish()

def sink_changed(*_):
    state["sink_text"] = sink.get_text()
    publish()

def clipboard_observed(_, text, *__):
    state["clipboard_empty"] = text is None
    state["clipboard_matches_ax_value"] = text == ax_text
    publish()

def clipboard_changed(*_):
    clipboard.request_text(clipboard_observed, None)

def restore_clipboard(*_):
    if not state["clipboard_initial_empty"]:
        raise RuntimeError("A fresh private display must begin with an empty clipboard")
    state["clipboard_restore_count"] += 1
    clipboard.clear()
    clipboard.request_text(clipboard_observed, None)

def clicked(*_):
    state["click_count"] += 1
    status.set_text(f"Fixture action count: {state['click_count']}")
    publish()

def activated(*_):
    state["enter_count"] += 1
    publish()

def scroll_changed(adjustment):
    state["scroll_value"] = adjustment.get_value()
    publish()

def close(*_):
    state["closed"] = True
    publish()
    Gtk.main_quit()
    return False

def key_pressed(_, event):
    if event.state & Gdk.ModifierType.CONTROL_MASK and event.keyval in (Gdk.KEY_q, Gdk.KEY_Q):
        window.destroy()
        return True
    return False

entry.connect("changed", changed)
sink.connect("changed", sink_changed)
clipboard.connect("owner-change", clipboard_changed)
clear_clipboard.connect("clicked", restore_clipboard)
entry.connect("activate", activated)
button.connect("clicked", clicked)
scrolled.get_vadjustment().connect("value-changed", scroll_changed)
window.connect("key-press-event", key_pressed)
def configured(*_):
    GLib.idle_add(publish)
    return False

window.connect("configure-event", configured)
window.connect("destroy", close)
window.show_all()
entry.grab_focus()
GLib.timeout_add(100, publish)
print(json.dumps({"ready": True, "pid": os.getpid(), "title": title, "state_path": str(state_path)}), flush=True)
Gtk.main()

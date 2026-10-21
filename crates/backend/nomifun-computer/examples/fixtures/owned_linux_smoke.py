#!/usr/bin/env python3
"""Opt-in GTK3 fixture for owned_linux_smoke; no injected input or signals."""
import hashlib
import json
import os
import pathlib
import sys
import uuid

root = pathlib.Path(sys.argv[1]).resolve(strict=True)
token = str(uuid.UUID(sys.argv[2]))
if root.name != "nomifun-owned-linux-" + token:
    raise RuntimeError("unexpected owned root")
if (root / "fixture-sentinel").read_text() != token:
    raise RuntimeError("owned sentinel mismatch")
os.environ["GDK_BACKEND"] = "x11"

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib, GdkX11

def atomic(name, data):
    temporary = root / ("." + name + "-" + uuid.uuid4().hex)
    with temporary.open("xb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    temporary.replace(root / name)

def write(name, value):
    atomic(name, json.dumps(value, ensure_ascii=False).encode("utf-8"))

def start_time():
    return int(pathlib.Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()[19])

started = start_time()
marker = "NomiFun owned Linux " + token
private_root = str(root / "host-data")
expected_user_paths = json.loads((root / "expected-user-paths.json").read_text())
expected_environment = expected_user_paths["environment"]
if len(expected_environment) != 12:
    raise RuntimeError("expected exactly twelve user path fields")
original_user_paths_match = all(
    (list(os.environb[key.encode("ascii")]) if key.encode("ascii") in os.environb else None) == value
    for key, value in expected_environment.items())
original_cwd_match = list(os.getcwdb()) == expected_user_paths["cwd"]
environment = {
    "original_user_paths_match": original_user_paths_match,
    "original_cwd_match": original_cwd_match,
    "cua_keys_present": any(key.startswith("CUA_") for key in os.environ),
    "worker_private_root_present": any(private_root in value for value in os.environ.values()),
}
window = Gtk.Window(title=marker)
window.set_default_size(640, 440)
window.get_accessible().set_name(marker)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
box.set_border_width(20)
window.add(box)
left = Gtk.Entry()
right = Gtk.Entry()
product = Gtk.Entry()
product.set_editable(False)
product.set_text("0")
editor = Gtk.TextView()
editor.set_wrap_mode(Gtk.WrapMode.WORD_CHAR)
multiply = Gtk.Button(label="Multiply")
save = Gtk.Button(label="Save owned text")
for widget, name in ((left, "Owned left operand"), (right, "Owned right operand"),
                     (product, "Owned product"), (editor, "Owned editor"),
                     (multiply, "Multiply owned operands"), (save, "Save owned text")):
    widget.get_accessible().set_name(name)
for widget in (left, right, multiply, product):
    box.pack_start(widget, False, False, 0)
scroll = Gtk.ScrolledWindow()
scroll.set_min_content_height(130)
scroll.add(editor)
box.pack_start(scroll, True, True, 0)
box.pack_start(save, False, False, 0)
multiply_count = 0
save_count = 0
close_requested = False
health_nonce = None

def text():
    buffer = editor.get_buffer()
    return buffer.get_text(*buffer.get_bounds(), True)

def guard(callback):
    def wrapped(*args):
        try:
            return callback(*args)
        except Exception as error:
            write("fixture-error.json", {"pid": os.getpid(), "token": token,
                                         "error_type": type(error).__name__})
    return wrapped

@guard
def calculate(_button):
    global multiply_count
    multiply_count += 1
    a, b = int(left.get_text()), int(right.get_text())
    value = str(a * b)
    product.set_text(value)
    write("calculation.json", {"pid": os.getpid(), "token": token, "start_time": started,
                              "left": a, "right": b, "result": value,
                              "multiply_count": multiply_count})

@guard
def save_text(_button):
    global save_count
    save_count += 1
    if save_count != 1:
        raise RuntimeError("save must be dispatched once")
    data = text().encode("utf-8")
    atomic("saved.txt", data)
    write("saved.json", {"pid": os.getpid(), "token": token, "start_time": started,
                        "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                        "save_count": save_count})


def x11_metadata(owned_xid):
    """Read only the owned window's ancestry and root counts, never other titles."""
    import ctypes as c
    import ctypes.util
    pointer, ulong = c.c_void_p, c.c_ulong
    class Attributes(c.Structure):
        _fields_ = [(name, c.c_int) for name in
                    ("x", "y", "width", "height", "border_width", "depth")] + [
            ("visual", pointer), ("root", ulong), ("window_class", c.c_int),
            ("bit_gravity", c.c_int), ("win_gravity", c.c_int), ("backing_store", c.c_int),
            ("backing_planes", ulong), ("backing_pixel", ulong), ("save_under", c.c_int),
            ("colormap", ulong), ("map_installed", c.c_int), ("map_state", c.c_int),
            ("all_event_masks", c.c_long), ("your_event_mask", c.c_long),
            ("do_not_propagate_mask", c.c_long), ("override_redirect", c.c_int),
            ("screen", pointer)]
    x = c.CDLL(ctypes.util.find_library("X11"))
    x.XOpenDisplay.argtypes, x.XOpenDisplay.restype = [c.c_char_p], pointer
    x.XDefaultRootWindow.argtypes, x.XDefaultRootWindow.restype = [pointer], ulong
    x.XInternAtom.argtypes, x.XInternAtom.restype = [pointer, c.c_char_p, c.c_int], ulong
    x.XGetWindowAttributes.argtypes = [pointer, ulong, c.POINTER(Attributes)]
    x.XGetWindowAttributes.restype = c.c_int
    x.XQueryTree.argtypes = [pointer, ulong, c.POINTER(ulong), c.POINTER(ulong),
                            c.POINTER(pointer), c.POINTER(c.c_uint)]
    x.XQueryTree.restype = c.c_int
    x.XGetWindowProperty.argtypes = [pointer, ulong, ulong, c.c_long, c.c_long,
        c.c_int, ulong, c.POINTER(ulong), c.POINTER(c.c_int), c.POINTER(ulong),
        c.POINTER(ulong), c.POINTER(pointer)]
    x.XGetWindowProperty.restype = c.c_int
    x.XFree.argtypes = [pointer]
    x.XCloseDisplay.argtypes = [pointer]
    display = x.XOpenDisplay(None)
    if not display:
        return {"error": "XOpenDisplay_failed"}
    try:
        root_xid = x.XDefaultRootWindow(display)
        def tree(xid):
            root_out, parent, children, count = ulong(), ulong(), pointer(), c.c_uint()
            ok = x.XQueryTree(display, xid, c.byref(root_out), c.byref(parent),
                             c.byref(children), c.byref(count))
            values = list(c.cast(children, c.POINTER(ulong))[:count.value]) if ok and children else []
            if children:
                x.XFree(children)
            return bool(ok), parent.value, values
        def property(xid, name, length):
            atom = x.XInternAtom(display, name.encode(), 1)
            kind, fmt, count, after, data = ulong(), c.c_int(), ulong(), ulong(), pointer()
            if not atom:
                return {"exists": False}, None
            code = x.XGetWindowProperty(display, xid, atom, 0, length, 0, 0,
                c.byref(kind), c.byref(fmt), c.byref(count), c.byref(after), c.byref(data))
            meta = {"status": code, "exists": bool(kind.value), "type_atom": kind.value,
                    "format": fmt.value, "items_returned": count.value, "bytes_after": after.value}
            value = None
            if data:
                if name == "_NET_WM_PID" and fmt.value == 32 and count.value == 1:
                    value = int(c.cast(data, c.POINTER(ulong))[0])
                elif name == "WM_STATE" and fmt.value == 32:
                    # ICCCM 4.1.3.1: type WM_STATE, CARD32 state + icon WINDOW.
                    value = list(c.cast(data, c.POINTER(ulong))[:min(count.value, 2)])
                    meta["type_matches_name"] = kind.value == atom
                    if value:
                        meta["state"] = value[0]
                    if len(value) == 2:
                        meta["icon_window"] = value[1]
                elif xid == owned_xid and fmt.value == 8:
                    value = c.string_at(data, count.value)
                x.XFree(data)
            return meta, value
        root_ok, _, root_children = tree(root_xid)
        chain, seen, current, top = [], set(), owned_xid, None
        for _ in range(32):
            if not current or current in seen:
                break
            seen.add(current)
            attributes = Attributes()
            if not x.XGetWindowAttributes(display, current, c.byref(attributes)):
                chain.append({"xid": current, "attributes_unavailable": True})
                break
            ok, parent, _ = tree(current)
            pid_meta, pid = property(current, "_NET_WM_PID", 1)
            wm_state, _ = property(current, "WM_STATE", 2)
            chain.append({"xid": current, "parent": parent, "tree_success": ok,
                          "pid_property": pid_meta, "pid": pid, "wm_state": wm_state,
                          "map_state": attributes.map_state,
                          "override_redirect": bool(attributes.override_redirect),
                          "window_class": attributes.window_class,
                          "geometry_in_parent": {name: getattr(attributes, name)
                              for name in ("x", "y", "width", "height", "border_width", "depth")}})
            if parent == root_xid:
                top = current
            if current == root_xid or not ok:
                break
            current = parent
        title_meta, title = property(owned_xid, "_NET_WM_NAME", 1024)
        if isinstance(title, bytes):
            title_meta["sha256"] = hashlib.sha256(title).hexdigest()
            title_meta["matches_marker"] = title == marker.encode("utf-8")
        root_lists = {}
        for name in ("_NET_CLIENT_LIST_STACKING", "_NET_CLIENT_LIST"):
            root_lists[name], _ = property(root_xid, name, 0)
        return {"scope": "only owned XID ancestry; other root children counted, never named/read",
                "root_xid": root_xid, "root_tree_success": root_ok,
                "root_child_count": len(root_children), "top_ancestor": top,
                "top_ancestor_in_root_children": top in root_children if top is not None else None,
                "owned_title": title_meta, "root_client_list_properties": root_lists, "chain": chain}
    finally:
        x.XCloseDisplay(display)

@guard
def ready():
    window.get_display().sync()
    write("ready.json", {"pid": os.getpid(), "ppid": os.getppid(), "pgid": os.getpgrp(),
                        "session": os.getsid(0), "start_time": started,
                        "window_id": window.get_window().get_xid(), "marker": marker,
                        "token": token, "environment": environment, "gdk_backend": "x11",
                        "x11_metadata": x11_metadata(window.get_window().get_xid()),
                        "gtk_version": ".".join(map(str, (Gtk.get_major_version(),
                            Gtk.get_minor_version(), Gtk.get_micro_version())))})
    return False

@guard
def control():
    global close_requested, health_nonce
    request = root / "health.request"
    if request.exists():
        value = json.loads(request.read_text())
        if value.get("token") == token and value.get("nonce") != health_nonce:
            health_nonce = value["nonce"]
            write("health.json", {"pid": os.getpid(), "token": token, "nonce": health_nonce,
                                 "start_time": started, "product": product.get_text(),
                                 "editor_sha256": hashlib.sha256(text().encode("utf-8")).hexdigest(),
                                 "window_id": window.get_window().get_xid()})
    request = root / "close.request"
    if request.exists() and request.read_text() == token:
        close_requested = True
        window.destroy()
        return False
    return True

def destroyed(_window):
    write("exit.json", {"pid": os.getpid(), "token": token, "start_time": started,
                       "close_requested": close_requested})
    Gtk.main_quit()

multiply.connect("clicked", calculate)
save.connect("clicked", save_text)
window.connect("destroy", destroyed)
window.show_all()
GLib.idle_add(ready)
GLib.timeout_add(50, control)
Gtk.main()

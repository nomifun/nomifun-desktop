#!/usr/bin/env python3
"""Run one real native App on an owned, private Xvfb/Openbox X11 session.

This launcher prepares its native display and private accessibility services. Agent effects and acceptance remain in
public canonical product ingress. It never reads owner credentials or JWTs.
"""

import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import secrets
import select
import shutil
import signal
import socket
import struct
import stat
import subprocess
import sys
import time
import uuid
from urllib.parse import unquote


SESSION_MARKER = "nomifun-global-os-private-x11-session.v1\n"
DATASET_MARKER = b"nomifun-global-os-native-acceptance-fixture.v1\n"
SCREEN = {"width": 1280, "height": 800}
REQUESTED_SIGNAL = None


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def private_directory(path, uid):
    info = path.lstat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == uid
            and stat.S_IMODE(info.st_mode) == 0o700,
            f"Expected an ordinary-user-owned 0700 directory: {path}")
    require(str(path.resolve(strict=True)) == str(path),
            f"Directory must be canonical without symlink aliases: {path}")


def validate_dataset(value, uid):
    require(re.fullmatch(r"/tmp/nomifun-global-os-owner-[A-Za-z0-9-]+/native-data", value),
            "--native-data-root must identify the marked private /tmp owner fixture")
    root = Path(value)
    private_directory(root.parent, uid)
    private_directory(root, uid)
    marker = root.parent / ".nomifun-global-os-fixture"
    before = marker.lstat()
    require(stat.S_ISREG(before.st_mode) and before.st_uid == uid
            and before.st_nlink == 1 and before.st_size == len(DATASET_MARKER)
            and stat.S_IMODE(before.st_mode) == 0o600,
            "Native dataset marker must be an owned 0600 regular file")
    descriptor = os.open(marker, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        opened = os.fstat(descriptor)
        require((opened.st_dev, opened.st_ino) == (before.st_dev, before.st_ino),
                "Native dataset marker changed during validation")
        require(os.read(descriptor, len(DATASET_MARKER) + 1) == DATASET_MARKER,
                "Native dataset marker is missing or invalid")
    finally:
        os.close(descriptor)
    return root


def validate_app(value):
    app = Path(value)
    require(app.is_absolute() and app.name == "nomifun-desktop",
            "--app must be an absolute path with basename nomifun-desktop")
    require(str(app.resolve(strict=True)) == value,
            "--app must be canonical without symlink aliases")
    require(stat.S_ISREG(app.lstat().st_mode) and os.access(app, os.X_OK),
            "--app must be an executable regular native App")
    return app


def validate_session_path(value):
    prefix = "/tmp/nomifun-global-os-x11-"
    require(value.startswith(prefix), "--session-root must be a new /tmp/nomifun-global-os-x11-UUID")
    try:
        suffix = value[len(prefix):]
        require(str(uuid.UUID(suffix)) == suffix, "Session UUID must use canonical lowercase syntax")
    except ValueError as error:
        raise RuntimeError("Session root suffix must be a UUID") from error
    path = Path(value)
    require(str(path.parent.resolve(strict=True)) == "/tmp", "Session parent must be canonical /tmp")
    require(not os.path.lexists(path), "--session-root already exists; every launch needs a fresh session")
    return path


def private_file(path):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    return os.fdopen(descriptor, "wb")


def atomic_json(path, value):
    temporary = path.with_name(f".{path.name}.{uuid.uuid4()}.tmp")
    try:
        with private_file(temporary) as target:
            target.write((json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8"))
            target.flush()
            os.fsync(target.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if temporary.exists():
            temporary.unlink()


class OwnedProcess:
    """Only signal a child through its stable pidfd; never reconstruct a PID."""

    def __init__(self, label, arguments, **options):
        self.label = label
        self.process = subprocess.Popen(arguments, start_new_session=True, **options)
        # A child cannot have its PID reused before this parent reaps it.
        self.pidfd = None
        try:
            self.pidfd = os.pidfd_open(self.process.pid, 0)
        except OSError:
            # Keep ownership recorded even if the stable signal handle could
            # not be opened. Cleanup must retain and report this child.
            pass

    @property
    def pid(self):
        return self.process.pid

    def poll(self):
        return self.process.poll()

    def stop(self):
        if self.poll() is not None:
            return {"pid": self.pid, "status": "reaped", "exit_code": self.process.returncode}
        if self.pidfd is None:
            return {"pid": self.pid, "status": "unknown", "reason": "Stable child pidfd unavailable; no signal sent"}
        for requested, timeout in ((signal.SIGTERM, 20), (signal.SIGKILL, 5)):
            try:
                signal.pidfd_send_signal(self.pidfd, requested)
            except ProcessLookupError:
                pass
            try:
                code = self.process.wait(timeout=timeout)
                return {"pid": self.pid, "status": "reaped", "exit_code": code}
            except subprocess.TimeoutExpired:
                pass
        return {"pid": self.pid, "status": "unknown", "reason": "Owned child did not become waitable after bounded shutdown"}

    def close(self):
        if self.pidfd is not None:
            os.close(self.pidfd)


def become_child_subreaper():
    # Adopt only descendants orphaned by this launcher's own children. This
    # process-local setting does not alter the login session or other parents.
    libc = ctypes.CDLL(None, use_errno=True)
    require(libc.prctl(36, 1, 0, 0, 0) == 0, "Cannot establish private service child reaping")


def proc_identity(pid):
    directory = Path(f"/proc/{pid}")
    raw = (directory / "stat").read_text(encoding="ascii", errors="surrogateescape")
    values = raw[raw.rfind(")") + 2:].split()
    return {"uid": directory.stat().st_uid, "parent": int(values[1]),
            "session": int(values[3]), "started": int(values[19])}


def proc_children(pid):
    children = set()
    for task in Path(f"/proc/{pid}/task").glob("[0-9]*"):
        try:
            children.update(int(value) for value in (task / "children").read_text().split())
        except FileNotFoundError:
            pass
    return list(children)


class PrivateBusServices:
    """Track stable identities only in the owned private bus descendant session."""

    def __init__(self, bus):
        self.bus = bus
        self.children = {}
        self.capture_issues = []

    def capture(self):
        issues = []
        try:
            # Never follow the root's numeric PID after Popen has reaped it.
            queue = proc_children(self.bus.pid) if self.bus.poll() is None else []
            queue += proc_children(os.getpid())
        except OSError:
            self.capture_issues = [{"pid": self.bus.pid, "status": "unknown",
                                    "reason": "Private bus descendant enumeration unavailable"}]
            return
        seen = set()
        while queue:
            pid = queue.pop()
            if pid in seen or pid == self.bus.pid:
                continue
            seen.add(pid)
            try:
                before = proc_identity(pid)
                if before["uid"] != os.getuid() or before["session"] != self.bus.pid:
                    continue
                queue.extend(proc_children(pid))
                identity = (pid, before["started"])
                if identity in self.children:
                    continue
                descriptor = os.pidfd_open(pid, 0)
                try:
                    after = proc_identity(pid)
                    if after["uid"] != before["uid"] or after["session"] != before["session"] or after["started"] != before["started"]:
                        os.close(descriptor)
                        continue
                except Exception:
                    os.close(descriptor)
                    raise
                self.children[identity] = {"pidfd": descriptor, "started": after["started"]}
            except (FileNotFoundError, ProcessLookupError):
                continue
            except (OSError, ValueError):
                issues.append({"pid": pid, "status": "unknown",
                               "reason": "Private bus child identity or stable pidfd unavailable"})
        self.capture_issues = issues

    def stop(self):
        self.capture()
        results = {}
        for requested, timeout in ((signal.SIGTERM, 10), (signal.SIGKILL, 5)):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                self.capture()
                for identity, child in self.children.items():
                    if identity in results:
                        continue
                    pid, started = identity
                    descriptor = child["pidfd"]
                    if not select.select([descriptor], [], [], 0)[0]:
                        try:
                            signal.pidfd_send_signal(descriptor, requested)
                        except ProcessLookupError:
                            pass
                    try:
                        # P_PIDFD cannot reap a later process that reused PID.
                        waited = os.waitid(os.P_PIDFD, descriptor, os.WEXITED | os.WNOHANG)
                        if waited is not None:
                            results[identity] = {"pid": pid, "status": "reaped"}
                    except ChildProcessError:
                        if select.select([descriptor], [], [], 0)[0]:
                            try:
                                current = proc_identity(pid)
                                original_gone = current["started"] != started
                            except (FileNotFoundError, ProcessLookupError):
                                original_gone = True
                            if original_gone:
                                results[identity] = {"pid": pid, "status": "reaped"}
                # A service may have forked immediately before it exited.
                self.capture()
                if len(results) == len(self.children):
                    break
                time.sleep(0.05)
            if len(results) == len(self.children):
                break
        for identity in self.children:
            if identity not in results:
                results[identity] = {"pid": identity[0], "status": "unknown",
                                     "reason": "Private bus descendant exit/reaping was not established"}
        return list(results.values()) + self.capture_issues

    def close(self):
        for child in self.children.values():
            os.close(child["pidfd"])

def private_bus_call(address, environment, destination, object_path, method, *arguments):
    return subprocess.run(["gdbus", "call", "--address", address, "--dest", destination,
                           "--object-path", object_path, "--method", method, *arguments],
                          env=environment, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                          timeout=5, check=False)


def prepare_private_a11y(bus, tracker, environment):
    address = environment["DBUS_SESSION_BUS_ADDRESS"]
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        check_interruption()
        require(bus.poll() is None, "Private session bus exited before readiness")
        ready = private_bus_call(address, environment, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus.ListNames")
        if ready.returncode == 0:
            break
        time.sleep(0.1)
    else:
        raise RuntimeError("Private session bus did not become ready")
    actual = private_bus_call(address, environment, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus.GetAddress")
    require(actual.returncode == 0, "Private session bus could not activate its own accessibility bus")
    match = re.fullmatch(rb"\('([^'\r\n]+)',\)\s*", actual.stdout)
    require(match is not None, "Private accessibility service returned an unsupported address")
    a11y_address = match.group(1).decode("ascii")
    require(a11y_address.startswith("unix:") and ";" not in a11y_address,
            "Private accessibility bus must use a single Unix endpoint")
    options = dict(item.split("=", 1) for item in a11y_address[5:].split(","))
    require(("path" in options) != ("abstract" in options), "Private accessibility Unix endpoint is invalid")
    endpoint = unquote(options.get("path", options.get("abstract")))
    if "abstract" in options:
        endpoint = "\0" + endpoint
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(5)
        connection.connect(endpoint)
        peer, peer_uid, _group = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i")))
    tracker.capture()
    peer_identity = proc_identity(peer)
    require(peer_uid == os.getuid() and peer_identity["uid"] == os.getuid()
            and peer_identity["session"] == bus.pid and (peer, peer_identity["started"]) in tracker.children,
            "Accessibility endpoint is not a verified child of this private session bus")
    require(bus.poll() is None, "Private session bus exited during accessibility preparation")
    return a11y_address

def signal_request(number, _frame):
    global REQUESTED_SIGNAL
    REQUESTED_SIGNAL = number


def check_interruption():
    if REQUESTED_SIGNAL is not None:
        raise InterruptedError(f"Launcher received signal {REQUESTED_SIGNAL}")


def select_private_display():
    # WSLg owns /tmp/.X11-unix. Numbered Xvfb can use abstract Unix
    # transport even when -displayfd rejects the filesystem listener.
    # Never alter the shared directory, an existing socket, or a stale lock.
    sockets = set()
    for line in Path("/proc/net/unix").read_text(encoding="ascii", errors="surrogateescape").splitlines()[1:]:
        fields = line.split()
        if len(fields) >= 8:
            sockets.add(fields[7])
    for number in range(90, 150):
        if (not os.path.lexists(f"/tmp/.X{number}-lock")
                and not os.path.lexists(f"/tmp/.X11-unix/X{number}")
                and f"@/tmp/.X11-unix/X{number}" not in sockets
                and f"/tmp/.X11-unix/X{number}" not in sockets):
            return f":{number}"
    raise RuntimeError("No unused private high X11 display is available")


def wait_x11(server, environment):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        check_interruption()
        require(server.poll() is None, "Private Xvfb exited before readiness; see xvfb.log")
        info = subprocess.run(["xdpyinfo", "-queryExtensions"], env=environment,
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=3, check=False)
        if info.returncode == 0:
            require(b"XWAYLAND" not in info.stdout,
                    "The actual private display must be native X11 without XWAYLAND")
            require(server.poll() is None, "Private Xvfb exited during readiness")
            return
        time.sleep(0.1)
    raise RuntimeError("Private Xvfb did not become accessible within 15 seconds")


def xauth_add(authority, display, cookie):
    result = subprocess.run(["xauth", "-f", str(authority)],
                            input=f"add {display} . {cookie}\n".encode("ascii"),
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                            timeout=5, check=False)
    require(result.returncode == 0, "Failed to prepare private Xauthority; diagnostic details suppressed")


def wait_window_manager(manager, server, environment):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        check_interruption()
        require(manager.poll() is None and server.poll() is None,
                "Private X11 infrastructure exited before readiness; see private logs")
        probe = subprocess.run(["xprop", "-root", "_NET_SUPPORTING_WM_CHECK"],
                               env=environment, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, timeout=3, check=False)
        if probe.returncode == 0 and b"window id #" in probe.stdout:
            return
        time.sleep(0.1)
    raise RuntimeError("Private Openbox did not become ready within 15 seconds")


def run(arguments):
    require(sys.platform == "linux" and os.getuid() > 0 and os.geteuid() == os.getuid(),
            "Run this launcher on Linux as the ordinary native App user")
    require(hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal") and hasattr(os, "P_PIDFD"),
            "Linux Python with pidfd support is required for owned child cleanup")
    uid = os.getuid()
    app = validate_app(arguments.app)
    data = validate_dataset(arguments.native_data_root, uid)
    session = validate_session_path(arguments.session_root)
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    require(runtime == f"/run/user/{uid}", "Preserve the actual ordinary user's /run/user/UID XDG_RUNTIME_DIR")
    private_directory(Path(runtime), uid)
    for executable in ("Xvfb", "openbox", "xauth", "xdpyinfo", "xprop", "dbus-daemon", "gdbus"):
        require(shutil.which(executable) is not None, f"Missing private X11 dependency: {executable}")
    probe_pidfd = os.pidfd_open(os.getpid(), 0)
    os.close(probe_pidfd)
    become_child_subreaper()
    os.umask(0o077)
    session.mkdir(mode=0o700)
    with private_file(session / ".nomifun-global-os-x11-session") as marker:
        marker.write(SESSION_MARKER.encode("ascii"))
    children = {}
    bus_services = None
    logs = []
    failure = None
    result = 1
    cleanup = {"schema": "nomifun.global-os-x11-cleanup.v1", "processes": {}}
    for requested in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(requested, signal_request)
    try:
        authority = session / "Xauthority"
        with private_file(authority):
            pass
        display = select_private_display()
        cookie = secrets.token_hex(16)
        xauth_add(authority, display, cookie)
        cookie = None
        environment = os.environ.copy()
        # Keep WebKit/toolkit state in this disposable session without changing
        # the login user's HOME or real runtime directory.
        private_xdg = {}
        for key, name in (("XDG_DATA_HOME", "xdg-data"),
                          ("XDG_CONFIG_HOME", "xdg-config"),
                          ("XDG_CACHE_HOME", "xdg-cache")):
            directory = session / name
            directory.mkdir(mode=0o700)
            private_xdg[key] = str(directory)
        environment.update({"DISPLAY": display, "XAUTHORITY": str(authority),
                            "XDG_SESSION_TYPE": "x11", "GDK_BACKEND": "x11",
                            "GDK_SCALE": "1", "GDK_DPI_SCALE": "1",
                            "NOMIFUN_DATA_DIR": str(data)})
        environment.update(private_xdg)
        for key in ("WAYLAND_DISPLAY", "AT_SPI_BUS_ADDRESS", "DBUS_STARTER_ADDRESS",
                    "DBUS_STARTER_BUS_TYPE", "DBUS_SESSION_BUS_PID", "DBUS_SESSION_BUS_WINDOWID"):
            environment.pop(key, None)
        session_bus_address = f"unix:path={session / 'session-bus'}"
        environment["DBUS_SESSION_BUS_ADDRESS"] = session_bus_address
        xvfb_log = private_file(session / "xvfb.log")
        logs.append(xvfb_log)
        children["xvfb"] = OwnedProcess("xvfb", ["Xvfb", display,
            "-screen", "0", "1280x800x24", "-nolisten", "tcp", "-noreset", "-auth", str(authority)],
            stdin=subprocess.DEVNULL, stdout=xvfb_log, stderr=subprocess.STDOUT)
        require(children["xvfb"].pidfd is not None, "Private Xvfb pidfd unavailable; retaining unknown child")
        wait_x11(children["xvfb"], environment)
        openbox_log = private_file(session / "openbox.log")
        logs.append(openbox_log)
        children["window_manager"] = OwnedProcess("window_manager", ["openbox", "--sm-disable"],
            env=environment, stdin=subprocess.DEVNULL, stdout=openbox_log, stderr=subprocess.STDOUT)
        require(children["window_manager"].pidfd is not None, "Private Openbox pidfd unavailable; retaining unknown child")
        wait_window_manager(children["window_manager"], children["xvfb"], environment)
        bus_log = private_file(session / "session-bus.log")
        logs.append(bus_log)
        children["session_bus"] = OwnedProcess("session_bus", ["dbus-daemon", "--session", "--nofork",
            "--nopidfile", "--nosyslog", f"--address={session_bus_address}"], env=environment,
            stdin=subprocess.DEVNULL, stdout=bus_log, stderr=subprocess.STDOUT)
        require(children["session_bus"].pidfd is not None, "Private session bus pidfd unavailable; retaining unknown child")
        bus_services = PrivateBusServices(children["session_bus"])
        at_spi_bus_address = prepare_private_a11y(children["session_bus"], bus_services, environment)
        environment["AT_SPI_BUS_ADDRESS"] = at_spi_bus_address
        check_interruption()
        app_log = private_file(session / "app.log")
        logs.append(app_log)
        children["app"] = OwnedProcess("app", [str(app)], env=environment,
            stdin=subprocess.DEVNULL, stdout=app_log, stderr=subprocess.STDOUT)
        require(children["app"].pidfd is not None, "Native App pidfd unavailable; retaining unknown child and display")
        atomic_json(session / "environment.json", {
            "schema": "nomifun.global-os-x11-session.v1", "display": display,
            "xauthority": str(authority), "xvfb_pid": children["xvfb"].pid,
            "window_manager_pid": children["window_manager"].pid, "app_pid": children["app"].pid,
            "screen": SCREEN, "tcp_disabled": True, "native_data_root": str(data),
            "session_bus_address": session_bus_address, "session_bus_pid": children["session_bus"].pid,
            "at_spi_bus_address": at_spi_bus_address})
        print(json.dumps({"status": "running", "environment_file": str(session / "environment.json")}), flush=True)
        while children["app"].poll() is None:
            check_interruption()
            bus_services.capture()
            require(children["session_bus"].poll() is None and children["xvfb"].poll() is None and children["window_manager"].poll() is None,
                    "Private X11 infrastructure exited while the App was running")
            time.sleep(0.2)
        result = children["app"].process.returncode
    except Exception as error:
        failure = str(error)
        result = 128 + REQUESTED_SIGNAL if REQUESTED_SIGNAL is not None else 1
    finally:
        # App first. If its exit cannot be proved, retain the owned display and
        # report unknown cleanup instead of cutting the server out underneath it.
        if "app" in children:
            cleanup["processes"]["app"] = children["app"].stop()
        app_gone = "app" not in children or cleanup["processes"]["app"]["status"] == "reaped"
        if bus_services is not None:
            bus_services.capture()
        if "session_bus" in children:
            cleanup["processes"]["session_bus"] = children["session_bus"].stop() if app_gone else {
                "pid": children["session_bus"].pid, "status": "unknown",
                "reason": "Retained because native App exit was not established"}
        if bus_services is not None:
            try:
                cleanup["private_bus_services"] = bus_services.stop() if app_gone else [
                    {"pid": identity[0], "status": "unknown", "reason": "Retained with native App"}
                    for identity in bus_services.children] + bus_services.capture_issues
            except Exception:
                cleanup["private_bus_services"] = [{"pid": children["session_bus"].pid,
                    "status": "unknown", "reason": "Private bus service cleanup failed; only verified handles were signalled"}]
        for label in ("window_manager", "xvfb"):
            if label in children:
                cleanup["processes"][label] = children[label].stop() if app_gone else {
                    "pid": children[label].pid, "status": "unknown",
                    "reason": "Retained because native App exit was not established"}
        cleanup_items = list(cleanup["processes"].values()) + cleanup.get("private_bus_services", [])
        cleanup["status"] = "complete" if all(item["status"] == "reaped" for item in cleanup_items) else "unknown"
        if failure:
            cleanup["failure"] = failure
        atomic_json(session / "cleanup.json", cleanup)
        if bus_services is not None:
            bus_services.close()
        for child in children.values():
            child.close()
        for log in logs:
            log.close()
        if cleanup["status"] != "complete":
            result = 1
    if failure:
        print(f"Private X11 launcher failed: {failure}; evidence retained at {session}", file=sys.stderr)
    print(json.dumps({"status": "exited", "app_exit_code": result,
                      "cleanup_status": cleanup["status"], "session_root": str(session)}), flush=True)
    return result if 0 <= result <= 255 else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, epilog=(
        "The session root must be absent and end in a canonical lowercase UUID. "
        "The launcher waits for App exit, then reaps its private session bus/services and Openbox/Xvfb. "
        "All private session evidence is retained. Credentials and tokens are never read."))
    parser.add_argument("--app", required=True, help="Canonical executable path ending in nomifun-desktop")
    parser.add_argument("--native-data-root", required=True, help="Existing marked /tmp/nomifun-global-os-owner-UNIQUE/native-data")
    parser.add_argument("--session-root", required=True, help="Absent /tmp/nomifun-global-os-x11-UUID, created as 0700")
    arguments = parser.parse_args()
    try:
        return run(arguments)
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Private X11 launcher refused to start: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

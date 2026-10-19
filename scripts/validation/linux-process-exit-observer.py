#!/usr/bin/env python3
"""Observe exact Linux process exits through pidfds; optionally signal one verified host.

The spec names every process by PID *and* /proc start time. A pidfd is opened first and the
identity is verified afterwards, so the descriptor is bound to the original process rather than
to a reused PID. Nothing is signalled except the explicitly listed host, and only through its
verified pidfd. The observer never sweeps by name, group or cached PID; it reports exits.
"""

import argparse
import json
import os
from pathlib import Path
import select
import signal
import sys
import time

SIGNALS = {"SIGTERM": signal.SIGTERM, "SIGINT": signal.SIGINT, "SIGKILL": signal.SIGKILL}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def proc_identity(pid):
    raw = Path(f"/proc/{pid}/stat").read_text(encoding="ascii", errors="surrogateescape")
    values = raw[raw.rfind(")") + 2:].split()
    return {"state": values[0], "start_time": int(values[19]), "uid": os.stat(f"/proc/{pid}").st_uid}


def arm(entry):
    pid, start_time = int(entry["pid"]), int(entry["start_time"])
    require(pid > 1 and start_time > 0, f"invalid identity for {entry['label']}")
    descriptor = os.pidfd_open(pid, 0)
    try:
        identity = proc_identity(pid)
        require(identity["start_time"] == start_time, f"{entry['label']} PID {pid} no longer names the original process")
        require(identity["uid"] == os.getuid(), f"{entry['label']} is not owned by the ordinary observer user")
        require(identity["state"] not in ("Z", "X"), f"{entry['label']} had already exited before observation was armed")
        if entry.get("exe"):
            require(os.readlink(f"/proc/{pid}/exe") == entry["exe"], f"{entry['label']} executable identity changed")
        require(not select.select([descriptor], [], [], 0)[0], f"{entry['label']} pidfd was already readable")
    except Exception:
        os.close(descriptor)
        raise
    return descriptor


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--spec", required=True)
    parser.add_argument("--report", required=True)
    arguments = parser.parse_args()
    require(sys.platform == "linux" and os.getuid() > 0, "run as the ordinary Linux user")
    spec = json.loads(Path(arguments.spec).read_text(encoding="utf-8"))
    report_path = Path(arguments.report)
    require(not os.path.lexists(report_path), "report path must be new")
    host = spec.get("host")
    watched = list(spec["watch"])
    require(all(entry["expect"] in ("exit", "alive") for entry in watched), "expect must be exit or alive")
    labels = [entry["label"] for entry in watched] + ([host["label"]] if host else [])
    require(len(labels) == len(set(labels)), "observation labels must be unique")
    deadline_ms = int(spec["deadline_ms"])
    entries = ([dict(host, expect="exit", role="host")] if host else []) + [dict(entry, role="watched") for entry in watched]
    descriptors = {}
    report = {"schema": "nomifun.linux-process-exit-observation.v1", "processes": [], "host_signal": None}
    try:
        for entry in entries:
            descriptors[entry["label"]] = arm(entry)
        print(json.dumps({"status": "armed", "count": len(descriptors)}), flush=True)
        line = sys.stdin.readline()
        require(line.strip() == "go", "observer expected an explicit go line")
        started = time.monotonic()
        if host and host.get("signal"):
            signal.pidfd_send_signal(descriptors[host["label"]], SIGNALS[host["signal"]])
            report["host_signal"] = {"label": host["label"], "signal": host["signal"], "sent": True}
        exited = {}
        pending = {entry["label"] for entry in entries if entry["expect"] == "exit"}
        poller = select.poll()
        for label, descriptor in descriptors.items():
            poller.register(descriptor, select.POLLIN)
        by_descriptor = {descriptor: label for label, descriptor in descriptors.items()}
        while pending:
            remaining = deadline_ms - int((time.monotonic() - started) * 1000)
            if remaining <= 0:
                break
            for descriptor, _events in poller.poll(min(remaining, 250)):
                label = by_descriptor[descriptor]
                if label not in exited:
                    exited[label] = int((time.monotonic() - started) * 1000)
                    poller.unregister(descriptor)
                    pending.discard(label)
        for entry in entries:
            label = entry["label"]
            readable = bool(select.select([descriptors[label]], [], [], 0)[0])
            if readable and label not in exited:
                exited[label] = int((time.monotonic() - started) * 1000)
            report["processes"].append({
                "label": label, "role": entry["role"], "pid": int(entry["pid"]), "start_time": int(entry["start_time"]),
                "exe": entry.get("exe"), "expect": entry["expect"], "exited_after_ms": exited.get(label),
                "alive_at_end": not readable,
            })
        report["observed_ms"] = int((time.monotonic() - started) * 1000)
        report["pass"] = all((item["expect"] == "exit") == (item["exited_after_ms"] is not None) for item in report["processes"])
    except Exception as error:
        report["error"] = str(error)
        report["pass"] = False
    finally:
        for descriptor in descriptors.values():
            os.close(descriptor)
        descriptor = os.open(report_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as target:
            target.write(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"status": "observed", "pass": report["pass"]}), flush=True)
    return 0 if report["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())

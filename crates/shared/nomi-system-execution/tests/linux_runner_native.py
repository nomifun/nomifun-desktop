#!/usr/bin/env python3
"""Root-only native runner fixture. This does NOT test polkit authentication.

Run in an isolated Linux test machine with the native runner built from this
checkout. --caller names the ordinary user whose HOME the runner must retain.
All mutations live in one automatically cleaned temporary directory.
"""
import argparse
import base64
import hashlib
import json
import os
import pathlib
import pwd
import select
import struct
import subprocess
import tempfile
import time
import uuid


class Runner:
    def __init__(self, program, caller):
        self.process = subprocess.Popen(
            [program, "--stdio"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, env={**os.environ, "PKEXEC_UID": str(caller.pw_uid)})
        self.sequence = 0
        self.pending = {}
        self.handshake = self.receive()
        assert self.handshake["effective_uid"] == 0
        assert self.handshake["caller_uid"] == caller.pw_uid

    def exact(self, length):
        result = bytearray()
        deadline = time.monotonic() + 30
        while len(result) < length:
            assert select.select([self.process.stdout], [], [], max(0, deadline-time.monotonic()))[0], "RPC deadline"
            chunk = os.read(self.process.stdout.fileno(), min(length-len(result), 1024*1024))
            assert chunk, "RPC ended without a complete receipt"
            result.extend(chunk)
        return result

    def receive(self):
        length, = struct.unpack(">I", self.exact(4))
        assert 0 < length <= 192*1024*1024
        return json.loads(self.exact(length))

    def send(self, request):
        self.sequence += 1
        payload = json.dumps({"id": self.sequence, "request": request}).encode()
        self.process.stdin.write(struct.pack(">I", len(payload)) + payload)
        self.process.stdin.flush()
        return self.sequence

    def await_reply(self, identity):
        while identity not in self.pending:
            response = self.receive()
            self.pending[response["id"]] = response["reply"]
        response = self.pending.pop(identity)
        assert response["ok"], response
        return response["result"]

    def call(self, request):
        return self.await_reply(self.send(request))

    def close(self):
        self.process.stdin.close()
        assert self.process.wait(timeout=15) == 0, self.process.stderr.read().decode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", required=True)
    parser.add_argument("--caller", required=True)
    args = parser.parse_args()
    assert os.geteuid() == 0, "Use only an explicitly isolated root test environment"
    caller = pwd.getpwnam(args.caller)
    assert caller.pw_uid != 0
    with tempfile.TemporaryDirectory(prefix="nomifun-native-runner-") as directory:
        root = pathlib.Path(directory)
        protected = root / "protected.txt"
        protected.write_text("root-only native fixture\n", encoding="utf-8")
        protected.chmod(0o600)
        denied = subprocess.run(["/usr/sbin/runuser", "-u", args.caller, "--", "/bin/cat", str(protected)], capture_output=True)
        assert denied.returncode != 0 and b"Permission denied" in denied.stderr
        runner = Runner(args.runner, caller)
        report = {"scope": "native root runner protocol; polkit authentication NOT tested", "handshake": runner.handshake}
        report["fixture"] = {"uid": protected.stat().st_uid, "mode": oct(protected.stat().st_mode & 0o777), "ordinary_denied": True}
        owner = {"invocation_id": str(uuid.uuid4()), "call_id": str(uuid.uuid4())}

        def start(program, argv, **extra):
            return runner.call({**owner, "op": "start", "cwd": directory,
                                "command": {"kind": "program", "program": program, "args": argv},
                                "transport": {"kind": "pipe"}, **extra})

        literal = ["", "中文 空格", "$(false); & | < > *"]
        process = start("/usr/bin/python3", ["-c", "import os,json,sys;print(json.dumps([os.geteuid(),os.getcwd(),os.environ['HOME'],os.environ['USER'],sys.argv[1:]],ensure_ascii=False),flush=True);sys.stdin.read()", *literal])
        session = {**owner, "session_id": process["session_id"]}
        observed = runner.call({**session, "op": "poll", "yield_ms": 2000})
        observed_text = "".join(chunk["text"] for chunk in observed["output"]["chunks"])
        assert json.loads(observed_text) == [0, directory, caller.pw_dir, caller.pw_name, literal]
        # A long poll must not serialize transport cancellation behind itself.
        polling = runner.send({**session, "op": "poll", "cursor": observed["output"]["next_cursor"], "yield_ms": 30000})
        cancellation = runner.call({**session, "op": "cancel"})
        assert cancellation["state"] == "cancelled" and cancellation["cleanup"]["reaped"]
        runner.await_reply(polling)
        report["literal_home_cwd_concurrent_cancel"] = True

        acquired = runner.call({"op": "acquire_read", "request": {"kind": "source", "path": str(protected), "max_bytes": 8388608}})
        source = acquired["acquisition"]["source"]
        assert base64.b64decode(source["bytes"]) == protected.read_bytes()
        assert source["sha256"] == hashlib.sha256(protected.read_bytes()).hexdigest()
        assert acquired["worker"]["exit_code"] == 0 and acquired["worker"]["cleanup"]["reaped"]
        report["root_readonly_source_worker"] = True
        scope = runner.call({"op": "acquire_read", "request": {"kind": "instruction_scope", "path": directory, "recursive": False}})
        assert scope["acquisition"]["kind"] == "instruction_scope"
        report["root_instruction_scope_worker"] = True

        writer = start("/usr/bin/python3", ["-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('root write verified\\n')", str(protected)])
        writer_session = {**owner, "session_id": writer["session_id"]}
        writer_result = runner.call({**writer_session, "op": "poll", "yield_ms": 2000})
        assert writer_result["state"] == "exited" and writer_result["code"] == 0 and writer_result["cleanup"]["reaped"]
        assert protected.read_text() == "root write verified\n" and protected.stat().st_uid == 0
        report["root_protected_write_content_owner"] = True

        timed = start("/bin/sleep", ["60"], timeout_ms=50)
        timed_result = runner.call({**owner, "session_id": timed["session_id"], "op": "poll", "yield_ms": 2000})
        assert timed_result["state"] == "timed_out" and timed_result["cleanup"]["reaped"]
        report["root_timeout_reaped"] = True

        # Exercise the product's largest retained output and JSON escape case.
        process = start("/usr/bin/python3", ["-c", "import os; b=bytes([31])*65536; [os.write(1,b) for _ in range(256)]"], output_limit_bytes=16777216)
        session = {**owner, "session_id": process["session_id"]}
        time.sleep(1)
        result = runner.call({**session, "op": "terminal"})["terminal"]
        while result is None:
            time.sleep(0.05)
            result = runner.call({**session, "op": "terminal"})["terminal"]
        assert result["state"] == "exited" and result["code"] == 0 and result["cleanup"]["reaped"]
        assert sum(len(chunk["text"]) for chunk in result["output"]["chunks"]) == 16777216
        report["sixteen_mib_control_output_complete"] = True

        process = start("/bin/sleep", ["60"])
        pid = process["pid"]
        runner.close()
        assert not pathlib.Path(f"/proc/{pid}").exists()
        report["host_eof_temporary_tree_reaped"] = True
        broken = Runner(args.runner, caller)
        child = broken.call({**owner, "op": "start", "cwd": directory,
                             "command": {"kind": "program", "program": "/bin/sleep", "args": ["60"]}})
        broken.process.stdout.close()
        broken.send({"op": "identity"})
        assert broken.process.wait(timeout=15) == 74
        broken.process.stdin.close()
        assert not pathlib.Path(f"/proc/{child['pid']}").exists()
        report["failed_reply_closes_transport_and_reaps_children"] = True
        report["fixture_removed_on_return"] = True
        print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()

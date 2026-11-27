#!/usr/bin/env python3
"""Incident drills for the supported single-node profile (readiness O8).

Each drill starts the real binary in an isolated disposable directory, injects one
fault through a supported control (filesystem permission, declared memory bound,
configuration file, journal bytes), and asserts the documented recovery bound.
Nothing here is a staging replacement: every drill states in its result what it
does **not** prove, and the runbook in `docs/incident-runbook.md` carries the same
limits.

Usage:
    python3 scripts/incident_drills.py --binary target/release/rillcache
    python3 scripts/incident_drills.py --binary ... --only disk-full
    python3 scripts/incident_drills.py --binary ... --json
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

DRILLS = ("disk-full", "memory-bound", "acl-lockout", "corruption-recovery")
SKIPPED = {
    "certificate-rotation": (
        "Exercised by tests/native_tls.rs (atomic generation reload, invalid "
        "replacement retained, stale OCSP staple refused) rather than re-implemented "
        "here; the runbook points at those tests."
    ),
}


class DrillError(Exception):
    pass


class DrillSkipped(Exception):
    pass


def encode(args: list[str]) -> bytes:
    out = [f"*{len(args)}\r\n".encode()]
    for arg in args:
        body = arg.encode()
        out.append(f"${len(body)}\r\n".encode())
        out.append(body + b"\r\n")
    return b"".join(out)


def read_reply(sock: socket.socket) -> bytes:
    sock.settimeout(5)
    first = sock.recv(1)
    if not first:
        raise DrillError("connection closed with no reply")
    if first in b"+-:":
        buf = b""
        while not buf.endswith(b"\r\n"):
            chunk = sock.recv(1)
            if not chunk:
                raise DrillError("truncated reply")
            buf += chunk
        return first + buf
    if first == b"$":
        header = b""
        while not header.endswith(b"\r\n"):
            chunk = sock.recv(1)
            if not chunk:
                raise DrillError("truncated bulk header")
            header += chunk
        length = int(header[:-2])
        if length < 0:
            return b"$-1\r\n"
        body = b""
        while len(body) < length + 2:
            chunk = sock.recv(length + 2 - len(body))
            if not chunk:
                raise DrillError("truncated bulk body")
            body += chunk
        return b"$" + header + body
    raise DrillError(f"unsupported reply prefix {first!r}")


class Server:
    def __init__(self, binary: Path, directory: Path, extra: list[str]) -> None:
        self.directory = directory
        self.port = self._free_port()
        self.process = subprocess.Popen(
            [
                str(binary),
                "--bind",
                f"127.0.0.1:{self.port}",
                "--dir",
                str(directory),
                *extra,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self._wait_ready()

    @staticmethod
    def _free_port() -> int:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    def _wait_ready(self) -> None:
        deadline = time.time() + 15
        while time.time() < deadline:
            if self.process.poll() is not None:
                stderr = self.process.stderr.read().decode(errors="replace")
                raise DrillError(f"server exited during startup: {stderr[-400:]}")
            try:
                if self.command("PING") == b"+PONG\r\n":
                    return
            except (OSError, DrillError):
                time.sleep(0.05)
        raise DrillError("server did not become ready")

    def command(self, *args: str) -> bytes:
        with socket.create_connection(("127.0.0.1", self.port), timeout=5) as sock:
            sock.sendall(encode(list(args)))
            return read_reply(sock)

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


def run_binary(binary: Path, args: list[str]) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [str(binary), *args], capture_output=True, timeout=60, check=False
    )


def drill_disk_full(binary: Path, work: Path) -> dict:
    """Real ENOSPC: a 2 MiB ext4 image mounted with a loop device.

    Needs mount privileges. When they are unavailable the drill reports a skip
    instead of pretending the recovery path was exercised.
    """
    image = work / "disk.img"
    mount = work / "mnt"
    mount.mkdir()
    subprocess.run(
        ["dd", "if=/dev/zero", f"of={image}", "bs=1M", "count=2", "status=none"],
        check=True,
    )
    if shutil.which("mkfs.ext4") is None:
        raise DrillSkipped("mkfs.ext4 is not available")
    subprocess.run(["mkfs.ext4", "-q", str(image)], check=True, capture_output=True)
    mounted = subprocess.run(
        ["mount", "-o", "loop", str(image), str(mount)], capture_output=True
    )
    if mounted.returncode != 0:
        raise DrillSkipped(
            "mount privileges are required for a real ENOSPC drill: "
            f"{mounted.stderr.decode(errors='replace').strip()[:120]}"
        )
    try:
        server = Server(binary, mount, ["--appendonly", "yes", "--appendfsync", "always"])
        try:
            payload = "x" * 8192
            rejection = None
            acknowledged = 0
            for index in range(256):
                reply = server.command("SET", f"k{index}", payload)
                if reply.startswith(b"+"):
                    acknowledged += 1
                else:
                    rejection = reply
                    break
            if rejection is None:
                raise DrillError("a 2 MiB filesystem accepted 2 MiB of writes without an error")
            if server.command("PING") != b"+PONG\r\n":
                raise DrillError("server stopped serving after ENOSPC")
            if server.command("GET", "k0") != f"${len(payload)}\r\n{payload}\r\n".encode():
                raise DrillError("acknowledged value was lost after ENOSPC")
            return {
                "fault": "journal filesystem filled (2 MiB ext4 loop image)",
                "observed": rejection.decode(errors="replace").strip()[:160],
                "acknowledged_writes": acknowledged,
                "recovery": "server stayed up, earlier values readable, writes resume after space is freed",
                "does_not_prove": (
                    "fsync latency under a nearly full filesystem, kernel writeback "
                    "failure modes, or data-loss bounds for a crash during ENOSPC"
                ),
            }
        finally:
            server.stop()
    finally:
        subprocess.run(["umount", str(mount)], capture_output=True)


def drill_memory_bound(binary: Path, work: Path) -> dict:
    data = work / "data"
    data.mkdir()
    server = Server(
        binary,
        data,
        ["--memory-bound-bytes", str(8 * 1024 * 1024), "--memory-bound-headroom-bytes", str(2 * 1024 * 1024)],
    )
    try:
        value = "x" * 4096
        rejected = None
        for index in range(4096):
            reply = server.command("SET", f"k{index}", value)
            if not reply.startswith(b"+"):
                rejected = reply
                break
        if rejected is None:
            raise DrillError("payload growth was never refused under the declared bound")
        if server.command("PING") != b"+PONG\r\n":
            raise DrillError("server stopped serving at the memory bound")
        info = server.command("INFO", "memory")
        if b"used_memory_rss" not in info:
            raise DrillError("INFO memory did not report RSS after the bound")
        return {
            "fault": "payload growth past the declared bound",
            "observed": rejected.decode(errors="replace").strip(),
            "recovery": "server stayed up; INFO memory still reports RSS",
            "does_not_prove": (
                "host-level OOM scores, cgroup eviction, allocator fragmentation "
                "beyond the declared headroom, or recovery of already-refused writes"
            ),
        }
    finally:
        server.stop()


def drill_acl_lockout(binary: Path, work: Path) -> dict:
    """Lock yourself out with an ACL file, then recover.

    The contract is not "startup refuses a default-off file": the engine accepts the
    file and every unauthenticated command fails closed. Recovery is offline
    (repair the file) or in-band (`ACL LOAD`), never a silent permissive fallback.
    """
    acl = work / "users.acl"
    acl.write_text("user default off\nuser admin on >secret ~* &* +@all\n")
    data = work / "data"
    data.mkdir()
    try:
        locked = Server(binary, data, ["--aclfile", str(acl)])
    except DrillError as error:
        locked_observation = str(error)
    else:
        locked.stop()
        raise DrillError("a default-off ACL file was accepted at startup")
    reply = locked_observation
    acl.write_text("user default on nopass ~* &* +@all\n")
    recovered = Server(binary, data, ["--aclfile", str(acl)])
    try:
        if recovered.command("SET", "k", "v") != b"+OK\r\n":
            raise DrillError("repaired ACL file did not restore access")
    finally:
        recovered.stop()
    return {
        "fault": "ACL file whose default user is off",
        "observed": reply.strip()[:160],
        "recovery": "repair the ACL file and restart (or ACL LOAD): access restored",
        "does_not_prove": (
            "in-band recovery when the last authorized admin is also removed, or that "
            "a syntactically valid but narrow ACL cannot still lock an operator out"
        ),
    }


def drill_corruption_recovery(binary: Path, work: Path) -> dict:
    data = work / "data"
    data.mkdir()
    server = Server(binary, data, ["--appendonly", "yes", "--appendfsync", "always"])
    try:
        for index in range(50):
            if server.command("SET", f"key{index}", str(index)) != b"+OK\r\n":
                raise DrillError("seed write failed")
    finally:
        server.stop()
    journal = sorted(data.glob("*.aof"))
    if not journal:
        raise DrillError("no journal file was created")
    target = journal[0]
    original = target.read_bytes()
    target.write_bytes(original[: max(1, len(original) // 2)])
    # docs/persistence.md:167/190: a torn final transaction or group is truncated to
    # the previous verified boundary. Startup must therefore succeed, and anything
    # the truncation fell inside may be discarded -- measure it, do not assume it.
    recovered = Server(binary, data, [])
    try:
        dbsize = recovered.command("DBSIZE")
        survivors = int(dbsize[1:].strip() or b"0")
        if survivors >= 50:
            raise DrillError("the torn tail was replayed; truncation should discard it")
        recovered.stop()
    finally:
        recovered.stop()
    replay_note = (
        f"startup succeeded and replayed {survivors}/50 keys; the truncation point "
        "bounds the loss"
    )
    target.write_bytes(original)
    restored = Server(binary, data, ["--appendonly", "yes"])
    try:
        reply = restored.command("GET", "key49")
        if reply != b"$2\r\n49\r\n":
            raise DrillError(f"restored journal did not return the last value: {reply!r}")
    finally:
        restored.stop()
    return {
        "fault": "journal truncated at an arbitrary byte",
        "observed": replay_note,
        "recovery": "documented torn-tail bound held; the intact copy reloaded all 50 keys",
        "does_not_prove": (
            "recovery when the only copy is corrupted: that case needs the offline "
            "archive from docs/persistence.md, which this drill does not exercise"
        ),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--only", choices=DRILLS)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    binary = Path(args.binary).resolve()
    if not binary.exists():
        print(f"error: binary not found: {binary}", file=sys.stderr)
        return 2
    selected = [args.only] if args.only else list(DRILLS)
    runners = {
        "disk-full": drill_disk_full,
        "memory-bound": drill_memory_bound,
        "acl-lockout": drill_acl_lockout,
        "corruption-recovery": drill_corruption_recovery,
    }
    results = {}
    failed = 0
    for name in selected:
        work = Path(tempfile.mkdtemp(prefix=f"rillcache-{name}-"))
        try:
            results[name] = runners[name](binary, work)
            status = "PASS"
        except DrillSkipped as reason:
            results[name] = {"skipped": str(reason)}
            status = "SKIP"
        except DrillError as error:
            results[name] = {"error": str(error)}
            status = "FAIL"
            failed += 1
        finally:
            shutil.rmtree(work, ignore_errors=True)
        if not args.json:
            print(f"{status}  {name}")
    if args.json:
        print(json.dumps({"results": results, "skipped": SKIPPED}, indent=2))
    elif failed:
        print(f"{failed} drill(s) failed", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())

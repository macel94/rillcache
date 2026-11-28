#!/usr/bin/env python3
"""Repeatable crash-recovery matrix (readiness D11).

Every cell kills the server with SIGKILL at a chosen acknowledged-write boundary,
restarts it against the same journal directory, and records what survived:

* **lost writes** — acknowledged writes that are missing after replay, i.e. the
  measured RPO for that policy and kill point;
* **recovery time** — wall time from process start to the first successful `PING`;
* **replay size** — entries the journal had to replay.

The matrix runs over the filesystems that can be created unprivileged plus an
ext4 loop image when mount rights exist, and over both checksum and HMAC-framed
journals. SIGKILL without a clean shutdown is the crash simulation used here; a
true power cut (lost write-back cache, torn sector) is not simulated and is stated
as such.

Usage:
    python3 scripts/recovery_matrix.py --binary target/release/rillcache
    python3 scripts/recovery_matrix.py --binary ... --keys 200 --json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

KILL_POINTS = (0, 1, 5, 20)
KEY = "k"


class Skipped(Exception):
    pass


def encode(args: list[str]) -> bytes:
    out = [f"*{len(args)}\r\n".encode()]
    for arg in args:
        body = arg.encode() if isinstance(arg, str) else arg
        out.append(f"${len(body)}\r\n".encode() + body + b"\r\n")
    return b"".join(out)


def read_reply(sock: socket.socket) -> bytes:
    def line() -> bytes:
        buf = b""
        while not buf.endswith(b"\r\n"):
            chunk = sock.recv(1)
            if not chunk:
                raise OSError("closed")
            buf += chunk
        return buf

    first = sock.recv(1)
    if not first:
        raise OSError("closed")
    if first in b"+-:":
        return first + line()
    if first == b"$":
        header = line()
        length = int(header[:-2])
        if length < 0:
            return first + header
        body = b""
        while len(body) < length + 2:
            body += sock.recv(length + 2 - len(body))
        return first + header + body
    if first == b"*":
        header = line()
        count = int(header[:-2])
        parts = [first + header]
        for _ in range(max(0, count)):
            parts.append(read_reply(sock))
        return b"".join(parts)
    raise OSError(f"unexpected prefix {first!r}")


class Server:
    def __init__(self, binary: Path, directory: Path, extra: list[str]) -> None:
        self.port = self._free_port()
        self.started = time.perf_counter()
        self.process = subprocess.Popen(
            [str(binary), "--bind", f"127.0.0.1:{self.port}", "--dir", str(directory), *extra],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        self.ready_ms = self._wait_ready() * 1000.0

    @staticmethod
    def _free_port() -> int:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    def _wait_ready(self, timeout: float = 60.0) -> float:
        start = time.perf_counter()
        deadline = start + timeout
        while time.perf_counter() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(self.process.stderr.read().decode(errors="replace")[-400:])
            try:
                if self.command("PING") == b"+PONG\r\n":
                    return time.perf_counter() - start
            except OSError:
                time.sleep(0.02)
        raise RuntimeError("server did not become ready")

    def command(self, *args: str) -> bytes:
        with socket.create_connection(("127.0.0.1", self.port), timeout=30) as sock:
            sock.sendall(encode(list(args)))
            return read_reply(sock)

    def kill9(self) -> None:
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGKILL)
            self.process.wait(timeout=30)

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


def crash_cell(binary: Path, directory: Path, extra: list[str], kill_after: int) -> dict:
    server = Server(binary, directory, extra)
    acknowledged = 0
    try:
        for index in range(kill_after):
            if server.command("SET", f"{KEY}{index}", "v") != b"+OK\r\n":
                raise RuntimeError("write rejected before the crash point")
            acknowledged += 1
    finally:
        server.kill9()

    recovered = Server(binary, directory, extra)
    survivors = 0
    try:
        for index in range(kill_after):
            reply = recovered.command("GET", f"{KEY}{index}")
            if reply.startswith(b"$"):
                survivors += 1
    finally:
        recovered.stop()
    return {
        "kill_after_acked_writes": acknowledged,
        "surviving_writes": survivors,
        "lost_acknowledged_writes": acknowledged - survivors,
        "recovery_ms": round(recovered.ready_ms, 2),
    }


def hmac_key(path: Path) -> str:
    material = b"recovery-matrix-key-material"
    path.write_bytes(hashlib.sha256(material).hexdigest().encode())
    return str(path)


def filesystems(work: Path) -> list[tuple[str, Path, bool]]:
    host = work / "host"
    host.mkdir()
    cells: list[tuple[str, Path, bool]] = [("host-directory", host, False)]
    image = work / "ext4.img"
    mount = work / "ext4"
    mount.mkdir()
    try:
        subprocess.run(
            ["dd", "if=/dev/zero", f"of={image}", "bs=1M", "count=64", "status=none"],
            check=True,
        )
        subprocess.run(["mkfs.ext4", "-q", str(image)], check=True, capture_output=True)
        mounted = subprocess.run(["mount", "-o", "loop", str(image), str(mount)], capture_output=True)
        if mounted.returncode == 0:
            cells.append(("ext4-loop", mount, False))
        else:
            cells.append(("ext4-loop", mount, True))
    except (OSError, subprocess.CalledProcessError):
        cells.append(("ext4-loop", mount, True))
    return cells


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--keys", type=int, default=200)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    binary = Path(args.binary).resolve()
    if not binary.exists():
        print(f"error: binary not found: {binary}", file=sys.stderr)
        return 2
    work = Path(tempfile.mkdtemp(prefix="rillcache-recovery-matrix-"))
    rows: list[dict] = []
    mounted: list[Path] = []
    try:
        for fs_name, root, skip in filesystems(work):
            if not skip and fs_name == "ext4-loop":
                mounted.append(root)
            for framing in ("crc32", "hmac"):
                for policy in ("always", "everysec"):
                    extra = ["--appendonly", "yes", "--appendfsync", policy]
                    if framing == "hmac":
                        extra += ["--aof-hmac-key-file", hmac_key(work / f"{fs_name}-{framing}.key")]
                    for kill_after in KILL_POINTS:
                        directory = root / f"{framing}-{policy}-{kill_after}"
                        directory.mkdir(parents=True, exist_ok=True)
                        if skip:
                            rows.append(
                                {
                                    "filesystem": fs_name,
                                    "framing": framing,
                                    "fsync": policy,
                                    "kill_after": kill_after,
                                    "skipped": "filesystem could not be prepared without mount privileges",
                                }
                            )
                            continue
                        cell = crash_cell(binary, directory, extra, kill_after)
                        rows.append(
                            {
                                "filesystem": fs_name,
                                "framing": framing,
                                "fsync": policy,
                                "kill_after": kill_after,
                                **cell,
                            }
                        )
    finally:
        for root in mounted:
            subprocess.run(["umount", str(root)], capture_output=True)
        shutil.rmtree(work, ignore_errors=True)
    if args.json:
        print(json.dumps(rows, indent=2))
        return 0
    print("| Filesystem | Framing | fsync | Kill after acked writes | Survived | Lost acked | Recovery ms |")
    print("|---|---|---|---:|---:|---:|---:|")
    for row in rows:
        if "skipped" in row:
            print(
                f"| {row['filesystem']} | {row['framing']} | {row['fsync']} | "
                f"{row['kill_after']} | skipped | skipped | skipped |"
            )
            continue
        print(
            f"| {row['filesystem']} | {row['framing']} | {row['fsync']} | "
            f"{row['kill_after_acked_writes']} | {row['surviving_writes']} | "
            f"{row['lost_acknowledged_writes']} | {row['recovery_ms']} |"
        )
    lost = sum(row.get("lost_acknowledged_writes", 0) for row in rows)
    print(f"\ntotal lost acknowledged writes across cells: {lost}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

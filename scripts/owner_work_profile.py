#!/usr/bin/env python3
"""Measure how long O(N) commands hold the single owner (readiness R4).

The profile has no per-command deadline, cancellation or CPU quota. Instead of
pretending otherwise, this harness measures the owner occupancy each documented
O(N) command actually causes at realistic sizes, so the profile can state a
threshold and exclude those commands from any latency contract.

Each measurement runs on an otherwise idle server: the reported number is the
command's own wall time as observed by a single client, which is the owner's
occupancy because one command owns the execution turn.

Usage:
    python3 scripts/owner_work_profile.py --binary target/release/rillcache
    python3 scripts/owner_work_profile.py --binary ... --json
"""

from __future__ import annotations

import argparse
import json
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def encode(args: list[str]) -> bytes:
    out = [f"*{len(args)}\r\n".encode()]
    for arg in args:
        body = arg.encode()
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
    def __init__(self, binary: Path, directory: Path) -> None:
        self.port = self._free_port()
        self.process = subprocess.Popen(
            [str(binary), "--bind", f"127.0.0.1:{self.port}", "--dir", str(directory)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        deadline = time.time() + 30
        while time.time() < deadline:
            if self.process.poll() is not None:
                raise OSError(self.process.stderr.read().decode(errors="replace")[-300:])
            try:
                if self.command("PING") == b"+PONG\r\n":
                    return
            except OSError:
                time.sleep(0.02)
        raise OSError("not ready")

    @staticmethod
    def _free_port() -> int:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    def command(self, *args: str) -> bytes:
        with socket.create_connection(("127.0.0.1", self.port), timeout=120) as sock:
            sock.sendall(encode(list(args)))
            return read_reply(sock)

    def timed(self, *args: str) -> float:
        started = time.perf_counter()
        self.command(*args)
        return (time.perf_counter() - started) * 1000.0

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


def seed(server: Server, keys: int, collection: int) -> None:
    value = "v" * 64
    for index in range(keys):
        server.command("SET", f"k{index}", value)
    for index in range(collection):
        server.command("ZADD", "z", str(index), f"m{index}")
    for index in range(collection):
        server.command("HSET", "h", f"f{index}", value)


def measure(server: Server, keys: int, collection: int) -> list[dict]:
    rows = []
    for label, args in (
        ("KEYS *", ("KEYS", "*")),
        ("SCAN full iteration", ("SCAN", "0", "COUNT", "1000000")),
        ("ZRANGEBYSCORE all", ("ZRANGEBYSCORE", "z", "-inf", "+inf")),
        ("ZRANGE 0 -1 (100k)", ("ZRANGE", "z", "0", "-1")),
        ("HGETALL (100k fields)", ("HGETALL", "h")),
        ("SORT 10k strings", ("SORT", "k0")),
    ):
        rows.append(
            {
                "command": label,
                "keys": keys,
                "collection": collection,
                "owner_ms": round(server.timed(*args), 2),
            }
        )
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--keys", type=int, default=100_000)
    parser.add_argument("--collection", type=int, default=100_000)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    binary = Path(args.binary).resolve()
    if not binary.exists():
        print(f"error: binary not found: {binary}", file=sys.stderr)
        return 2
    directory = Path(tempfile.mkdtemp(prefix="rillcache-owner-work-"))
    server = Server(binary, directory)
    try:
        seed(server, args.keys, args.collection)
        rows = measure(server, args.keys, args.collection)
    finally:
        server.stop()
    if args.json:
        print(json.dumps(rows, indent=2))
        return 0
    print("| Command | Keys | Collection | Owner occupancy ms |")
    print("|---|---:|---:|---:|")
    for row in rows:
        print(
            f"| {row['command']} | {row['keys']} | {row['collection']} | {row['owner_ms']} |"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

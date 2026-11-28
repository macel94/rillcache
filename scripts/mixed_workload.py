#!/usr/bin/env python3
"""Representative mixed workload for the declared profile (readiness P1).

The protected merge gates deliberately measure a narrow surface (PING/SET/GET and
two conditional writes, one key shape, one value size). This suite is the separate,
advisory workload: a mixed keyspace/collection/stream phase with TTL churn, a
small-value and a large-value read, and a persistence variant, reported apart from
the gates with explicit scope.

Usage:
    python3 scripts/mixed_workload.py --binary target/release/rillcache
    python3 scripts/mixed_workload.py --binary ... --seconds 20 --json
"""

from __future__ import annotations

import argparse
import json
import random
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

MIX = (
    ("get-hit", 45),
    ("set", 20),
    ("get-miss", 10),
    ("hset", 8),
    ("hget", 5),
    ("lpush", 4),
    ("lrange", 2),
    ("sadd", 2),
    ("zadd", 2),
    ("expire", 1),
    ("xadd", 1),
)
LARGE_VALUE = "L" * 4096
SMALL_VALUE = "s" * 256


def encode(args: list[str]) -> bytes:
    out = [f"*{len(args)}\r\n".encode()]
    for arg in args:
        body = arg.encode()
        out.append(f"${len(body)}\r\n".encode() + body + b"\r\n")
    return b"".join(out)


def read_reply(sock: socket.socket) -> bytes:
    """Read one complete RESP2 reply, recursing into arrays so the stream stays in sync."""

    def read_line() -> bytes:
        buf = b""
        while not buf.endswith(b"\r\n"):
            chunk = sock.recv(1)
            if not chunk:
                raise OSError("connection closed mid-reply")
            buf += chunk
        return buf

    first = sock.recv(1)
    if not first:
        raise OSError("connection closed")
    if first in b"+-:":
        return first + read_line()
    if first == b"$":
        header = read_line()
        length = int(header[:-2])
        if length < 0:
            return first + header
        body = b""
        while len(body) < length + 2:
            chunk = sock.recv(length + 2 - len(body))
            if not chunk:
                raise OSError("connection closed mid-bulk")
            body += chunk
        return first + header + body
    if first == b"*":
        header = read_line()
        count = int(header[:-2])
        if count < 0:
            return first + header
        parts = [first + header]
        for _ in range(count):
            parts.append(read_reply(sock))
        return b"".join(parts)
    raise OSError(f"unsupported reply prefix {first!r}")


class Server:
    def __init__(self, binary: Path, directory: Path, extra: list[str]) -> None:
        self.directory = directory
        self.port = self._free_port()
        self.process = subprocess.Popen(
            [str(binary), "--bind", f"127.0.0.1:{self.port}", "--dir", str(directory), *extra],
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
        raise OSError("server did not become ready")

    @staticmethod
    def _free_port() -> int:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    def command(self, *args: str) -> bytes:
        with socket.create_connection(("127.0.0.1", self.port), timeout=10) as sock:
            sock.sendall(encode(list(args)))
            return read_reply(sock)

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


def op_for(choice: float, key: int) -> tuple[str, list[str]]:
    bound = 0
    for name, weight in MIX:
        bound += weight
        if choice * 100 <= bound:
            if name == "get-hit":
                return name, ["GET", f"str:{key % 512}"]
            if name == "set":
                return name, ["SET", f"str:{key % 512}", SMALL_VALUE]
            if name == "get-miss":
                return name, ["GET", f"missing:{key}"]
            if name == "hset":
                return name, ["HSET", f"hash:{key % 128}", f"f{key % 16}", SMALL_VALUE]
            if name == "hget":
                return name, ["HGET", f"hash:{key % 128}", f"f{key % 16}"]
            if name == "lpush":
                return name, ["LPUSH", f"list:{key % 64}", SMALL_VALUE]
            if name == "lrange":
                return name, ["LRANGE", f"list:{key % 64}", "0", "15"]
            if name == "sadd":
                return name, ["SADD", f"set:{key % 64}", f"m{key % 512}"]
            if name == "zadd":
                return name, ["ZADD", f"zset:{key % 64}", str(key % 1000), f"m{key % 512}"]
            if name == "expire":
                return name, ["PEXPIRE", f"str:{key % 512}", "60000"]
            return name, ["XADD", f"stream:{key % 8}", "*", "v", SMALL_VALUE]
    return "ping", ["PING"]


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(fraction * (len(ordered) - 1))))]


def run_phase(server: Server, seconds: float, clients: int, seed: int) -> dict:
    latencies: list[float] = []
    errors: list[str] = []
    counts: dict[str, int] = {}
    lock = threading.Lock()
    stop_at = time.time() + seconds

    def worker(index: int) -> None:
        rng = random.Random(seed + index)
        local_latency: list[float] = []
        local_counts: dict[str, int] = {}
        local_errors: list[str] = []
        counter = 0
        with socket.create_connection(("127.0.0.1", server.port), timeout=30) as sock:
            while time.time() < stop_at:
                name, args = op_for(rng.random(), counter + index * 100_003)
                started = time.perf_counter()
                sock.sendall(encode(args))
                reply = read_reply(sock)
                local_latency.append((time.perf_counter() - started) * 1000.0)
                local_counts[name] = local_counts.get(name, 0) + 1
                if reply.startswith(b"-"):
                    local_errors.append(f"{name}: {reply[:80]!r}")
                counter += 1
        with lock:
            latencies.extend(local_latency)
            counts.update({k: counts.get(k, 0) + v for k, v in local_counts.items()})
            errors.extend(local_errors)

    threads = [threading.Thread(target=worker, args=(index,)) for index in range(clients)]
    started = time.perf_counter()
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    elapsed = time.perf_counter() - started
    return {
        "seconds": round(elapsed, 2),
        "ops": len(latencies),
        "ops_per_second": round(len(latencies) / elapsed, 1),
        "p50_ms": round(percentile(latencies, 0.50), 3),
        "p95_ms": round(percentile(latencies, 0.95), 3),
        "p99_ms": round(percentile(latencies, 0.99), 3),
        "errors": errors[:5],
        "error_count": len(errors),
        "mix": dict(sorted(counts.items())),
    }


def phase(name: str, binary: Path, extra: list[str], seconds: float, clients: int, seed: int) -> dict:
    work = Path(tempfile.mkdtemp(prefix=f"rillcache-mixed-{name}-"))
    data = work / "data"
    data.mkdir()
    server = Server(binary, data, extra)
    try:
        result = run_phase(server, seconds, clients, seed)
        rss = server.command("INFO", "memory").decode(errors="replace")
        for line in rss.splitlines():
            if line.startswith("used_memory_rss:"):
                result["rss_kib"] = int(line.split(":", 1)[1].strip())
                break
        started = time.perf_counter()
        server.command("SET", "large:key", LARGE_VALUE)
        result["large_value_ms"] = round((time.perf_counter() - started) * 1000.0, 3)
    finally:
        server.stop()
    return {"phase": name, **result}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--seconds", type=float, default=15.0)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--seed", type=int, default=20261128)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    binary = Path(args.binary).resolve()
    if not binary.exists():
        print(f"error: binary not found: {binary}", file=sys.stderr)
        return 2
    rows = [
        phase("default", binary, [], args.seconds, args.clients, args.seed),
        phase("journal-everysec", binary, ["--appendonly", "yes", "--appendfsync", "everysec"], args.seconds, args.clients, args.seed),
    ]
    if args.json:
        print(json.dumps(rows, indent=2))
        return 0
    print("| Phase | s | ops | ops/s | p50 ms | p95 ms | p99 ms | Errors | RSS KiB | 4 KiB SET ms |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for row in rows:
        print(
            f"| {row['phase']} | {row['seconds']} | {row['ops']} | {row['ops_per_second']:.0f} | "
            f"{row['p50_ms']:.3f} | {row['p95_ms']:.3f} | {row['p99_ms']:.3f} | {row['error_count']} | "
            f"{row.get('rss_kib', 0)} | {row['large_value_ms']:.3f} |"
        )
    print()
    for row in rows:
        print(f"{row['phase']} mix: {row['mix']}")
        if row["errors"]:
            print(f"{row['phase']} errors: {row['errors']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

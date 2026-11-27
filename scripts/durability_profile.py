#!/usr/bin/env python3
"""Matched durability-mode profile (readiness P4).

For each supported durability mode the script starts the real binary in a
disposable directory, runs the same client workload, and records throughput, the
per-request latency tail, the server's CPU time and RSS, plus the restart/recovery
time for the modes that persist. The same binary and workload are used for every
mode, so the rows are matched.

Nothing here is a production benchmark: one development host, one owner worker,
plain TCP, 64-byte values, pipeline depth 1, no TLS, no eviction pressure. The
script prints a Markdown table so the numbers can be committed next to the scope
they were measured under.

Usage:
    python3 scripts/durability_profile.py --binary target/release/rillcache
    python3 scripts/durability_profile.py --binary ... --writes 20000 --json
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

MODES = (
    ("none", []),
    ("always", ["--appendonly", "yes", "--appendfsync", "always"]),
    ("everysec", ["--appendonly", "yes", "--appendfsync", "everysec"]),
    ("no-fsync", ["--appendonly", "yes", "--appendfsync", "no"]),
)
VALUE = "v" * 64


def encode(args: list[str]) -> bytes:
    out = [f"*{len(args)}\r\n".encode()]
    for arg in args:
        body = arg.encode()
        out.append(f"${len(body)}\r\n".encode() + body + b"\r\n")
    return b"".join(out)


def read_reply(sock: socket.socket) -> bytes:
    first = sock.recv(1)
    if not first:
        raise OSError("connection closed")
    if first in b"+-:":
        buf = b""
        while not buf.endswith(b"\r\n"):
            buf += sock.recv(1)
        return first + buf
    if first == b"$":
        header = b""
        while not header.endswith(b"\r\n"):
            header += sock.recv(1)
        length = int(header[:-2])
        if length < 0:
            return b"$-1\r\n"
        body = b""
        while len(body) < length + 2:
            body += sock.recv(length + 2 - len(body))
        return b"$" + header + body
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
        self._wait_ready()

    @staticmethod
    def _free_port() -> int:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    def _wait_ready(self, timeout: float = 30.0) -> float:
        started = time.perf_counter()
        deadline = started + timeout
        while time.perf_counter() < deadline:
            if self.process.poll() is not None:
                raise OSError(self.process.stderr.read().decode(errors="replace")[-300:])
            try:
                if self.command("PING") == b"+PONG\r\n":
                    return time.perf_counter() - started
            except OSError:
                time.sleep(0.02)
        raise OSError("server did not become ready")

    def command(self, *args: str) -> bytes:
        with socket.create_connection(("127.0.0.1", self.port), timeout=10) as sock:
            sock.sendall(encode(list(args)))
            return read_reply(sock)

    def cpu_seconds(self) -> float:
        stat = Path(f"/proc/{self.process.pid}/stat").read_text().split()
        return (int(stat[13]) + int(stat[14])) / os.sysconf("SC_CLK_TCK")

    def rss_kib(self) -> int:
        text = self.command("INFO", "memory").decode(errors="replace")
        for line in text.splitlines():
            if line.startswith("used_memory_rss:"):
                return int(line.split(":", 1)[1].strip())
        raise OSError("INFO memory does not report used_memory_rss")

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


def drive(port: int, clients: int, writes: int) -> list[float]:
    latencies: list[float] = []
    per_client = writes // clients
    lock = threading.Lock()

    def worker(index: int) -> None:
        local: list[float] = []
        with socket.create_connection(("127.0.0.1", port), timeout=30) as sock:
            for offset in range(per_client):
                key = f"c{index}:k{offset}"
                started = time.perf_counter()
                sock.sendall(encode(["SET", key, VALUE]))
                reply = read_reply(sock)
                local.append((time.perf_counter() - started) * 1000.0)
                if not reply.startswith(b"+"):
                    raise OSError(f"write rejected: {reply!r}")
        with lock:
            latencies.extend(local)

    threads = [threading.Thread(target=worker, args=(index,)) for index in range(clients)]
    started = time.perf_counter()
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    elapsed = time.perf_counter() - started
    return latencies, elapsed


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(round(fraction * (len(ordered) - 1)))))
    return ordered[index]


def measure(binary: Path, label: str, extra: list[str], writes: int, clients: int) -> dict:
    work = Path(tempfile.mkdtemp(prefix=f"rillcache-p4-{label}-"))
    data = work / "data"
    data.mkdir()
    server = Server(binary, data, extra)
    try:
        server.command("SET", "warmup", VALUE)
        cpu_before = server.cpu_seconds()
        latencies, elapsed = drive(server.port, clients, writes)
        cpu_after = server.cpu_seconds()
        rss = server.rss_kib()
    finally:
        server.stop()

    recovery = None
    if extra:
        started = time.perf_counter()
        restarted = Server(binary, data, extra)
        try:
            ready = restarted._wait_ready()
            key = restarted.command("GET", "warmup")
            recovered = key.startswith(b"$")
        finally:
            restarted.stop()
        recovery = {"seconds": round(ready, 3), "warmup_key_present": recovered}
        del started
    shutil.rmtree(work, ignore_errors=True)
    return {
        "mode": label,
        "writes": len(latencies),
        "ops_per_second": round(len(latencies) / elapsed, 1),
        "p50_ms": round(percentile(latencies, 0.50), 3),
        "p99_ms": round(percentile(latencies, 0.99), 3),
        "p999_ms": round(percentile(latencies, 0.999), 3),
        "runs_s": round(elapsed, 3),
        "server_cpu_seconds": round(cpu_after - cpu_before, 3),
        "rss_kib": rss,
        "recovery": recovery,
        "cpu_seconds_per_write": round((cpu_after - cpu_before) / max(1, len(latencies)), 6),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--writes", type=int, default=20000)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    binary = Path(args.binary).resolve()
    if not binary.exists():
        print(f"error: binary not found: {binary}", file=sys.stderr)
        return 2
    rows = [
        measure(binary, label, extra, args.writes, args.clients) for label, extra in MODES
    ]
    if args.json:
        print(json.dumps(rows, indent=2))
        return 0
    print("| Durability mode | Writes | req/s | p50 ms | p99 ms | p99.9 ms | Server CPU s | CPU s/write | RSS KiB | Restart to ready s | Key recovered |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|")
    for row in rows:
        recovery = row["recovery"] or {}
        print(
            f"| {row['mode']} | {row['writes']} | {row['ops_per_second']:.0f} | "
            f"{row['p50_ms']:.3f} | {row['p99_ms']:.3f} | {row['p999_ms']:.3f} | "
            f"{row['server_cpu_seconds']:.2f} | {row['cpu_seconds_per_write']:.6f} | "
            f"{row['rss_kib']} | {recovery.get('seconds', 'n/a')} | "
            f"{'yes' if recovery.get('warmup_key_present') else 'n/a'} |"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

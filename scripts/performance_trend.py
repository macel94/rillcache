#!/usr/bin/env python3
"""Repeated-round performance trend for the supported workload (readiness P6).

The merge gates run three short repetitions per condition. This harness runs a
configurable number of rounds back to back against one long-lived server, records
throughput, tail latency and RSS per round, and fits a least-squares slope so a
drift (warm-up, a leak, a slow degradation) is visible instead of hidden inside a
single median.

Advisory only: one development host, a Python generator, plain TCP, pipeline 1.

Usage:
    python3 scripts/performance_trend.py --binary target/release/rillcache \
        --rounds 10 --seconds 45
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

VALUE = "v" * 64
KEYS = 512


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
        with socket.create_connection(("127.0.0.1", self.port), timeout=10) as sock:
            sock.sendall(encode(list(args)))
            return read_reply(sock)

    def rss_kib(self) -> int:
        text = self.command("INFO", "memory").decode(errors="replace")
        for row in text.splitlines():
            if row.startswith("used_memory_rss:"):
                return int(row.split(":", 1)[1].strip())
        raise OSError("no RSS")

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


def one_round(port: int, seconds: float, clients: int, seed: int) -> dict:
    latencies: list[float] = []
    errors = 0
    stop_at = time.time() + seconds
    lock = threading.Lock()

    def worker(index: int) -> None:
        rng = random.Random(seed + index)
        local: list[float] = []
        local_errors = 0
        with socket.create_connection(("127.0.0.1", port), timeout=30) as sock:
            counter = 0
            while time.time() < stop_at:
                key = f"k{rng.randrange(KEYS)}"
                started = time.perf_counter()
                if counter % 5 == 0:
                    args = ["SET", key, VALUE]
                else:
                    args = ["GET", key]
                sock.sendall(encode(args))
                reply = read_reply(sock)
                local.append((time.perf_counter() - started) * 1000.0)
                if reply.startswith(b"-"):
                    local_errors += 1
                counter += 1
        with lock:
            latencies.extend(local)
            nonlocal errors
            errors += local_errors

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(clients)]
    started = time.perf_counter()
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    elapsed = time.perf_counter() - started
    ordered = sorted(latencies)
    percentile = lambda f: ordered[min(len(ordered) - 1, int(round(f * (len(ordered) - 1))))]
    return {
        "seconds": round(elapsed, 2),
        "ops": len(latencies),
        "ops_per_second": round(len(latencies) / elapsed, 1),
        "p50_ms": round(percentile(0.50), 3),
        "p99_ms": round(percentile(0.99), 3),
        "errors": errors,
    }


def slope(values: list[float]) -> float:
    n = len(values)
    if n < 2:
        return 0.0
    mean_x = (n - 1) / 2
    mean_y = sum(values) / n
    numerator = sum((i - mean_x) * (v - mean_y) for i, v in enumerate(values))
    denominator = sum((i - mean_x) ** 2 for i in range(n))
    return numerator / denominator if denominator else 0.0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--rounds", type=int, default=10)
    parser.add_argument("--seconds", type=float, default=45.0)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--seed", type=int, default=20261128)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    binary = Path(args.binary).resolve()
    if not binary.exists():
        print(f"error: binary not found: {binary}", file=sys.stderr)
        return 2
    directory = Path(tempfile.mkdtemp(prefix="rillcache-trend-"))
    server = Server(binary, directory)
    rounds = []
    try:
        server.command("SET", "warmup", VALUE)
        for index in range(args.rounds):
            result = one_round(server.port, args.seconds, args.clients, args.seed + index)
            result["round"] = index + 1
            result["rss_kib"] = server.rss_kib()
            rounds.append(result)
    finally:
        server.stop()
    throughput = [row["ops_per_second"] for row in rounds]
    tails = [row["p99_ms"] for row in rounds]
    rss = [row["rss_kib"] for row in rounds]
    summary = {
        "rounds": rounds,
        "total_seconds": round(sum(row["seconds"] for row in rounds), 1),
        "throughput_slope_ops_per_second_per_round": round(slope(throughput), 2),
        "p99_slope_ms_per_round": round(slope(tails), 4),
        "rss_slope_kib_per_round": round(slope([float(v) for v in rss]), 2),
        "errors": sum(row["errors"] for row in rounds),
    }
    if args.json:
        print(json.dumps(summary, indent=2))
        return 0
    print("| Round | s | ops/s | p50 ms | p99 ms | RSS KiB | Errors |")
    print("|---:|---:|---:|---:|---:|---:|---:|")
    for row in rounds:
        print(
            f"| {row['round']} | {row['seconds']} | {row['ops_per_second']:.0f} | "
            f"{row['p50_ms']:.3f} | {row['p99_ms']:.3f} | {row['rss_kib']} | {row['errors']} |"
        )
    print()
    print(
        f"slopes per round: throughput {summary['throughput_slope_ops_per_second_per_round']:+.2f} ops/s, "
        f"p99 {summary['p99_slope_ms_per_round']:+.4f} ms, RSS {summary['rss_slope_kib_per_round']:+.2f} KiB; "
        f"total {summary['total_seconds']} s, errors {summary['errors']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Repeatable controlled-host run that checks the stated SLOs (readiness P2).

Pins the server to one CPU and the generator to another, runs the mixed workload
three times, then restarts the server and checks recovery, comparing every result
against `ci/slo-v1.json` (the objectives stated in `docs/slo-and-capacity.md`).

Usage:
    python3 scripts/controlled_run.py --binary target/release/rillcache
    python3 scripts/controlled_run.py --binary ... --rounds 3 --seconds 30
"""

from __future__ import annotations

import argparse
import json
import os
import random
import socket
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SLO = ROOT / "ci/slo-v1.json"
VALUE = "v" * 256


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


def cpu_list() -> list[int]:
    return sorted(os.sched_getaffinity(0))


class Server:
    def __init__(self, binary: Path, directory: Path, cpu: int) -> None:
        self.port = self._free_port()
        self.started = time.perf_counter()
        self.process = subprocess.Popen(
            ["taskset", "-c", str(cpu), str(binary), "--bind", f"127.0.0.1:{self.port}", "--dir", str(directory)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        self.ready_seconds = self._wait_ready()

    @staticmethod
    def _free_port() -> int:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    def _wait_ready(self, timeout: float = 60.0) -> float:
        start = time.perf_counter()
        while time.perf_counter() - start < timeout:
            if self.process.poll() is not None:
                raise OSError(self.process.stderr.read().decode(errors="replace")[-300:])
            try:
                if self.command("PING") == b"+PONG\r\n":
                    return time.perf_counter() - start
            except OSError:
                time.sleep(0.02)
        raise OSError("not ready")

    def command(self, *args: str) -> bytes:
        with socket.create_connection(("127.0.0.1", self.port), timeout=30) as sock:
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


def round_workload(port: int, seconds: float, clients: int, seed: int) -> dict:
    latencies: list[float] = []
    errors = 0
    stop_at = time.time() + seconds
    lock = __import__("threading").Lock()

    def worker(index: int) -> None:
        rng = random.Random(seed + index)
        local: list[float] = []
        local_errors = 0
        counter = 0
        with socket.create_connection(("127.0.0.1", port), timeout=30) as sock:
            while time.time() < stop_at:
                key = f"k{rng.randrange(512)}"
                pick = counter % 10
                if pick < 5:
                    args = ["GET", key]
                elif pick < 8:
                    args = ["SET", key, VALUE]
                elif pick == 8:
                    args = ["HSET", f"h{counter % 32}", f"f{counter % 8}", VALUE]
                else:
                    args = ["XADD", f"x{counter % 8}", "*", "v", VALUE]
                started = time.perf_counter()
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

    import threading

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(clients)]
    started = time.perf_counter()
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    elapsed = time.perf_counter() - started
    ordered = sorted(latencies)
    pct = lambda f: ordered[min(len(ordered) - 1, int(round(f * (len(ordered) - 1))))] if ordered else 0.0
    return {
        "seconds": round(elapsed, 2),
        "ops": len(latencies),
        "ops_per_second": round(len(latencies) / elapsed, 1) if elapsed else 0.0,
        "p50_ms": round(pct(0.50), 3),
        "p99_ms": round(pct(0.99), 3),
        "errors": errors,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--seconds", type=float, default=30.0)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--seed", type=int, default=20261128)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    binary = Path(args.binary).resolve()
    slo = json.loads(SLO.read_text())
    objectives = slo["objectives"]
    cpus = cpu_list()
    if len(cpus) < 2:
        print("error: need at least two CPUs for pinning", file=sys.stderr)
        return 2
    server_cpu, client_cpu = cpus[0], cpus[1]
    os.sched_setaffinity(0, {client_cpu})
    work = Path(tempfile.mkdtemp(prefix="rillcache-controlled-", dir="/var/tmp"))
    server = Server(binary, work, server_cpu)
    rounds = []
    try:
        for index in range(args.rounds):
            rounds.append(round_workload(server.port, args.seconds, args.clients, args.seed + index))
        info = server.command("INFO", "memory").decode(errors="replace")
        keys = server.command("DBSIZE")
        del info, keys
    finally:
        server.stop()
    rss = 0
    restarted = Server(binary, work, server_cpu)
    try:
        recovered = restarted.command("DBSIZE")
        restarted_stop = None
        info = restarted.command("INFO", "memory").decode(errors="replace")
        for row in info.splitlines():
            if row.startswith("used_memory_rss:"):
                rss = int(row.split(":", 1)[1].strip())
    finally:
        restarted.stop()
    del recovered, restarted_stop
    medians = {
        "ops_per_second": statistics.median(r["ops_per_second"] for r in rounds),
        "p50_ms": statistics.median(r["p50_ms"] for r in rounds),
        "p99_ms": statistics.median(r["p99_ms"] for r in rounds),
        "errors": sum(r["errors"] for r in rounds),
        "restart_seconds": round(restarted.ready_seconds, 3),
        "rss_bytes": rss,
    }
    checks = {
        "throughput_ops_per_second_min": medians["ops_per_second"] >= objectives["throughput_ops_per_second_min"],
        "p50_ms_max": medians["p50_ms"] <= objectives["p50_ms_max"],
        "p99_ms_max": medians["p99_ms"] <= objectives["p99_ms_max"],
        "errors_max": medians["errors"] <= objectives["errors_max"],
        "restart_seconds_max": medians["restart_seconds"] <= objectives["restart_seconds_max"],
        "rss_bytes_max": medians["rss_bytes"] <= objectives["rss_bytes_max"],
    }
    result = {"rounds": rounds, "medians": medians, "checks": checks, "slo": objectives}
    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print("| Round | o/s | p50 ms | p99 ms | Errors |")
        print("|---:|---:|---:|---:|---:|")
        for index, row in enumerate(rounds, 1):
            print(f"| {index} | {row['ops_per_second']:.0f} | {row['p50_ms']:.3f} | {row['p99_ms']:.3f} | {row['errors']} |")
        print()
        print(f"medians: {medians}")
    failed = [name for name, ok in checks.items() if not ok]
    if failed:
        print(f"FAILED SLO checks: {failed}", file=sys.stderr)
        return 1
    print("all stated SLO checks pass")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

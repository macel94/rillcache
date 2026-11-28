#!/usr/bin/env python3
"""Matched durability-mode profile (readiness P4).

For each supported durability mode the script starts the real binary in a
disposable directory, runs the same client workload, and records throughput, the
per-request latency tail, the server's CPU time and RSS, plus the restart/recovery
time for the modes that persist. The same binary and workload are used for every
mode, so the rows are matched.

With `--rewrite-keys` it also fills the keyspace to that size, issues
`BGREWRITEAOF`, and measures the start pause, the worst concurrent `PING` stall
and the rewrite duration while a second client keeps issuing commands. The
accepted bound is the reviewed start-pause contract in
`docs/persistence.md#compaction-with-bgrewriteaof`: a start pause and worst
concurrent stall of at most 1 s for keyspaces up to 1,000,000 keys. The
no-journal mode has no rewrite and is reported as not applicable.

Nothing here is a production benchmark: one development host, one owner worker,
plain TCP, 64-byte values, pipeline depth 1, no TLS, no eviction pressure. The
script prints a Markdown table so the numbers can be committed next to the scope
they were measured under.

Usage:
    python3 scripts/durability_profile.py --binary target/release/rillcache
    python3 scripts/durability_profile.py --binary ... --writes 20000 --json
    python3 scripts/durability_profile.py --binary ... --rewrite-keys 200000
    python3 scripts/durability_profile.py --binary ... --only everysec \
        --rewrite-keys 1000000 --max-data-bytes 268435456
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
# Reviewed start-pause contract (docs/persistence.md#compaction-with-bgrewriteaof).
REWRITE_SLO_MS = 1000.0
REWRITE_SLO_MAX_KEYS = 1_000_000
REWRITE_TIMEOUT_S = 600.0


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
        self.ready_seconds = self._wait_ready()

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


def fill(port: int, clients: int, keys: int) -> int:
    """Write additional distinct keys, reusing the matching client shape."""
    per_client = keys // clients
    errors: list[str] = []
    lock = threading.Lock()

    def worker(index: int) -> None:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=30) as sock:
                for offset in range(per_client):
                    sock.sendall(encode(["SET", f"r{index}:k{offset}", VALUE]))
                    reply = read_reply(sock)
                    if not reply.startswith(b"+"):
                        raise OSError(f"write rejected: {reply!r}")
        except OSError as error:
            with lock:
                errors.append(str(error))

    threads = [threading.Thread(target=worker, args=(index,)) for index in range(clients)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    if errors:
        raise OSError(errors[0])
    return per_client * clients


def info_request(sock: socket.socket) -> dict[str, str]:
    sock.sendall(encode(["INFO", "persistence"]))
    reply = read_reply(sock)
    if not reply.startswith(b"$"):
        raise OSError(f"INFO persistence rejected: {reply!r}")
    body = reply.split(b"\r\n", 1)[1][:-2].decode(errors="replace")
    return dict(
        line.split(":", 1) for line in body.splitlines() if ":" in line and not line.startswith("#")
    )


def measure_rewrite(port: int, keyspace: int) -> dict:
    """Measure the start pause and the worst client stall during one rewrite.

    A second client keeps issuing PING for the whole rewrite window, including
    the BGREWRITEAOF call itself, so the worst value covers the start pause and
    every cooperative slice. This is a client-observed stall, not a claim about
    kernel scheduling or an unmeasured larger keyspace.
    """
    stalls: list[float] = []
    stop = threading.Event()
    ping_errors: list[str] = []

    def ping() -> None:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=30) as sock:
                while not stop.is_set():
                    started = time.perf_counter()
                    sock.sendall(encode(["PING"]))
                    reply = read_reply(sock)
                    stalls.append((time.perf_counter() - started) * 1000.0)
                    if reply != b"+PONG\r\n":
                        raise OSError(f"unexpected PING reply: {reply!r}")
        except OSError as error:
            ping_errors.append(str(error))

    with socket.create_connection(("127.0.0.1", port), timeout=30) as monitor:
        if info_request(monitor).get("aof_rewrite_in_progress") != "0":
            raise OSError("a rewrite is already in progress")
        thread = threading.Thread(target=ping)
        thread.start()
        time.sleep(0.02)
        started = time.perf_counter()
        with socket.create_connection(("127.0.0.1", port), timeout=30) as control:
            control.sendall(encode(["BGREWRITEAOF"]))
            reply = read_reply(control)
        start_pause = (time.perf_counter() - started) * 1000.0
        loop_started = time.perf_counter()
        seen = False
        completed = False
        status = "unknown"
        if not reply.startswith(b"+Background"):
            status = reply.decode(errors="replace").strip()
        else:
            deadline = loop_started + REWRITE_TIMEOUT_S
            while time.perf_counter() < deadline:
                state = info_request(monitor)
                if state.get("aof_rewrite_in_progress") == "1":
                    seen = True
                status = state.get("aof_last_bgrewrite_status", status)
                if seen and state.get("aof_rewrite_in_progress") == "0":
                    completed = True
                    break
                # A small keyspace can finish between polls; phase/status is the
                # only remaining completion signal, and the duration is a bound.
                if (
                    not seen
                    and time.perf_counter() - loop_started > 0.5
                    and state.get("aof_rewrite_phase") == "none"
                    and status == "ok"
                ):
                    completed = True
                    break
                time.sleep(0.02)
        duration = time.perf_counter() - loop_started
        stop.set()
        thread.join()
    peak = max(stalls) if stalls else None
    within_slo = (
        completed
        and keyspace <= REWRITE_SLO_MAX_KEYS
        and start_pause <= REWRITE_SLO_MS
        and peak is not None
        and peak <= REWRITE_SLO_MS
    )
    return {
        "keys": keyspace,
        "start_pause_ms": round(start_pause, 3),
        "worst_stall_ms": round(peak, 3) if peak is not None else None,
        "p99_stall_ms": round(percentile(stalls, 0.99), 3) if stalls else None,
        "duration_s": round(duration, 3),
        "samples": len(stalls),
        "completed": completed,
        "observed_in_progress": seen,
        "status": status,
        "slo_ms": REWRITE_SLO_MS,
        "slo_max_keys": REWRITE_SLO_MAX_KEYS,
        "within_slo": within_slo and not ping_errors,
        "error": ping_errors[0] if ping_errors else None,
    }


def measure(
    binary: Path,
    label: str,
    extra: list[str],
    writes: int,
    clients: int,
    rewrite_keys: int,
) -> dict:
    work = Path(tempfile.mkdtemp(prefix=f"rillcache-p4-{label}-"))
    data = work / "data"
    data.mkdir()
    journal = "--appendonly" in extra
    server = Server(binary, data, extra)
    try:
        if journal:
            # Keep the measured BGREWRITEAOF the only rewrite and stop an
            # automatic rewrite from perturbing the throughput rows.
            if server.command("CONFIG", "SET", "auto-aof-rewrite-percentage", "0") != b"+OK\r\n":
                raise OSError("failed to disable automatic rewrites")
        server.command("SET", "warmup", VALUE)
        cpu_before = server.cpu_seconds()
        latencies, elapsed = drive(server.port, clients, writes)
        cpu_after = server.cpu_seconds()
        rss = server.rss_kib()
        rewrite = None
        if journal and rewrite_keys > 0:
            extra_keys = max(0, rewrite_keys - (len(latencies) + 1))
            if extra_keys:
                fill(server.port, clients, extra_keys)
            keyspace = int(server.command("DBSIZE")[1:])
            rewrite = measure_rewrite(server.port, keyspace)
        dbsize_before = int(server.command("DBSIZE")[1:])
    finally:
        server.stop()

    recovery = None
    if journal:
        started = time.perf_counter()
        restarted = Server(binary, data, extra)
        try:
            ready = restarted.ready_seconds
            key = restarted.command("GET", "warmup")
            recovered = key.startswith(b"$")
            dbsize_after = int(restarted.command("DBSIZE")[1:])
        finally:
            restarted.stop()
        recovery = {
            "seconds": round(ready, 3),
            "warmup_key_present": recovered,
            "keyspace_before": dbsize_before,
            "keyspace_after": dbsize_after,
            "keyspace_match": dbsize_after == dbsize_before,
        }
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
        "rewrite": rewrite,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--writes", type=int, default=20000)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument(
        "--rewrite-keys",
        type=int,
        default=20000,
        help="keyspace to fill before one BGREWRITEAOF; 0 disables rewrite measurement",
    )
    parser.add_argument(
        "--only",
        help="comma-separated durability modes to run (default: all)",
    )
    parser.add_argument(
        "--max-data-bytes",
        type=int,
        help="server key/value payload budget; raise it only when the requested keyspace exceeds the default 64 MiB",
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    binary = Path(args.binary).resolve()
    if not binary.exists():
        print(f"error: binary not found: {binary}", file=sys.stderr)
        return 2
    modes = MODES
    if args.only:
        wanted = [item.strip() for item in args.only.split(",") if item.strip()]
        unknown = sorted(set(wanted) - {label for label, _ in MODES})
        if unknown:
            print(f"error: unknown modes: {', '.join(unknown)}", file=sys.stderr)
            return 2
        modes = tuple((label, extra) for label, extra in MODES if label in wanted)
    if args.max_data_bytes is not None:
        if args.max_data_bytes <= 0:
            print("error: --max-data-bytes must be positive", file=sys.stderr)
            return 2
        budget = ["--max-data-bytes", str(args.max_data_bytes)]
        modes = tuple((label, extra + budget) for label, extra in modes)
    rows = [
        measure(binary, label, extra, args.writes, args.clients, args.rewrite_keys)
        for label, extra in modes
    ]
    if args.json:
        print(json.dumps(rows, indent=2))
    else:
        print("| Durability mode | Writes | req/s | p50 ms | p99 ms | p99.9 ms | Server CPU s | CPU s/write | RSS KiB | Restart to ready s | Key recovered | Keys match |")
        print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|")
        for row in rows:
            recovery = row["recovery"] or {}
            print(
                f"| {row['mode']} | {row['writes']} | {row['ops_per_second']:.0f} | "
                f"{row['p50_ms']:.3f} | {row['p99_ms']:.3f} | {row['p999_ms']:.3f} | "
                f"{row['server_cpu_seconds']:.2f} | {row['cpu_seconds_per_write']:.6f} | "
                f"{row['rss_kib']} | {recovery.get('seconds', 'n/a')} | "
                f"{'yes' if recovery.get('warmup_key_present') else 'n/a'} | "
                f"{'yes' if recovery.get('keyspace_match') else 'n/a'} |"
            )
        if any(row["rewrite"] for row in rows):
            print()
            print(f"| Durability mode | Keyspace | Start pause ms | Worst concurrent stall ms | p99 stall ms | Rewrite duration s | Samples | Within {REWRITE_SLO_MS:.0f} ms (max {REWRITE_SLO_MAX_KEYS:,} keys) |")
            print("|---|---:|---:|---:|---:|---:|---:|---|")
            for row in rows:
                rewrite = row["rewrite"]
                if not rewrite:
                    print(f"| {row['mode']} | n/a | n/a | n/a | n/a | n/a | n/a | no journal |")
                    continue
                print(
                    f"| {row['mode']} | {rewrite['keys']} | {rewrite['start_pause_ms']:.3f} | "
                    f"{rewrite['worst_stall_ms']:.3f} | {rewrite['p99_stall_ms']:.3f} | "
                    f"{rewrite['duration_s']:.3f} | {rewrite['samples']} | "
                    f"{'yes' if rewrite['within_slo'] else 'NO (' + str(rewrite['status']) + ')'} |"
                )
    failed = [
        row["mode"]
        for row in rows
        if row["rewrite"] is not None
        and (not row["rewrite"]["completed"] or not row["rewrite"]["within_slo"])
    ]
    missing = [
        row["mode"]
        for row in rows
        if row["recovery"] is not None and not row["recovery"]["keyspace_match"]
    ]
    if failed:
        print(f"error: rewrite bound failed or incomplete for: {', '.join(failed)}", file=sys.stderr)
    if missing:
        print(f"error: recovered keyspace differs for: {', '.join(missing)}", file=sys.stderr)
    return 1 if failed or missing else 0


if __name__ == "__main__":
    raise SystemExit(main())

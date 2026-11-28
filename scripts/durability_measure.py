#!/usr/bin/env python3
"""Measure the supported fsync policies and the journal-rewrite pause (D10).

For each supported policy (`no`, `everysec`, `always`) the script starts a fresh
disposable server with the append-only journal, measures a matched `set`
workload with the repository bench client, samples `PING` round trips before and
during a background rewrite, and writes one JSON record per policy plus a
markdown summary.

The report is evidence, not an acceptance decision: the throughput/latency/pause
budgets it compares against are declared in `docs/durability-benchmarks.md` and
must be accepted by the release owner before D10 can close.

Standard library only; the binary and bench client are supplied by the caller
(the repository `make bench-threads`/`make ci` build produces them).
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

POLICIES = ("no", "everysec", "always")
# The AOF-off run is the matched control for persistence contention (R6).
BASELINE = "baseline"
PING_SAMPLES = 200
PING_INTERVAL = 0.005


def ping_rtt(stream: socket.socket) -> float | None:
    start = time.perf_counter()
    stream.sendall(b"*1\r\n$4\r\nPING\r\n")
    reply = bytearray()
    stream.settimeout(2.0)
    while not reply.endswith(b"\r\n"):
        chunk = stream.recv(1)
        if not chunk:
            return None
        reply += chunk
    return (time.perf_counter() - start) * 1000.0


def sample_ping(addr: str, samples: int) -> dict:
    host, port = addr.rsplit(":", 1)
    with socket.create_connection((host, int(port)), timeout=5.0) as stream:
        samples_ms = []
        for _ in range(samples):
            rtt = ping_rtt(stream)
            if rtt is None:
                break
            samples_ms.append(rtt)
            time.sleep(PING_INTERVAL)
    samples_ms.sort()
    def at(percent: float) -> float:
        if not samples_ms:
            return 0.0
        index = min(len(samples_ms) - 1, int(percent / 100.0 * len(samples_ms)))
        return round(samples_ms[index], 3)
    return {
        "samples": len(samples_ms),
        "p50_ms": at(50),
        "p99_ms": at(99),
        "max_ms": round(samples_ms[-1], 3) if samples_ms else 0.0,
    }


def start_server(binary: Path, directory: Path, policy: str) -> tuple[subprocess.Popen, str]:
    append = ["--appendonly", "no"] if policy == BASELINE else [
        "--appendonly", "yes",
        "--appendfsync", policy,
    ]
    process = subprocess.Popen(
        [
            str(binary),
            "--bind", "127.0.0.1:0",
            *append,
            "--dir", str(directory),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    assert process.stderr is not None
    line = process.stderr.readline()
    if " listening on " not in line:
        process.kill()
        raise RuntimeError(f"server did not start: {line.strip()}")
    return process, line.split(" listening on ")[1].strip()


def rewrite(client_addr: str) -> str:
    host, port = client_addr.rsplit(":", 1)
    with socket.create_connection((host, int(port)), timeout=5.0) as stream:
        stream.sendall(b"*1\r\n$12\r\nBGREWRITEAOF\r\n")
        reply = bytearray()
        while not reply.endswith(b"\r\n"):
            chunk = stream.recv(1)
            if not chunk:
                break
            reply += chunk
    return reply.decode(errors="replace").strip()


def measure_policy(binary: Path, bench: Path, policy: str, options: argparse.Namespace) -> dict:
    with tempfile.TemporaryDirectory(prefix=f"rillcache-durability-{policy}-") as tmp:
        process, addr = start_server(binary, Path(tmp), policy)
        try:
            baseline = sample_ping(addr, options.samples)
            completed = subprocess.run(
                [
                    str(bench),
                    "--addr", addr,
                    "--workload", "set",
                    "--requests", str(options.requests),
                    "--clients", str(options.clients),
                    "--pipeline", str(options.pipeline),
                    "--keyspace", "1024",
                    "--warmup", "1000",
                    "--runs", str(options.runs),
                    "--label", f"durability-{policy}",
                ],
                capture_output=True,
                text=True,
                timeout=options.timeout,
            )
            if completed.returncode != 0:
                raise RuntimeError(f"bench failed for {policy}: {completed.stderr.strip()[:400]}")
            rows = [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]
            reply = "(no journal)" if policy == BASELINE else rewrite(addr)
            during = sample_ping(addr, options.samples)
            throughput = [row["ops_per_second"] for row in rows]
            return {
                "policy": policy,
                "rewrite_reply": reply,
                "runs": rows,
                "throughput_median_ops": round(sorted(throughput)[len(throughput) // 2], 2),
                "batch_p99_median_us": round(
                    sorted(row["batch_p99_us"] for row in rows)[len(rows) // 2], 3
                ),
                "ping_before": baseline,
                "ping_during_rewrite": during,
                "rewrite_pause_p99_ms": during["p99_ms"],
            }
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()


def summary(records: list[dict]) -> str:
    baseline = next((r for r in records if r["policy"] == BASELINE), None)
    lines = [
        "| Policy | Throughput median req/s | vs AOF-off | batch p99 median us | PING p99 before ms | PING p99 during rewrite ms |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for record in records:
        ratio = "-"
        if baseline and baseline["throughput_median_ops"]:
            ratio = f"{record['throughput_median_ops'] / baseline['throughput_median_ops']:.3f}"
        record["contention_ratio_vs_baseline"] = ratio
        lines.append(
            f"| {record['policy']} | {record['throughput_median_ops']} | {ratio} | "
            f"{record['batch_p99_median_us']} | {record['ping_before']['p99_ms']} | "
            f"{record['rewrite_pause_p99_ms']} |"
        )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/rillcache")
    parser.add_argument("--bench", default="target/release/rillcache-bench")
    parser.add_argument("--requests", type=int, default=20000)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--pipeline", type=int, default=1)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--samples", type=int, default=PING_SAMPLES)
    parser.add_argument("--timeout", type=float, default=600.0)
    parser.add_argument("--json", default=None, help="write the records to this path")
    parser.add_argument("--dry-run", action="store_true", help="print the plan and exit")
    args = parser.parse_args(argv)

    binary = Path(args.binary)
    bench = Path(args.bench)
    if args.dry_run:
        print(f"policies: {BASELINE} (AOF off), {', '.join(POLICIES)}")
        print(f"binary: {binary}")
        print(f"bench: {bench}")
        print(f"workload: set, requests={args.requests}, clients={args.clients}, "
              f"pipeline={args.pipeline}, runs={args.runs}, ping_samples={args.samples}")
        return 0
    if not binary.is_file():
        print(f"FAIL: missing server binary at {binary}; build it first", file=sys.stderr)
        return 2
    if not bench.is_file():
        print(f"FAIL: missing bench client at {bench}; build it first", file=sys.stderr)
        return 2

    records = []
    for policy in (BASELINE, *POLICIES):
        records.append(measure_policy(binary, bench, policy, args))
        print(f"measured {policy}", file=sys.stderr)
    print(summary(records), end="")
    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps({"policies": records}, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())

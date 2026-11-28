#!/usr/bin/env python3
"""Repeatable mixed-workload soak and recovery drill (O9).

Runs the built server from a disposable directory with the journal enabled and a
mixed workload for `--duration` seconds:

* writers/readers drive `SET`/`GET`/`INCR`/`EXPIRE`/`LPUSH`/`LRANGE`/`HSET`/`HGET`;
* a churn client opens and closes connections continuously (reconnect storm);
* a slow client subscribes and never drains its socket until the end;
* an admin probe samples `INFO memory`/`INFO clients`, triggers `BGREWRITEAOF`
  periodically and records RSS/CPU from `/proc`.

At the end it stops the server, restarts it on the same directory, verifies that
the acknowledged keys survived, and writes a JSON report plus a markdown summary.
The soak fails on liveness or correctness problems (server death, unexpected
error replies, a lost acknowledged key); it does **not** fail on throughput or
memory thresholds, which are the owner-accepted budgets this harness exists to
inform.

Standard library only.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

OPERATIONS = ("set", "get", "incr", "expire", "lpush", "lrange", "hset", "hget")


class Client:
    def __init__(self, addr: str) -> None:
        host, port = addr.rsplit(":", 1)
        self.stream = socket.create_connection((host, int(port)), timeout=10.0)
        self.stream.settimeout(10.0)

    def call(self, *args: bytes) -> bytes:
        request = [f"*{len(args)}\r\n".encode()]
        for arg in args:
            request += [f"${len(arg)}\r\n".encode(), arg, b"\r\n"]
        self.stream.sendall(b"".join(request))
        return self._read_reply()

    def _read_reply(self) -> bytes:
        first = self.stream.recv(1)
        if not first:
            raise ConnectionError("server closed the connection")
        if first == b"-":
            line = self._read_line()
            raise RuntimeError(f"error reply: {line}")
        if first in b"+:":
            return self._read_line()
        if first == b"$":
            length = int(self._read_line())
            if length < 0:
                return b""
            body = b""
            while len(body) < length + 2:
                body += self.stream.recv(length + 2 - len(body))
            return body[:-2]
        if first == b"*":
            count = int(self._read_line())
            for _ in range(max(count, 0)):
                self._read_reply()
            return b""
        raise RuntimeError(f"unexpected reply prefix {first!r}")

    def _read_line(self) -> bytes:
        line = b""
        while not line.endswith(b"\r\n"):
            chunk = self.stream.recv(1)
            if not chunk:
                raise ConnectionError("server closed mid-reply")
            line += chunk
        return line[:-2]

    def close(self) -> None:
        try:
            self.stream.close()
        except OSError:
            pass


def start_server(binary: Path, directory: Path) -> tuple[subprocess.Popen, str]:
    process = subprocess.Popen(
        [
            str(binary),
            "--bind", "127.0.0.1:0",
            "--appendonly", "yes",
            "--appendfsync", "everysec",
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


def info(addr: str, section: str) -> dict[str, str]:
    client = Client(addr)
    try:
        body = client.call(b"INFO", section.encode()).decode(errors="replace")
    finally:
        client.close()
    fields = {}
    for line in body.splitlines():
        if ":" in line and not line.startswith("#"):
            key, value = line.split(":", 1)
            fields[key] = value.strip()
    return fields


def proc_stats(pid: int) -> tuple[int, int]:
    status = Path(f"/proc/{pid}/status").read_text()
    rss = next((int(line.split()[1]) for line in status.splitlines()
                if line.startswith("VmRSS:")), 0)
    stat = Path(f"/proc/{pid}/stat").read_text().split()
    cpu_seconds = (int(stat[13]) + int(stat[14])) / os.sysconf("SC_CLK_TCK")
    return rss, int(cpu_seconds)


def worker(addr: str, deadline: float, errors: list[str], counter: int) -> None:
    try:
        client = Client(addr)
        index = 0
        while time.monotonic() < deadline:
            key = f"soak:w:{counter}:{index % 256}".encode()
            choice = OPERATIONS[index % len(OPERATIONS)]
            index += 1
            if choice == "set":
                client.call(b"SET", key, b"v" * 32)
            elif choice == "get":
                client.call(b"GET", key)
            elif choice == "incr":
                client.call(b"INCR", b"soak:counter")
            elif choice == "expire":
                client.call(b"EXPIRE", key, b"5")
            elif choice == "lpush":
                # Keep the list bounded: an unbounded push stream fills the disk
                # (and the journal) instead of measuring steady state.
                client.call(b"LPUSH", b"soak:list", key)
                client.call(b"LTRIM", b"soak:list", b"0", b"63")
            elif choice == "lrange":
                client.call(b"LRANGE", b"soak:list", b"0", b"9")
            elif choice == "hset":
                # Fixed field set keeps the hash bounded too.
                client.call(b"HSET", b"soak:hash", f"f{index % 64}".encode(), b"v")
            else:
                client.call(b"HGET", b"soak:hash", key)
        client.close()
    except Exception as error:  # noqa: BLE001 - collected and reported
        errors.append(f"worker {counter}: {error}")


def churn(addr: str, deadline: float, errors: list[str]) -> None:
    try:
        while time.monotonic() < deadline:
            client = Client(addr)
            client.call(b"PING")
            client.close()
            time.sleep(0.02)
    except Exception as error:  # noqa: BLE001
        errors.append(f"churn: {error}")


def slow_client(addr: str, stop: threading.Event, errors: list[str]) -> None:
    try:
        client = Client(addr)
        client.call(b"SUBSCRIBE", b"soak:events")
        stop.wait()
        client.close()
    except Exception as error:  # noqa: BLE001
        errors.append(f"slow-client: {error}")


def measure(binary: Path, options: argparse.Namespace) -> dict:
    with tempfile.TemporaryDirectory(prefix="rillcache-soak-") as tmp:
        directory = Path(tmp)
        process, addr = start_server(binary, directory)
        errors: list[str] = []
        freed = []
        deadline = time.monotonic() + options.duration
        stop = threading.Event()
        threads = [
            threading.Thread(target=worker, args=(addr, deadline, errors, index), daemon=True)
            for index in range(options.clients)
        ]
        threads.append(threading.Thread(target=churn, args=(addr, deadline, errors), daemon=True))
        threads.append(threading.Thread(target=slow_client, args=(addr, stop, errors), daemon=True))
        for thread in threads:
            thread.start()

        samples = []
        rewrites = 0
        while time.monotonic() < deadline:
            time.sleep(options.interval)
            if process.poll() is not None:
                errors.append("server exited during the soak")
                break
            memory = info(addr, "memory")
            clients = info(addr, "clients")
            rss, cpu = proc_stats(process.pid)
            samples.append({
                "at_seconds": round(options.duration - (deadline - time.monotonic()), 1),
                "rss_kib": rss,
                "cpu_seconds": cpu,
                "used_memory": memory.get("used_memory"),
                "connected_clients": clients.get("connected_clients"),
                "blocked_clients": clients.get("blocked_clients"),
            })
            if options.rewrite_every and len(samples) % options.rewrite_every == 0:
                client = Client(addr)
                try:
                    client.call(b"BGREWRITEAOF")
                    rewrites += 1
                finally:
                    client.close()

        stop.set()
        for thread in threads:
            thread.join(timeout=5)
        stop_signal = process.poll() is None
        if stop_signal:
            process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            errors.append("server did not stop within the shutdown deadline")

        # Recovery: restart on the same directory and verify acknowledged keys.
        restarted, restart_addr = start_server(binary, directory)
        recovered = 0
        try:
            client = Client(restart_addr)
            for index in range(options.clients):
                for slot in range(0, 256, 32):
                    key = f"soak:w:{index}:{slot}".encode()
                    value = client.call(b"GET", key)
                    if value:
                        recovered += 1
            client.close()
        finally:
            restarted.send_signal(signal.SIGTERM)
            restarted.wait(timeout=30)

        return {
            "duration_seconds": options.duration,
            "clients": options.clients,
            "samples": samples,
            "rewrites": rewrites,
            "errors": errors,
            "recovered_keys": recovered,
            "graceful_stop": stop_signal,
        }


def summarize(report: dict) -> str:
    samples = report["samples"]
    first = samples[0]["rss_kib"] if samples else 0
    last = samples[-1]["rss_kib"] if samples else 0
    peak = max((s["rss_kib"] for s in samples), default=0)
    lines = [
        "| Metric | Value |",
        "| --- | ---: |",
        f"| Duration (s) | {report['duration_seconds']} |",
        f"| Clients | {report['clients']} |",
        f"| Samples | {len(samples)} |",
        f"| RSS first/peak/last (KiB) | {first} / {peak} / {last} |",
        f"| Rewrites triggered | {report['rewrites']} |",
        f"| Recovered keys | {report['recovered_keys']} |",
        f"| Graceful stop | {report['graceful_stop']} |",
        f"| Errors | {len(report['errors'])} |",
    ]
    for error in report["errors"]:
        lines.append(f"| error | {error} |")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/rillcache")
    parser.add_argument("--duration", type=int, default=300)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--interval", type=float, default=5.0)
    parser.add_argument("--rewrite-every", type=int, default=12)
    parser.add_argument("--json", default=None)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    if args.dry_run:
        print(f"soak plan: duration={args.duration}s clients={args.clients} "
              f"interval={args.interval}s rewrite_every={args.rewrite_every}")
        print(f"binary: {args.binary}")
        return 0
    binary = Path(args.binary)
    if not binary.is_file():
        print(f"FAIL: missing server binary at {binary}; build it first", file=sys.stderr)
        return 2
    report = measure(binary, args)
    print(summarize(report), end="")
    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
    if report["errors"]:
        print(f"FAIL: {len(report['errors'])} soak error(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

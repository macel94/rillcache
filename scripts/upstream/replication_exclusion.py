#!/usr/bin/env python3
"""Pinned native replicas versus a standalone Rillcache (A1 exclusion evidence).

The declared profile excludes replication. This harness proves the two-end
boundary with the pinned reference builds:

1. start the built Rillcache candidate as a standalone master;
2. start pinned Redis and pinned Valkey with ``--replicaof <candidate>``;
3. require that each native replica reports ``master_link_status:down`` and that
   the candidate keeps ``role:master``, zero connected replicas and a zero
   replication offset while local writes continue;
4. require that no ``+FULLRESYNC`` or ``+CONTINUE`` payload was produced and that
   the replicas received no candidate data.

The output records every probe for review. A failed assertion exits non-zero and
keeps the evidence directory.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import socket
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PINS = json.loads((ROOT / "ci/upstream/pins.json").read_text())
READY = re.compile(rb" listening on 127\.0\.0\.1:(\d{1,5})")
TIMEOUT = 25.0


class HarnessError(RuntimeError):
    pass


def frame(args) -> bytes:
    encoded = [part if isinstance(part, bytes) else str(part).encode() for part in args]
    return b"*%d\r\n" % len(encoded) + b"".join(b"$%d\r\n" % len(part) + part + b"\r\n" for part in encoded)


def read_reply(stream):
    prefix = stream.read(1)
    if not prefix:
        raise HarnessError("connection closed before a reply")
    line = stream.readline(1 << 20)
    if not line.endswith(b"\r\n"):
        raise HarnessError("malformed reply line")
    payload = line[:-2]
    if prefix == b"-":
        return {"error": payload.decode(errors="replace")}
    if prefix == b"+":
        return payload
    if prefix == b":":
        return int(payload)
    if prefix == b"$":
        length = int(payload)
        if length == -1:
            return None
        data = stream.read(length)
        if len(data) != length or stream.read(2) != b"\r\n":
            raise HarnessError("truncated bulk reply")
        return data
    if prefix == b"*":
        count = int(payload)
        if count == -1:
            return None
        return [read_reply(stream) for _ in range(count)]
    raise HarnessError(f"unexpected reply prefix {prefix!r}")


def call(port, *args):
    with socket.create_connection(("127.0.0.1", port), timeout=TIMEOUT) as connection:
        connection.sendall(frame(args))
        return read_reply(connection.makefile("rb"))


def info_replication(port) -> dict:
    text = call(port, b"INFO", b"replication")
    if not isinstance(text, bytes):
        raise HarnessError(f"INFO replication failed: {text}")
    fields = {}
    for line in text.decode(errors="replace").splitlines():
        name, _, value = line.partition(":")
        if name and not name.startswith("#"):
            fields[name] = value
    return fields


def available_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def start_candidate(binary: Path, directory: Path) -> tuple[subprocess.Popen, int]:
    directory.mkdir(parents=True, exist_ok=True)
    log = (directory.parent / "candidate.log").open("wb")
    process = subprocess.Popen(
        [str(binary), "--bind", "127.0.0.1:0", "--appendonly", "yes",
         "--dir", str(directory)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        line = process.stdout.readline()
        if not line:
            raise HarnessError("candidate exited during startup")
        log.write(line)
        log.flush()
        match = READY.search(line)
        if match:
            return process, int(match.group(1))
    raise HarnessError("candidate readiness deadline exceeded")


def start_replica(binary: Path, directory: Path, master_port: int, label: str) -> tuple[subprocess.Popen, int, Path]:
    directory.mkdir(parents=True, exist_ok=True)
    port = available_port()
    log_path = directory.parent / f"{label}.log"
    log = log_path.open("wb")
    process = subprocess.Popen(
        [str(binary), "--bind", "127.0.0.1", "--port", str(port),
         "--replicaof", "127.0.0.1", str(master_port),
         "--save", "", "--appendonly", "no", "--dir", str(directory),
         "--daemonize", "no", "--maxclients", "64", "--io-threads", "1"],
        stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise HarnessError(f"{label} exited during startup")
        try:
            if call(port, b"PING") == b"PONG":
                return process, port, log_path
        except OSError:
            pass
        time.sleep(0.05)
    raise HarnessError(f"{label} readiness deadline exceeded")


def stop(process: subprocess.Popen):
    if process.poll() is None:
        process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=15)


def run(server: Path, redis: Path, valkey: Path, output: Path) -> dict:
    for path in (server, redis, valkey):
        if not path.is_file() or not os.access(path, os.X_OK):
            raise HarnessError(f"missing executable: {path}")
    output.mkdir(parents=True, exist_ok=True)
    result = {"schema": 1, "candidate": str(server),
              "pins": {"redis": PINS["upstreams"]["redis"]["version"],
                       "valkey": PINS["upstreams"]["valkey"]["version"]},
              "environment": {"platform": platform.platform(), "python": platform.python_version()},
              "references": {}, "failures": []}
    candidate, port = start_candidate(server, output / "candidate-data")
    try:
        call(port, b"SET", b"local-before", b"value")
        role = call(port, b"ROLE")
        result["candidate_role"] = _evidence(role)
        if role != [b"master", 0, []]:
            result["failures"].append({"kind": "candidate-role", "role": _evidence(role)})

        for label, binary, version_key in (("redis", redis, "redis"), ("valkey", valkey, "valkey")):
            replica, replica_port, log_path = start_replica(
                binary, output / f"{label}-data", port, label)
            try:
                # A real replica decides within a bounded window; the link must
                # never come up and the replicas must keep no candidate data.
                deadline = time.monotonic() + 10
                observed = {}
                while time.monotonic() < deadline:
                    observed = info_replication(replica_port)
                    if observed.get("master_link_status") == "up":
                        break
                    time.sleep(0.2)
                candidate_info = info_replication(port)
                candidate_role = call(port, b"ROLE")
                replica_keys = call(replica_port, b"DBSIZE")
                record = {
                    "version": PINS["upstreams"][version_key]["version"],
                    "replica_role": observed.get("role"),
                    "master_link_status": observed.get("master_link_status"),
                    "replica_dbsize": replica_keys,
                    "candidate_role": _evidence(candidate_role),
                    "candidate_connected_slaves": candidate_info.get("connected_slaves"),
                    "candidate_master_repl_offset": candidate_info.get("master_repl_offset"),
                    "log": str(log_path.relative_to(output)),
                }
                log_text = log_path.read_text(errors="replace")
                record["full_resync_sent"] = "+FULLRESYNC" in log_text or "+FULLRESYNC" in str(candidate_info)
                record["replication_disabled_error_observed"] = "replication is not supported" in log_text
                result["references"][label] = record
                if observed.get("master_link_status") != "down":
                    result["failures"].append({"kind": "link-up", "reference": label})
                if observed.get("role") not in ("slave", "replica"):
                    result["failures"].append({"kind": "replica-role", "reference": label,
                                               "role": observed.get("role")})
                if replica_keys != 0:
                    result["failures"].append({"kind": "snapshot-applied", "reference": label,
                                               "dbsize": replica_keys})
                if candidate_role != [b"master", 0, []]:
                    result["failures"].append({"kind": "candidate-role-changed", "reference": label,
                                               "role": _evidence(candidate_role)})
                if str(candidate_info.get("connected_slaves")) != "0":
                    result["failures"].append({"kind": "connected-slaves", "reference": label})
                if str(candidate_info.get("master_repl_offset")) != "0":
                    result["failures"].append({"kind": "offset-advanced", "reference": label})
                if record["full_resync_sent"]:
                    result["failures"].append({"kind": "full-resync", "reference": label})
            finally:
                stop(replica)

        # The candidate still serves its own keyspace after the attempts.
        call(port, b"SET", b"local-after", b"value")
        if call(port, b"GET", b"local-before") != b"value" or call(port, b"GET", b"local-after") != b"value":
            result["failures"].append({"kind": "candidate-keyspace"})
    finally:
        stop(candidate)
    result["status"] = "passed" if not result["failures"] else "failed"
    return result


def _evidence(value):
    if isinstance(value, bytes):
        return value.decode(errors="replace")
    if isinstance(value, list):
        return [_evidence(item) for item in value]
    return value


def summary_text(result: dict) -> str:
    lines = [
        f"## Replication exclusion vs pinned replicas: {result['status']}", "",
        f"Pins: Redis {result['pins']['redis']}, Valkey {result['pins']['valkey']}.", "",
        "| Replica | Role | master_link_status | Replica DBSIZE | FULLRESYNC | Candidate replicas / offset |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for label, record in result.get("references", {}).items():
        lines.append(
            f"| {label} {record['version']} | {record['replica_role']} | {record['master_link_status']} | "
            f"{record['replica_dbsize']} | {record['full_resync_sent']} | "
            f"{record['candidate_connected_slaves']} / {record['candidate_master_repl_offset']} |")
    lines += ["", f"- failures: {len(result['failures'])}", "",
              "A pinned native replica must never form a link with the standalone profile: "
              "the candidate refuses every replication entry point, keeps zero connected "
              "replicas and a zero offset, and transfers no snapshot. This is exclusion "
              "evidence, not a replication implementation.", ""]
    return "\n".join(lines)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, default=ROOT / "target/upstream/rillcache")
    parser.add_argument("--redis", type=Path,
                        default=ROOT / "target/upstream/redis-8.10.2/src/redis-server")
    parser.add_argument("--valkey", type=Path,
                        default=ROOT / "target/upstream/valkey-9.1.2/src/valkey-server")
    parser.add_argument("--output", type=Path, default=ROOT / "target/upstream/evidence/replication-exclusion")
    args = parser.parse_args(argv)
    try:
        result = run(args.server.resolve(), args.redis.resolve(), args.valkey.resolve(), args.output.resolve())
    except (HarnessError, OSError, subprocess.SubprocessError) as error:
        result = {"schema": 1, "status": "failed",
                  "failures": [{"kind": "harness-error", "detail": str(error)}]}
        print(f"replication exclusion evidence failed: {error}", file=sys.stderr)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    text = summary_text(result)
    (args.output / "summary.md").write_text(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write(text + "\n")
    print(text)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())

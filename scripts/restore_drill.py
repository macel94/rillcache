#!/usr/bin/env python3
"""Measured, isolated backup/restore drill for gap D5.

The drill runs the real server and the real ``rillcache-backup`` tool:

1. load a mixed dataset (binary strings, collections, streams, absolute TTLs,
   counters, consumer groups) into an isolated source instance;
2. stop the source, take a quiescent archive, verify it, and confirm that an
   online `create` is refused while a writer runs (the profile's explicit
   exclusion, enforced rather than only documented);
3. restore into a separate isolated directory, start a second instance from it
   and compare the exact key set, per-key type/value bytes, absolute expirations,
   ``DBSIZE``, ``INFO keyspace`` accounting and the logical payload accounting;
4. record recovery-point (RPO), recovery-time (RTO) components in milliseconds,
   archive sizes and the tested retention rule (keep the newest two verified
   archives, restore the newest, prove the pruned one is gone).

It writes ``result.json`` and ``summary.md`` and exits non-zero on any mismatch.
The numbers are evidence for the tested head and dataset, not a service promise.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import random
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
READY = re.compile(rb" listening on 127\.0\.0\.1:(\d{1,5})")
TIMEOUT = 30.0


class DrillError(RuntimeError):
    pass


def now_ms() -> int:
    return int(time.time() * 1000)


class Client:
    """Minimal RESP2 client; bounded reads only."""

    def __init__(self, port: int):
        import socket

        self.socket = socket.create_connection(("127.0.0.1", port), timeout=TIMEOUT)
        self.stream = self.socket.makefile("rb")

    def call(self, *args):
        self.socket.sendall(frame(args))
        return read_reply(self.stream)

    def close(self):
        self.stream.close()
        self.socket.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def frame(args) -> bytes:
    encoded = [part if isinstance(part, bytes) else str(part).encode() for part in args]
    return b"*%d\r\n" % len(encoded) + b"".join(b"$%d\r\n" % len(part) + part + b"\r\n" for part in encoded)


def read_reply(stream):
    prefix = stream.read(1)
    if not prefix:
        raise DrillError("connection closed before a reply")
    line = stream.readline(1 << 20)
    if not line.endswith(b"\r\n"):
        raise DrillError("malformed reply line")
    payload = line[:-2]
    if prefix == b"-":
        raise DrillError(payload.decode(errors="replace"))
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
            raise DrillError("truncated bulk reply")
        return data
    if prefix == b"*":
        count = int(payload)
        if count == -1:
            return None
        return [read_reply(stream) for _ in range(count)]
    raise DrillError(f"unexpected reply prefix {prefix!r}")


class Server:
    def __init__(self, binary: Path, directory: Path, label: str):
        self.binary = binary
        self.directory = directory
        self.label = label
        directory.mkdir(parents=True, exist_ok=True)
        self.log = (directory.parent / f"{label}.log").open("wb")
        start = time.monotonic()
        self.process = subprocess.Popen(
            [str(binary), "--bind", "127.0.0.1:0", "--appendonly", "yes",
             "--appendfsync", "always", "--dir", str(directory)],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
        self.port = self._await_ready()
        self.startup_ms = round((time.monotonic() - start) * 1000, 3)

    def _await_ready(self) -> int:
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            line = self.process.stdout.readline()
            if not line:
                raise DrillError(f"{self.label}: server exited during startup")
            self.log.write(line)
            self.log.flush()
            match = READY.search(line)
            if match:
                return int(match.group(1))
        raise DrillError(f"{self.label}: readiness deadline exceeded")

    def client(self) -> Client:
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            try:
                client = Client(self.port)
                if client.call("PING") == b"PONG":
                    return client
                client.close()
            except OSError:
                pass
            time.sleep(0.02)
        raise DrillError(f"{self.label}: no PING")

    def stop(self) -> int:
        self.process.send_signal(signal.SIGTERM)
        try:
            code = self.process.wait(timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            self.process.kill()
            code = self.process.wait(timeout=TIMEOUT)
        self.log.close()
        return code

    def kill(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=TIMEOUT)


def run_cli(argv: list[str]) -> dict:
    """Run a tool command and record its exact argv, output and duration."""
    start = time.monotonic()
    completed = subprocess.run(argv, capture_output=True, timeout=300, check=False)
    return {
        "argv": argv,
        "exit_code": completed.returncode,
        "milliseconds": round((time.monotonic() - start) * 1000, 3),
        "stdout": completed.stdout.decode(errors="replace").strip(),
        "stderr": completed.stderr.decode(errors="replace").strip(),
    }


def require(condition, message):
    if not condition:
        raise DrillError(message)


def dataset(seed: int, keys: int, now: int | None = None):
    """Deterministic mixed dataset with absolute expirations.

    ``now`` is injectable so a test can assert byte-identical commands; the
    drill passes a single timestamp for one consistent expiration base.
    """
    rng = random.Random(seed)
    expires_at = (now if now is not None else now_ms()) + 3_600_000  # beyond the drill window
    commands = []
    expectations = {}
    for index in range(keys):
        kind = index % 7
        key = f"drill:{index}".encode()
        if kind == 0:
            value = bytes([index % 256, 0x00, 0xFF, 0x0D, 0x0A]) + f"value-{index}".encode()
            commands.append(["SET", key, value])
            expectations[key] = {"type": "string", "read": ["GET", key], "value": value}
        elif kind == 1:
            commands.extend([
                ["RPUSH", key, b"a", b"b", f"c-{index}".encode()],
                ["LRANGE", key, "0", "-1"],
            ])
            expectations[key] = {"type": "list", "read": ["LRANGE", key, "0", "-1"],
                                 "value": [b"a", b"b", f"c-{index}".encode()]}
        elif kind == 2:
            commands.append(["HSET", key, b"field", f"v-{index}".encode(), b"other", b"\x00\xff"])
            expectations[key] = {"type": "hash", "read": ["HGETALL", key],
                                 "pairs": {b"field": f"v-{index}".encode(), b"other": b"\x00\xff"}}
        elif kind == 3:
            commands.append(["SADD", key, b"one", b"two", f"three-{index}".encode()])
            expectations[key] = {"type": "set", "read": ["SMEMBERS", key],
                                 "set": {b"one", b"two", f"three-{index}".encode()}}
        elif kind == 4:
            commands.append(["ZADD", key, "1.5", b"low", "2.5", b"high"])
            expectations[key] = {"type": "zset", "read": ["ZRANGE", key, "0", "-1", "WITHSCORES"],
                                 "value": [b"low", b"1.5", b"high", b"2.5"]}
        elif kind == 5:
            commands.extend([
                ["XADD", key, "1-1", b"f", f"x-{index}".encode()],
                ["XADD", key, "2-2", b"f", b"second"],
                ["XGROUP", "CREATE", key, b"group", "0"],
                ["XREADGROUP", "GROUP", b"group", b"consumer", "COUNT", "1", "STREAMS", key, ">"],
            ])
            expectations[key] = {"type": "stream", "read": ["XRANGE", key, "-", "+"],
                                 "entries": 2}
        else:
            commands.append(["SET", key, str(index)])
            commands.append(["INCRBY", key, "41"])
            expectations[key] = {"type": "string", "read": ["GET", key], "value": str(index + 41).encode()}
        if index % 5 == 0:
            commands.append(["PEXPIREAT", key, str(expires_at + rng.randrange(0, 1000))])
            expectations[key]["expire"] = True
    return commands, expectations, expires_at


def load(server: Server, commands, expectations) -> dict:
    with server.client() as client:
        pipeline = b"".join(frame(command) for command in commands)
        client.socket.sendall(pipeline)
        for _ in commands:
            read_reply(client.stream)
        return snapshot(client, expectations)


def snapshot(client: Client, expectations) -> dict:
    """Exact observable state: types, values, absolute expirations, accounting."""
    state = {"keys": {}, "db0": None, "payload": None, "observed_at_ms": now_ms()}
    for key, expected in expectations.items():
        kind = client.call("TYPE", key)
        payload = {"type": kind.decode()}
        if "pairs" in expected:
            flat = client.call(*expected["read"])
            payload["pairs"] = {flat[i]: flat[i + 1] for i in range(0, len(flat), 2)}
        elif "set" in expected:
            payload["set"] = sorted(client.call(*expected["read"]))
        elif "entries" in expected:
            payload["entries"] = len(client.call(*expected["read"]))
        else:
            payload["value"] = client.call(*expected["read"])
        payload["pexpiretime"] = client.call("PEXPIRETIME", key)
        state["keys"][key] = payload
    state["db0"] = client.call("INFO", "keyspace").decode(errors="replace")
    stats = client.call("MEMORY", "STATS")
    state["payload"] = {stats[i].decode(): stats[i + 1] for i in range(0, len(stats), 2)}
    return state


def db0_line(info: str) -> str:
    for line in info.splitlines():
        if line.startswith("db0:"):
            return line.strip()
    raise DrillError("INFO keyspace has no db0 line")


def db0_components(info: str) -> dict:
    """Split the keyspace report; avg_ttl moves with the wall clock."""
    components = {}
    for field in db0_line(info).removeprefix("db0:").split(","):
        name, _, value = field.partition("=")
        components[name] = int(value) if value.isdigit() else value
    return components


def compare(source: dict, restored: dict, avg_ttl_tolerance_ms: int = 0) -> list[dict]:
    problems = []
    if set(source["keys"]) != set(restored["keys"]):
        missing = sorted(set(source["keys"]) - set(restored["keys"]))[:5]
        extra = sorted(set(restored["keys"]) - set(source["keys"]))[:5]
        problems.append({"kind": "keyset", "missing": [k.decode(errors="replace") for k in missing],
                         "extra": [k.decode(errors="replace") for k in extra]})
    for key in sorted(set(source["keys"]) & set(restored["keys"])):
        if source["keys"][key] != restored["keys"][key]:
            problems.append({"kind": "value", "key": key.decode(errors="replace"),
                             "source": {k: _jsonable(v) for k, v in source["keys"][key].items()},
                             "restored": {k: _jsonable(v) for k, v in restored["keys"][key].items()}})
    source_db0 = db0_components(source["db0"])
    restored_db0 = db0_components(restored["db0"])
    for field in ("keys", "expires", "subexpiry"):
        if source_db0.get(field) != restored_db0.get(field):
            problems.append({"kind": "keyspace-accounting", "field": field,
                             "source": source_db0.get(field), "restored": restored_db0.get(field)})
    # avg_ttl is recomputed from the live clock, so it decays with the elapsed
    # drill time plus any cached clock read; the per-key absolute expirations
    # are compared exactly above, so the aggregate only needs a bounded window.
    if abs(source_db0.get("avg_ttl", 0) - restored_db0.get("avg_ttl", 0)) > avg_ttl_tolerance_ms:
        problems.append({"kind": "keyspace-avg-ttl",
                         "source": source_db0.get("avg_ttl"),
                         "restored": restored_db0.get("avg_ttl"),
                         "tolerance_ms": avg_ttl_tolerance_ms})
    for field in ("keys.count", "dataset.bytes"):
        if source["payload"].get(field) != restored["payload"].get(field):
            problems.append({"kind": "payload-accounting", "field": field,
                             "source": source["payload"].get(field),
                             "restored": restored["payload"].get(field)})
    return problems


def _jsonable(value):
    if isinstance(value, bytes):
        return {"bytes_hex": value.hex()}
    if isinstance(value, list):
        return [_jsonable(item) for item in value]
    if isinstance(value, dict):
        return {k.decode(errors="replace") if isinstance(k, bytes) else k: _jsonable(v) for k, v in value.items()}
    if isinstance(value, tuple):
        return [_jsonable(item) for item in value]
    return value


def mutate(server: Server, expectations) -> None:
    """Second recovery point: overwrite and add keys after the first archive."""
    with server.client() as client:
        client.call("SET", b"drill:0", b"mutated-after-first-archive")
        client.call("SET", b"drill:new", b"second-round")
    expectations[b"drill:0"] = {"type": "string", "read": ["GET", b"drill:0"],
                                "value": b"mutated-after-first-archive"}
    expectations[b"drill:new"] = {"type": "string", "read": ["GET", b"drill:new"],
                                  "value": b"second-round"}


def drill(server_bin: Path, backup_bin: Path, output: Path, keys: int, seed: int, retain: int) -> dict:
    for binary in (server_bin, backup_bin):
        require(binary.is_file() and os.access(binary, os.X_OK), f"missing executable {binary}")
    output.mkdir(parents=True, exist_ok=True)
    work = output / "work"
    if work.exists():
        shutil.rmtree(work)
    source_dir = work / "source"
    restore_dir = work / "restored"
    archive_dir = work / "archives"
    archive_dir.mkdir(parents=True)
    restore_dir.mkdir()
    result = {"schema": 1, "candidate": {"server": str(server_bin), "backup": str(backup_bin)},
              "dataset": {"keys": keys, "seed": seed}, "archives": [], "timings_ms": {},
              "rpo": {}, "retention": {}, "environment": {"platform": platform.platform(),
                                                          "python": platform.python_version()},
              "failures": []}
    commands, expectations, _ = dataset(seed, keys)
    started = time.monotonic()

    # Recovery point 1: quiescent copy of the loaded dataset.
    source = Server(server_bin, source_dir, "source")
    result["timings_ms"]["source_startup"] = source.startup_ms
    result["state"] = load(source, commands, expectations)
    require(source.stop() == 0, "the source must stop cleanly before a quiescent copy")
    journal = source_dir / "appendonly.aof"

    archive_one = archive_dir / "snapshot-1.rcb"
    created = run_cli([str(backup_bin), "create", "--dir", str(source_dir), "--output", str(archive_one)])
    require(created["exit_code"] == 0, f"create failed: {created['stderr']}")
    result["timings_ms"]["create"] = created["milliseconds"]
    verified = run_cli([str(backup_bin), "verify", "--input", str(archive_one)])
    require(verified["exit_code"] == 0, f"verify failed: {verified['stderr']}")
    result["timings_ms"]["verify"] = verified["milliseconds"]

    # The profile excludes online copies: creation must refuse a live writer.
    live = Server(server_bin, source_dir, "source-live")
    online = run_cli([str(backup_bin), "create", "--dir", str(source_dir),
                      "--output", str(archive_dir / "online.rcb")])
    result["online_copy"] = {"refused": online["exit_code"] != 0, "stderr": online["stderr"][:300]}
    require(online["exit_code"] != 0, "an online copy was accepted; the profile excludes it")
    require(not (archive_dir / "online.rcb").exists(), "a refused online copy must not publish an archive")
    # Second recovery point: mutate after the first archive.
    mutate(live, expectations)
    point_two = dict(expectations)
    result["state_second"] = snapshot_checked(live, expectations)
    require(live.stop() == 0, "the second source must stop cleanly")
    archive_two = archive_dir / "snapshot-2.rcb"
    created = run_cli([str(backup_bin), "create", "--dir", str(source_dir), "--output", str(archive_two)])
    require(created["exit_code"] == 0, f"second create failed: {created['stderr']}")
    result["timings_ms"]["create_second"] = created["milliseconds"]
    # The restore must never mutate the source journal.
    source_bytes = journal.read_bytes()

    # Restore the newest archive into an isolated instance and validate it.
    restored = run_cli([str(backup_bin), "restore", "--input", str(archive_two), "--dir", str(restore_dir)])
    require(restored["exit_code"] == 0, f"restore failed: {restored['stderr']}")
    result["timings_ms"]["restore"] = restored["milliseconds"]
    require(sorted(path.name for path in restore_dir.iterdir()) == ["appendonly.aof"],
            "restore must leave only the restored journal, no staging residue")
    require(journal.read_bytes() == source_bytes, "the source journal changed during the drill")
    isolated = Server(server_bin, restore_dir, "isolated")
    with isolated.client() as client:
        state = snapshot(client, expectations)
    tolerance = max(5000, state["observed_at_ms"] - result["state_second"]["observed_at_ms"] + 5000)
    problems = compare(result["state_second"], state, tolerance)
    if problems:
        result["failures"].extend(problems)
    require(not problems, f"restored instance differs from the source: {problems[:2]}")
    result["timings_ms"]["isolated_startup"] = isolated.startup_ms
    isolated.stop()
    result["rpo"] = {"quiescent_acknowledged_writes_lost": 0,
                     "online_copy": "excluded (create refuses a live writer)",
                     "point_in_time_recovery": "excluded (no archive history replay)"}

    # Third recovery point so the retention rule actually prunes and the
    # retained pair can be validated independently.
    third = Server(server_bin, source_dir, "source-third")
    with third.client() as client:
        client.call("SET", b"drill:third", b"third-round")
    expectations[b"drill:third"] = {"type": "string", "read": ["GET", b"drill:third"],
                                    "value": b"third-round"}
    point_three = dict(expectations)
    third_state = snapshot_checked(third, expectations)
    require(third.stop() == 0, "the third source must stop cleanly")
    archive_three = archive_dir / "snapshot-3.rcb"
    created = run_cli([str(backup_bin), "create", "--dir", str(source_dir), "--output", str(archive_three)])
    require(created["exit_code"] == 0, f"third create failed: {created['stderr']}")
    result["timings_ms"]["create_third"] = created["milliseconds"]

    # Retention rule: keep the newest `retain` verified archives, prune the rest.
    archives = sorted(archive_dir.glob("snapshot-*.rcb"), key=lambda path: path.name)
    pruned = []
    while len(archives) > retain:
        oldest = archives.pop(0)
        oldest.unlink()
        pruned.append(oldest.name)
    for archive in archives:
        verified = run_cli([str(backup_bin), "verify", "--input", str(archive)])
        require(verified["exit_code"] == 0, f"retained archive {archive.name} does not verify")
    require(all(not (archive_dir / name).exists() for name in pruned), "pruned archives must be gone")
    require(len(archives) == retain, "retention must keep exactly the configured number of archives")
    result["retention"] = {"policy": f"keep the newest {retain} verified archives; operator copies pinned externally",
                           "retained": [archive.name for archive in archives], "pruned": pruned,
                           "archive_bytes": {archive.name: archive.stat().st_size for archive in archives},
                           "restored_points": {}}

    # Each retained archive must reproduce its own recovery point in isolation.
    for archive, (expected_keys, expected) in zip(archives, [(point_two, result["state_second"]),
                                                             (point_three, third_state)]):
        directory = work / f"restored-{archive.stem}"
        directory.mkdir()
        restored_point = run_cli([str(backup_bin), "restore", "--input", str(archive), "--dir", str(directory)])
        require(restored_point["exit_code"] == 0, f"restore of {archive.name} failed: {restored_point['stderr']}")
        instance = Server(server_bin, directory, f"isolated-{archive.stem}")
        with instance.client() as client:
            state = snapshot(client, expected_keys)
        instance.stop()
        tolerance = max(5000, state["observed_at_ms"] - expected["observed_at_ms"] + 5000)
        problems = compare(expected, state, tolerance)
        require(not problems, f"{archive.name} does not reproduce its recovery point: {problems[:2]}")
        result["retention"]["restored_points"][archive.name] = {
            "keys": len(state["keys"]), "db0": db0_line(state["db0"]),
            "restore_ms": restored_point["milliseconds"]}

    result["timings_ms"]["total"] = round((time.monotonic() - started) * 1000, 3)
    result["rto"] = {
        "measured_total_ms": result["timings_ms"]["total"],
        "restore_to_serving_ms": result["timings_ms"]["restore"] + result["timings_ms"]["isolated_startup"],
        "quiescent_create_ms": result["timings_ms"]["create"],
        "note": "measured on this host and dataset; sizes scale with the archive, not with the store's history beyond the journal",
    }
    result["status"] = "passed" if not result["failures"] else "failed"
    return jsonable_result(result)


def jsonable_result(value):
    """Convert the evidence tree to JSON-safe types without losing bytes."""
    if isinstance(value, bytes):
        return {"bytes_hex": value.hex()}
    if isinstance(value, dict):
        return {(key.decode(errors="replace") if isinstance(key, bytes) else str(key)): jsonable_result(item)
                for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [jsonable_result(item) for item in value]
    if isinstance(value, set):
        return sorted(jsonable_result(item) for item in value)
    return value


def snapshot_checked(server: Server, expectations) -> dict:
    with server.client() as client:
        return snapshot(client, expectations)


def summary_text(result: dict) -> str:
    timings = result["timings_ms"]
    retention = result["retention"]
    return (
        f"## Isolated backup/restore drill: {result['status']}\n\n"
        f"Dataset {result['dataset']['keys']} keys (seed {result['dataset']['seed']}), "
        f"binary strings, collections, streams with a consumer group, counters and absolute TTLs.\n\n"
        f"| Measurement | Value |\n| --- | --- |\n"
        f"| Quiescent create | {timings['create']} ms |\n"
        f"| Archive verify | {timings['verify']} ms |\n"
        f"| Restore | {timings['restore']} ms |\n"
        f"| Isolated instance startup | {timings['isolated_startup']} ms |\n"
        f"| Restore-to-serving | {result['rto']['restore_to_serving_ms']} ms |\n"
        f"| Total drill | {timings['total']} ms |\n\n"
        f"- RPO (quiescent): {result['rpo']['quiescent_acknowledged_writes_lost']} acknowledged writes lost; "
        f"online copy {result['rpo']['online_copy']}; PITR {result['rpo']['point_in_time_recovery']}.\n"
        f"- Retention: {retention['policy']}; retained {retention['retained']}, pruned {retention['pruned']}.\n"
        f"- Each retained archive reproduced its recovery point in isolation: "
        f"{', '.join(f'{name} ({point['keys']} keys, {point['restore_ms']} ms)' for name, point in retention['restored_points'].items())}.\n"
        f"- Failures: {len(result['failures'])}.\n\n"
        "Measured on this host and dataset for this exact head. It is evidence, not an SLA: "
        "capacity, disk speed and archive size change the timings.\n")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, default=ROOT / "target/release/rillcache")
    parser.add_argument("--backup", type=Path, default=ROOT / "target/release/rillcache-backup")
    parser.add_argument("--output", type=Path, default=ROOT / "target/restore-drill")
    parser.add_argument("--keys", type=int, default=2000)
    parser.add_argument("--seed", type=int, default=459)
    parser.add_argument("--retain", type=int, default=2)
    args = parser.parse_args(argv)
    if args.keys < 50:
        parser.error("--keys must be at least 50")
    if args.retain < 1:
        parser.error("--retain must be at least 1")
    try:
        result = drill(args.server.resolve(), args.backup.resolve(), args.output.resolve(),
                       args.keys, args.seed, args.retain)
    except (DrillError, OSError, subprocess.SubprocessError) as error:
        result = {"schema": 1, "status": "failed",
                  "failures": [{"kind": "drill-error", "detail": str(error)}]}
        print(f"backup/restore drill failed: {error}", file=sys.stderr)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    text = summary_text(result) if result.get("timings_ms") else (
        f"## Isolated backup/restore drill: {result['status']}\n\n"
        + "\n".join(f"- {failure}" for failure in result["failures"]) + "\n")
    (args.output / "summary.md").write_text(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write(text + "\n")
    print(text)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Four-CPU PR gate using the unchanged, checksum-pinned valkey-benchmark.

Only orchestration/validation lives here. All timed traffic and throughput/latency
measurement comes from the same official Valkey executable for both servers.
"""
import argparse
import contextlib
import csv
import io
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time

sys.dont_write_bytecode = True
# The installation workflow explicitly supplies protected-base helpers too.
sys.path.insert(0, str(Path(os.environ.get("RILLCACHE_GATE_HELPERS", Path(__file__).parent / "upstream"))))
from common import Client, available_port, command, digest, process_stats, save, terminate
from performance import parse_csv

WORKLOADS = ("ping", "set", "get", "set-nx-existing", "set-xx-existing")
PIPELINES = (1, 16)
RUNS = 3
KEYSPACE = 1024
VALUE = b"v" * 64
REPLACEMENT = b"w" * 64
WARMUP = 8000
SHA = r"[0-9a-f]{40}"
CSV_HEADER = ["test", "rps", "avg_latency_ms", "min_latency_ms", "p50_latency_ms",
              "p95_latency_ms", "p99_latency_ms", "max_latency_ms"]
RAW_FILES = ("server.json", "server.log", "sample.json",
             "warmup/command.json", "warmup/stdout.log", "warmup/stderr.log",
             "measured/command.json", "measured/stdout.log", "measured/stderr.log")


def ensure(condition, message):
    if not condition:
        raise ValueError(message)


def read_json(path):
    def pairs(items):
        result = {}
        for key, value in items:
            ensure(key not in result, "duplicate JSON key")
            result[key] = value
        return result
    def invalid(value):
        raise ValueError(f"invalid JSON number: {value}")
    text = Path(path).read_text()
    ensure(len(text) <= 2 * 1024 * 1024, "oversized JSON evidence")
    def finite_float(text):
        value = float(text)
        ensure(math.isfinite(value), "nonfinite JSON number")
        return value
    return json.loads(text, object_pairs_hook=pairs, parse_constant=invalid, parse_float=finite_float)


def placement(cpus):
    ensure(len(cpus) >= 4 and all(type(c) is int and c >= 0 for c in cpus)
           and cpus == sorted(set(cpus)), "at least four distinct allowed logical CPUs required")
    return cpus[:4], cpus[4:] or cpus[:4]


def cpu_list(cpus):
    return ",".join(map(str, cpus))


def requests(pipeline):
    return 100000 if pipeline == 1 else 1000000


def workload_command(workload):
    key = "data:__rand_int__"
    return {"ping": ["PING"], "get": ["GET", key],
            "set": ["SET", key, REPLACEMENT.decode()],
            "set-nx-existing": ["SET", key, REPLACEMENT.decode(), "NX"],
            "set-xx-existing": ["SET", key, REPLACEMENT.decode(), "XX"]}[workload]


def generator_argv(binary, port, workload, pipeline, count, cpus):
    return ["taskset", "-c", cpu_list(cpus), str(binary), "-h", "127.0.0.1", "-p", str(port),
            "-c", "8", "-P", str(pipeline), "-n", str(count), "-r", str(KEYSPACE),
            "-d", "64", "-k", "1", "--threads", "2", "--seed", "42", "--csv", "--",
            *workload_command(workload)]


def validate_dataset(port, workload, *, preload=False, after=False):
    with Client(port) as client:
        ensure(client.call("PING") == b"PONG", "invalid PING reply")
        if workload == "ping":
            return {"validated_keys": 0, "ping": True}
        expected = REPLACEMENT if after and workload in ("set", "set-xx-existing") else VALUE
        # Preloading every key prevents an empty-GET or successful-NX benchmark.
        for start in range(0, KEYSPACE, 128):
            keys = [f"data:{i:012d}" for i in range(start, start + 128)]
            if preload:
                ensure(client.batch(("SET", key, VALUE) for key in keys) == [b"OK"] * 128, "preload failed")
            values = client.batch(("GET", key) for key in keys)
            if after and workload in ("set", "set-xx-existing"):
                # Random key selection need not touch every key. Both original
                # and replacement values are valid; unrelated/corrupt data isn't.
                ensure(all(value in (VALUE, expected) for value in values), "overwrite data corrupted")
            else:
                ensure(values == [expected] * 128, "hit/NX dataset changed or missing")
        # A direct probe checks the required reply contract outside timing.
        probe = "data:000000000000"
        if workload == "set-nx-existing":
            ensure(client.call("SET", probe, REPLACEMENT, "NX") is None, "NX existing must reject")
            ensure(client.call("GET", probe) == VALUE, "NX changed existing data")
        elif workload in ("set", "set-xx-existing"):
            options = ("XX",) if workload == "set-xx-existing" else ()
            ensure(client.call("SET", probe, expected, *options) == b"OK", "overwrite must succeed")
        return {"validated_keys": KEYSPACE, "ping": True}


def server_argv(binary, variant, port, cpus, directory):
    args = [str(binary), "--bind", f"127.0.0.1:{port}"] if variant == "rillcache" else [
        str(binary), "--bind", "127.0.0.1", "--port", str(port), "--save", "", "--appendonly", "no",
        "--daemonize", "no", "--loglevel", "warning", "--dir", str(directory)]
    return ["taskset", "-c", cpu_list(cpus), *args]


@contextlib.contextmanager
def owned_server(binary, variant, cpus, directory):
    port = available_port()
    argv = server_argv(binary, variant, port, cpus, directory)
    with (directory / "server.log").open("wb") as log:
        proc = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        identity = {"argv": argv, "port": port, "pid": proc.pid, "cpus": cpus}
        try:
            deadline = time.monotonic() + 10
            while True:
                ensure(proc.poll() is None, "owned server exited before readiness")
                try:
                    with Client(port) as client:
                        ensure(client.call("PING") == b"PONG", "invalid readiness reply")
                    break
                except (OSError, ConnectionError):
                    ensure(time.monotonic() < deadline, "server readiness deadline")
                    time.sleep(.01)
            identity["actual_cpus_before"] = sorted(os.sched_getaffinity(proc.pid))
            ensure(identity["actual_cpus_before"] == cpus, "actual server affinity differs")
            yield proc, port
            ensure(proc.poll() is None, "owned server exited during measurement")
            identity["actual_cpus_after"] = sorted(os.sched_getaffinity(proc.pid))
            ensure(identity["actual_cpus_after"] == cpus, "server affinity changed")
        finally:
            identity["exit_before_cleanup"] = proc.poll()
            identity["exit_after_cleanup"] = terminate(proc)
            identity["joined"] = True
            save(directory / "server.json", identity)


def read_generator(directory, workload, argv):
    result = read_json(directory / "command.json")
    ensure(result["argv"] == argv and type(result["exit_code"]) is int and result["exit_code"] == 0
           and not result.get("timed_out"), "load generator failed or command differs")
    stderr = (directory / "stderr.log").read_text()
    ensure(all(line == "WARNING: Could not fetch server CONFIG" for line in stderr.splitlines() if line.strip()),
           "unexpected generator stderr")
    text = (directory / "stdout.log").read_text()
    # Require the complete command title, including NX/XX, not just SET.
    reader = csv.DictReader(io.StringIO(text))
    ensure(reader.fieldnames == CSV_HEADER, "missing/duplicate/unexpected CSV column")
    rows = list(reader)
    ensure(len(rows) == 1 and rows[0].get("test") == " ".join(workload_command(workload)), "wrong benchmark command")
    return parse_csv(text, workload)


def sample(binaries, variant, workload, pipeline, cpus, client_cpus, directory):
    directory.mkdir()
    result = {}
    try:
        with owned_server(binaries[variant], variant, cpus, directory) as (proc, port):
            result.update(port=port, pid=proc.pid)
            result["preload"] = validate_dataset(port, workload, preload=True)
            for phase, count in (("warmup", WARMUP), ("measured", requests(pipeline))):
                argv = generator_argv(binaries["generator"], port, workload, pipeline, count, client_cpus)
                before = process_stats(proc.pid)
                start = time.monotonic()
                command(argv, directory / phase, timeout=180)
                metrics = read_generator(directory / phase, workload, argv)
                elapsed = time.monotonic() - start
                after = process_stats(proc.pid)
                result[phase] = metrics
                result[phase + "_validation"] = validate_dataset(port, workload, after=True)
                if phase == "measured":
                    result.update(server_cpu_seconds=after["cpu_seconds"] - before["cpu_seconds"],
                                  resource_window_seconds=elapsed, server_rss_kib=after["rss_kib"])
        result["valid"] = True
    except BaseException as error:
        result.update(valid=False, error=str(error))
        raise
    finally:
        save(directory / "sample.json", result)


def identities():
    for index, (workload, pipeline) in enumerate((w, p) for w in WORKLOADS for p in PIPELINES):
        for run in range(1, RUNS + 1):
            order = ("rillcache", "valkey") if (run + index) % 2 else ("valkey", "rillcache")
            for variant in order:
                yield workload, pipeline, run, variant


def name_for(workload, pipeline, run, variant):
    return f"{workload}-p{pipeline}-r{run}-{variant}"


def evaluate(samples):
    expected = set(identities())
    actual = [(s["workload"], s["pipeline"], s["run"], s["variant"]) for s in samples]
    ensure(len(actual) == len(expected) and set(actual) == expected, "missing/duplicate cohort sample")
    rows = []
    for workload in WORKLOADS:
        for pipeline in PIPELINES:
            row = {"workload": workload, "pipeline": pipeline}
            for variant in ("rillcache", "valkey"):
                records = [s for s in samples if (s["workload"], s["pipeline"], s["variant"]) == (workload, pipeline, variant)]
                rates = [s["rps"] for s in records]
                ensure(all(type(r) in (float, int) and 0 < r < float("inf") for r in rates), "invalid throughput")
                row[variant] = {"median": statistics.median(rates), "min": min(rates), "max": max(rates),
                                "p99_ms": statistics.median(s["p99_latency_ms"] for s in records)}
            row["passed"] = row["rillcache"]["median"] > row["valkey"]["median"]
            rows.append(row)
    return rows


def summary(rows):
    wins = sum(row["passed"] for row in rows)
    text = ["# Official Valkey benchmark: four-CPU PR gate", "",
            f"**{'PASS' if wins == 10 else 'FAIL'}: {wins}/10 medians strictly faster than Valkey.**", "",
            "Same pinned, unmodified valkey-benchmark for both default-mode servers; exactly four logical server CPUs. "
            "Eight clients, two generator threads, 1024 random keys, 64-byte values, seed 42, 8000 warmup commands, "
            "100k requests at P=1 / 1m at P=16; three alternating native repetitions. Ties or any loss fail.", "",
            "Client CPUs are disjoint when available, otherwise share the four server CPUs (see manifest). "
            "The upstream tool rejects error replies but does not validate every non-error payload; full-keyspace "
            "checks/probes run before and after load. p99 is upstream pipeline latency, not per-command latency at P>1. "
            "Shared-runner noise/client contention can block this scoped gate; no universal superiority claim.", "",
            "| Workload | P | Rillcache median [min–max] req/s | Valkey median [min–max] req/s | p99 ms R/V | Gate |",
            "| --- | ---: | ---: | ---: | ---: | --- |"]
    for row in rows:
        a, b = row["rillcache"], row["valkey"]
        text.append(f'| {row["workload"]} | {row["pipeline"]} | {a["median"]:.2f} [{a["min"]:.2f}–{a["max"]:.2f}] | '
                    f'{b["median"]:.2f} [{b["min"]:.2f}–{b["max"]:.2f}] | {a["p99_ms"]:.3f}/{b["p99_ms"]:.3f} | {"PASS" if row["passed"] else "FAIL"} |')
    return "\n".join(text) + "\n"


def load_cohort(directory):
    root = Path(directory).resolve()
    manifest = read_json(root / "manifest.json")
    ensure(type(manifest["schema_version"]) is int and manifest["schema_version"] == 1
           and manifest["complete"] is True, "incomplete gate evidence")
    server_cpus, client_cpus = placement(manifest["allowed_cpus"])
    ensure(manifest["server_cpus"] == server_cpus and manifest["client_cpus"] == client_cpus, "CPU allocation differs")
    ensure(type(manifest["runs"]) is int and manifest["runs"] == RUNS and manifest["workloads"] == list(WORKLOADS)
           and manifest["pipelines"] == list(PIPELINES), "gate policy differs")
    ensure(all(re.fullmatch(SHA, manifest[k]) for k in ("base_source", "head_source", "candidate_source")), "invalid source identity")
    ensure(set(manifest["binary_sha256"]) == {"rillcache", "valkey", "generator"}
           and all(re.fullmatch(r"[0-9a-f]{64}", v) for v in manifest["binary_sha256"].values()), "invalid binary hashes")
    samples = []
    expected_files = {f"{name_for(*identity)}/{f}" for identity in identities() for f in RAW_FILES}
    ensure(set(manifest["evidence_sha256"]) == expected_files, "missing/unexpected raw evidence")
    ensure(manifest["order"] == [name_for(*i) for i in identities()], "measurement order differs")
    for identity in identities():
        workload, pipeline, run, variant = identity
        directory = root / name_for(*identity)
        for file in RAW_FILES:
            relative = f"{directory.name}/{file}"
            ensure(digest(directory / file) == manifest["evidence_sha256"][relative], "raw evidence checksum differs")
        sample = read_json(directory / "sample.json")
        server = read_json(directory / "server.json")
        ensure(sample["valid"] is True and server["joined"] is True and server["exit_before_cleanup"] is None, "invalid server lifecycle")
        ensure(type(server["pid"]) is int and server["pid"] > 0 and sample["pid"] == server["pid"], "wrong process identity")
        port = sample["port"]
        ensure(type(port) is int and 0 < port < 65536 and server["port"] == port, "invalid endpoint")
        ensure(server["actual_cpus_before"] == server_cpus == server["actual_cpus_after"], "observed affinity differs")
        ensure(server["cpus"] == server_cpus and server["argv"] == server_argv(
            manifest["binaries"][variant], variant, port, server_cpus, Path(manifest["launch_directory"]) / directory.name), "server command/affinity differs")
        for phase, count in (("warmup", WARMUP), ("measured", requests(pipeline))):
            metrics = read_generator(directory / phase, workload, generator_argv(
                manifest["binaries"]["generator"], port, workload, pipeline, count, client_cpus))
            ensure(sample[phase] == metrics, "parsed metrics differ from official CSV")
            ensure(sample[phase + "_validation"] == {"validated_keys": 0 if workload == "ping" else KEYSPACE, "ping": True}, "missing data validation")
        ensure(sample["preload"] == sample["warmup_validation"], "missing preload validation")
        samples.append(dict(workload=workload, pipeline=pipeline, run=run, variant=variant, **sample["measured"]))
    rows = evaluate(samples)
    ensure(type(manifest["gate_passed"]) is bool and manifest["gate_passed"] == all(r["passed"] for r in rows), "gate outcome differs")
    ensure((root / "summary.md").read_text() == summary(rows), "gate summary differs")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("candidate", "valkey", "generator", "base-source", "head-source", "candidate-source", "valkey-version", "out"):
        parser.add_argument("--" + key, required=True)
    args = parser.parse_args()
    ensure(sys.platform == "linux", "Linux procfs/taskset required")
    ensure(all(re.fullmatch(SHA, getattr(args, k)) for k in ("base_source", "head_source", "candidate_source")), "invalid source SHA")
    ensure(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", args.valkey_version), "invalid Valkey pin")
    output = Path(args.out).resolve()
    ensure(not output.exists(), "output already exists; never reuse stale evidence")
    output.mkdir(parents=True)
    manifest = {"schema_version": 1, "complete": False, "gate_passed": False}
    save(output / "manifest.json", manifest)
    try:
        cpus = sorted(os.sched_getaffinity(0))
        server_cpus, client_cpus = placement(cpus)
        binaries = {"rillcache": str(Path(args.candidate).resolve(strict=True)),
                    "valkey": str(Path(args.valkey).resolve(strict=True)),
                    "generator": str(Path(args.generator).resolve(strict=True))}
        hashes = {name: digest(path) for name, path in binaries.items()}
        version = subprocess.check_output([binaries["valkey"], "--version"], timeout=10).decode().strip()
        ensure(re.search(r"\bv=" + re.escape(args.valkey_version) + r"(?:\s|$)", version), "Valkey version differs from protected pin")
        manifest.update(base_source=args.base_source, head_source=args.head_source, candidate_source=args.candidate_source,
                        allowed_cpus=cpus, server_cpus=server_cpus, client_cpus=client_cpus, binaries=binaries,
                        binary_sha256=hashes, valkey_version=version, runs=RUNS, workloads=WORKLOADS,
                        pipelines=PIPELINES, launch_directory=str(output), order=[], evidence_sha256={})
        save(output / "manifest.json", manifest)
        samples = []
        for identity in identities():
            workload, pipeline, run, variant = identity
            ensure(all(digest(path) == hashes[name] for name, path in binaries.items()), "binary changed during gate")
            name = name_for(*identity)
            directory = output / name
            sample(binaries, variant, workload, pipeline, server_cpus, client_cpus, directory)
            parsed = read_json(directory / "sample.json")["measured"]
            samples.append(dict(workload=workload, pipeline=pipeline, run=run, variant=variant, **parsed))
            manifest["order"].append(name)
            manifest["evidence_sha256"].update({f"{name}/{f}": digest(directory / f) for f in RAW_FILES})
            save(output / "manifest.json", manifest)
            print(name, "validated", flush=True)
        ensure(all(digest(path) == hashes[name] for name, path in binaries.items()), "binary changed after gate")
        rows = evaluate(samples)
        (output / "summary.md").write_text(summary(rows))
        manifest.update(complete=True, gate_passed=all(r["passed"] for r in rows))
        save(output / "manifest.json", manifest)
        load_cohort(output)
        return 0 if manifest["gate_passed"] else 1
    except BaseException as error:
        manifest.update(complete=False, gate_passed=False, error=str(error))
        save(output / "manifest.json", manifest)
        raise


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--check":
        rows = load_cohort(sys.argv[2])
        print(summary(rows))
        raise SystemExit(0 if all(r["passed"] for r in rows) else 1)
    raise SystemExit(main())

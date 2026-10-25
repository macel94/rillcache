#!/usr/bin/env python3
"""Optional Linux A/B orchestration. Python stdlib only; no Cargo dependency.

The frozen Rust client validates every reply. No profiler attaches during timing.
"""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import selectors
import statistics
import subprocess
import sys
import unittest

WORKLOADS = ("ping", "set", "get", "set-nx-existing", "set-xx-existing")


def ensure(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def strict_json(text):
    def pairs(items):
        result = {}
        for key, value in items:
            ensure(key not in result, "duplicate JSON key")
            result[key] = value
        return result
    def constant(_):
        raise ValueError("nonfinite JSON")
    ensure(len(text) <= 2 * 1024 * 1024, "oversized evidence")
    def finite_float(text):
        value = float(text)
        ensure(math.isfinite(value), "nonfinite JSON number")
        return value
    return json.loads(text, object_pairs_hook=pairs, parse_constant=constant, parse_float=finite_float)


def number(value):
    ensure(type(value) in (int, float) and math.isfinite(value) and value >= 0, "invalid nonnegative number")
    return value


def validate(data, requests, pipeline, pid):
    ensure(type(data["schema_version"]) is int and data["schema_version"] == 1 and data["mode"] == "external", "wrong evidence kind")
    ensure(data["server_configuration"] is None, "external mode must be launcher-owned")
    records = data["results"]
    ensure(len(records) == 5 and {r["workload"] for r in records} == set(WORKLOADS), "missing/duplicate workload")
    batches = sum(((requests // 8 + int(c < requests % 8)) + pipeline - 1) // pipeline for c in range(8))
    for row in records:
        expected = {"requests": requests, "clients": 8, "pipeline": pipeline, "keyspace": 1024, "value_size": 64, "warmup_per_client": 1000, "run": 1, "batches": batches}
        ensure(all(type(row[k]) is int and row[k] == v for k, v in expected.items()), "parameter/batch mismatch")
        elapsed = number(row["elapsed_seconds"])
        rate = number(row["ops_per_second"])
        ensure(elapsed > 0 and rate > 0 and math.isclose(rate, requests / elapsed, rel_tol=1e-6, abs_tol=.02), "timer/rate mismatch")
        latencies = [number(row[k]) for k in ("batch_p50_us", "batch_p95_us", "batch_p99_us", "batch_max_us")]
        ensure(latencies == sorted(latencies), "unordered percentiles")
        profile = row["server_profile"]
        ensure(profile["source"] == "linux-procfs" and type(profile["pid"]) is int and profile["pid"] == pid and type(profile["process_start_ticks"]) is int and profile["process_start_ticks"] > 0, "wrong process identity")
        ensure(profile["context_switches_scope"] == "matched_thread_lifetimes", "wrong accounting scope")
        hz = profile["clock_ticks_per_second"]
        ensure(all(type(profile[k]) is int and profile[k] >= 0 for k in ("user_ticks", "system_ticks")), "invalid CPU tick counters")
        ticks = profile["user_ticks"] + profile["system_ticks"]
        ensure(type(hz) is int and hz > 0 and type(ticks) is int and ticks >= 0, "invalid CPU counters")
        duration = number(profile["observation_seconds"])
        cpu = number(profile["cpu_percent_one_logical_cpu"])
        ensure(duration > 0 and math.isclose(cpu, ticks / hz / duration * 100, rel_tol=1e-4, abs_tol=.002), "CPU accounting mismatch")
    return records


def measure(binary, client, mode, budget, requests, pipeline, cpus, directory):
    command = [str(binary), "--bind", "127.0.0.1:0"]
    if mode:
        command += ["--threads", str(mode)]
    if budget == "one-cpu":
        command = ["taskset", "-c", str(cpus[0]), *command]
    server = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        with selectors.DefaultSelector() as ready:
            ready.register(server.stderr, selectors.EVENT_READ)
            ensure(ready.select(10), "server readiness deadline")
            line = server.stderr.readline(512).decode("utf-8", "strict")
        ensure(server.poll() is None and " listening on " in line, "server failed before readiness")
        expected_cpus = {cpus[0]} if budget == "one-cpu" else set(cpus)
        ensure(os.sched_getaffinity(server.pid) == expected_cpus, "actual server affinity differs")
        address = line.split()[-1]
        ensure(re.fullmatch(r"127\.0\.0\.1:[1-9][0-9]*", address), "invalid listening address")
        command = [str(client), "--addr", address, "--server-pid", str(server.pid), "--profile-idle-ms", "1000", "--requests", str(requests), "--clients", "8", "--pipeline", str(pipeline), "--keyspace", "1024", "--value-size", "64", "--warmup", "1000", "--runs", "1"]
        if budget == "one-cpu":
            command = ["taskset", "-c", ",".join(map(str, cpus[1:])), *command]
        result = subprocess.run(command, capture_output=True, check=True, timeout=180)
        ensure(server.poll() is None, "server exited during measurement")
        data = strict_json(result.stdout.decode())
        records = validate(data, requests, pipeline, server.pid)
        (directory.with_suffix(".json")).write_bytes(result.stdout)
        (directory.with_suffix(".log")).write_bytes(result.stderr)
        return records
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=5)
        server.stderr.close()


def summarize(samples, repetitions, before, after):
    ensure(len(samples) == 2 * 2 * 3 * 2 * repetitions * 5, "incomplete A/B cohort")
    text = ["# Matched before/after hot-path study", "", f"Baseline source: `{before}`. Candidate source: `{after}`.", "", "Same frozen client, eight clients, 1024 cyclic keys, 64-byte values and 1000 warmup commands/client. Build order alternates within each scenario. Separate native processes are reaped between runs. No timing gate or general Redis/Valkey victory is claimed.", "", "CPU µs/command comes from tick-quantized process accounting; windows bracket request timers. p99 is whole-batch completion, never per-request p99. Inherited budgets can consume several CPUs. The machine is shared; inspect spreads, CPU and tails, not just throughput. Idle accounting and matched-thread coverage remain in raw JSON.", "", "| Budget | Workers (0=per-client) | Pipeline | Workload | Before cmd/s [min–max] | After cmd/s [min–max] | Rate Δ | Before batch p99 µs | After batch p99 µs | Before CPU µs/cmd | After CPU µs/cmd |", "| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for budget in ("inherited", "one-cpu"):
        for mode in (0, 1, 4):
            for pipeline in (1, 16):
                for workload in WORKLOADS:
                    values = []
                    for variant in ("before", "after"):
                        rows = [r for b, m, p, v, r in samples if (b, m, p, v, r["workload"]) == (budget, mode, pipeline, variant, workload)]
                        ensure(len(rows) == repetitions, "missing A/B repetition")
                        rates = [r["ops_per_second"] for r in rows]
                        cpu = [(r["server_profile"]["user_ticks"] + r["server_profile"]["system_ticks"]) / r["server_profile"]["clock_ticks_per_second"] / r["requests"] * 1e6 for r in rows]
                        values.append((statistics.median(rates), min(rates), max(rates), statistics.median(r["batch_p99_us"] for r in rows), statistics.median(cpu)))
                    a, b = values
                    text.append(f"| {budget} | {mode} | {pipeline} | {workload} | {a[0]:.0f} [{a[1]:.0f}–{a[2]:.0f}] | {b[0]:.0f} [{b[1]:.0f}–{b[2]:.0f}] | {(b[0]/a[0]-1)*100:+.1f}% | {a[3]:.1f} | {b[3]:.1f} | {a[4]:.2f} | {b[4]:.2f} |")
    return "\n".join(text) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("before", "after", "client", "before-source", "after-source", "out"):
        parser.add_argument("--" + key, required=True)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--pipeline-1-requests", type=int, default=20000)
    parser.add_argument("--pipeline-16-requests", type=int, default=1000000)
    args = parser.parse_args()
    ensure(sys.platform == "linux", "Linux procfs/taskset required")
    ensure(1 <= args.runs <= 10, "runs must be 1..10")
    ensure(all(8 <= n <= 10000000 for n in (args.pipeline_1_requests, args.pipeline_16_requests)), "invalid request count")
    ensure(all(re.fullmatch("[0-9a-f]{40}", s) for s in (args.before_source, args.after_source)), "invalid source SHA")
    cpus = sorted(os.sched_getaffinity(0))
    ensure(len(cpus) >= 2, "separate server/client logical CPUs required")
    output = Path(args.out)
    ensure(not output.exists(), "output already exists; never mix stale evidence")
    output.mkdir(parents=True)
    binaries = {name: Path(getattr(args, name)).resolve(strict=True) for name in ("before", "after", "client")}
    hashes = {name: digest(path) for name, path in binaries.items()}
    metadata = {"schema_version": 1, "complete": False, "before_source": args.before_source, "after_source": args.after_source, "binary_sha256": hashes, "allowed_cpus": cpus, "host": platform.platform(), "date": datetime.datetime.now(datetime.timezone.utc).isoformat(), "runs": args.runs, "pipeline_1_requests": args.pipeline_1_requests, "pipeline_16_requests": args.pipeline_16_requests}
    (output / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n")
    samples = []
    order = []
    for budget in ("inherited", "one-cpu"):
        for mode in (0, 1, 4):
            for pipeline in (1, 16):
                requests = args.pipeline_1_requests if pipeline == 1 else args.pipeline_16_requests
                for run in range(1, args.runs + 1):
                    for variant in (("before", "after") if (run + mode + pipeline) % 2 else ("after", "before")):
                        ensure(all(digest(path) == hashes[name] for name, path in binaries.items()), "binary changed during study")
                        name = f"{budget}-threads-{mode}-pipeline-{pipeline}-run-{run}-{variant}"
                        order.append(name)
                        rows = measure(binaries[variant], binaries["client"], mode, budget, requests, pipeline, cpus, output / name)
                        samples.extend((budget, mode, pipeline, variant, r) for r in rows)
                        print(name, "validated", flush=True)
    ensure(all(digest(path) == hashes[name] for name, path in binaries.items()), "binary changed during final measurement")
    (output / "summary.md").write_text(summarize(samples, args.runs, args.before_source, args.after_source))
    metadata.update(complete=True, records=len(samples), measurement_order=order)
    metadata["evidence_sha256"] = {p.name: digest(p) for p in sorted(output.glob("*.json")) if p.name != "manifest.json"}
    (output / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n")


class ValidationTests(unittest.TestCase):
    def test_strict_json(self):
        for invalid in ('{"a":1,"a":2}', '{"a":NaN}', '{"a":Infinity}', '{"a":1e999}', '{}[]'):
            with self.assertRaises(ValueError):
                strict_json(invalid)
        self.assertEqual(strict_json('{"a":18446744073709551615}')["a"], 18446744073709551615)

    def test_nonfinite_and_bool_are_not_numbers(self):
        for invalid in (True, -1, float("inf"), float("nan"), "1"):
            with self.assertRaises(ValueError):
                number(invalid)

    def fixture(self, pipeline=16):
        profile = dict(source="linux-procfs", pid=47, process_start_ticks=123, context_switches_scope="matched_thread_lifetimes", clock_ticks_per_second=100, user_ticks=1, system_ticks=2, observation_seconds=1, cpu_percent_one_logical_cpu=3)
        records = [dict(workload=w, requests=80, clients=8, pipeline=pipeline, keyspace=1024, value_size=64, warmup_per_client=1000, run=1, batches=80 if pipeline == 1 else 8, elapsed_seconds=.1, ops_per_second=800, batch_p50_us=1, batch_p95_us=2, batch_p99_us=3, batch_max_us=4, server_profile=profile.copy()) for w in WORKLOADS]
        return dict(schema_version=1, mode="external", server_configuration=None, results=records)

    def test_valid_wire_record_parameters_and_partial_batches(self):
        for pipeline in (1, 16):
            self.assertEqual(len(validate(self.fixture(pipeline), 80, pipeline, 47)), 5)

    def test_bad_workload_parameters_timers_and_accounting_are_rejected(self):
        for key, value in (("requests", 81), ("requests", True), ("clients", 7), ("pipeline", 1), ("batches", 9), ("run", 2), ("ops_per_second", 0), ("elapsed_seconds", 0), ("ops_per_second", 10), ("batch_p99_us", 0), ("batch_p99_us", float("inf"))):
            data = self.fixture()
            data["results"][0][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                validate(data, 80, 16, 47)
        for key, value in (("pid", 48), ("process_start_ticks", True), ("clock_ticks_per_second", 0), ("user_ticks", -1), ("system_ticks", True), ("observation_seconds", 0), ("cpu_percent_one_logical_cpu", 0), ("context_switches_scope", "leader-only")):
            data = self.fixture()
            data["results"][0]["server_profile"][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                validate(data, 80, 16, 47)
        for bad in ([], self.fixture()["results"][:4], [self.fixture()["results"][0]] * 5):
            data = self.fixture()
            data["results"] = bad
            with self.assertRaises(ValueError):
                validate(data, 80, 16, 47)

    def test_complete_summary_has_all_scenarios_and_rejects_missing_repetitions(self):
        samples = [(budget, mode, pipeline, variant, row) for budget in ("inherited", "one-cpu") for mode in (0, 1, 4) for pipeline in (1, 16) for variant in ("before", "after") for row in self.fixture(pipeline)["results"]]
        text = summarize(samples, 1, "a" * 40, "b" * 40)
        self.assertEqual(sum(line.startswith("| one-cpu") or line.startswith("| inherited") for line in text.splitlines()), 60)
        self.assertEqual(text, summarize(samples, 1, "a" * 40, "b" * 40))
        samples[-1] = samples[0]
        with self.assertRaises(ValueError):
            summarize(samples, 1, "a" * 40, "b" * 40)

    def test_incomplete_summary_is_rejected(self):
        with self.assertRaises(ValueError):
            summarize([], 3, "a" * 40, "b" * 40)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        main()

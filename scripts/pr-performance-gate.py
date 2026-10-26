#!/usr/bin/env python3
"""Mandatory PR comparison against pinned Valkey. Python stdlib; Linux tooling only.

Use the protected base branch's copy of this harness/client, not PR replacements.
A measured cohort is not a universal performance or production-readiness claim.
"""
import argparse
import copy
import datetime
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import socket
import statistics
import subprocess
import sys
import time
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("matched_builds", Path(__file__).with_name("compare-builds.py"))
matched = importlib.util.module_from_spec(spec)
spec.loader.exec_module(matched)
ensure, digest, strict_json, validate = matched.ensure, matched.digest, matched.strict_json, matched.validate
WORKLOADS = matched.WORKLOADS
RUNS = 3
PROFILES = (("inherited", 1, 100000), ("inherited", 16, 1000000),
            ("one-cpu", 1, 100000), ("one-cpu", 16, 1000000))
SHA = r"[0-9a-f]{40}"


def evaluate(samples):
    ensure(len(samples) == len(PROFILES) * 2 * RUNS * len(WORKLOADS), "incomplete gate cohort")
    expected = {(budget, pipeline, variant, run, workload)
                for budget, pipeline, _ in PROFILES for variant in ("rillcache", "valkey")
                for run in range(1, RUNS + 1) for workload in WORKLOADS}
    actual = [(b, p, v, n, row["workload"]) for b, p, v, n, row in samples]
    ensure(len(set(actual)) == len(actual) and set(actual) == expected, "duplicate/missing gate sample")
    rows = []
    for budget, pipeline, _ in PROFILES:
        for workload in WORKLOADS:
            values = []
            for variant in ("rillcache", "valkey"):
                records = [row for b, p, v, _, row in samples
                           if (b, p, v, row["workload"]) == (budget, pipeline, variant, workload)]
                rates = [matched.number(row["ops_per_second"]) for row in records]
                ensure(all(rate > 0 for rate in rates), "nonpositive rate")
                tails = [matched.number(row["batch_p99_us"]) for row in records]
                cpus = [(row["server_profile"]["user_ticks"] + row["server_profile"]["system_ticks"])
                        / row["server_profile"]["clock_ticks_per_second"] / row["requests"] * 1e6 for row in records]
                values.append(dict(median=statistics.median(rates), minimum=min(rates), maximum=max(rates),
                                   batch_p99_us=statistics.median(tails), cpu_us_per_command=statistics.median(cpus)))
            candidate, valkey = values
            rows.append(dict(budget=budget, pipeline=pipeline, workload=workload,
                             rillcache=candidate, valkey=valkey,
                             passed=candidate["median"] > valkey["median"]))
    return rows


def summary(rows):
    passed = sum(row["passed"] for row in rows)
    text = ["# PR Valkey performance gate", "", f"**{'PASS' if passed == 20 else 'FAIL'}: {passed}/20 conditions strictly faster than Valkey.**", "",
            "Gate: Rillcache median throughput must strictly exceed Valkey in EVERY condition; ties fail. No aggregate score or docs-only exemption. Three alternating native repetitions, default server modes, eight clients, 1024 cyclic keys, 64-byte values, 1000 warmups/client; 100k requests at pipeline 1 and 1m at pipeline 16. The frozen Rust client validates every reply.", "",
            "p99 is whole-pipeline-batch completion, not per-request p99. CPU windows bracket request timers; tick quantization and matched-thread coverage remain in raw evidence. Inherited Rillcache can use multiple CPUs; equal-one-CPU servers use separate client CPUs. Shared-runner noise can block a PR; this is a scoped merge policy, not statistical certainty or a universal Valkey/production claim.", "",
            "| Budget | Pipeline | Workload | Rillcache cmd/s [min–max] | Valkey cmd/s [min–max] | Rate lead | Batch p99 µs R/V | CPU µs/cmd R/V | Gate |",
            "| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | --- |"]
    for row in rows:
        a, b = row["rillcache"], row["valkey"]
        text.append(f'| {row["budget"]} | {row["pipeline"]} | {row["workload"]} | {a["median"]:.0f} [{a["minimum"]:.0f}–{a["maximum"]:.0f}] | {b["median"]:.0f} [{b["minimum"]:.0f}–{b["maximum"]:.0f}] | {(a["median"]/b["median"]-1)*100:+.1f}% | {a["batch_p99_us"]:.1f}/{b["batch_p99_us"]:.1f} | {a["cpu_us_per_command"]:.2f}/{b["cpu_us_per_command"]:.2f} | {"PASS" if row["passed"] else "FAIL"} |')
    return "\n".join(text) + "\n"


def wait_ready(server, port):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        ensure(server.poll() is None, "server exited before readiness")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=.2) as connection:
                connection.settimeout(.2)
                connection.sendall(b"*1\r\n$4\r\nPING\r\n")
                reply = b""
                while len(reply) < 7:
                    part = connection.recv(7 - len(reply))
                    ensure(part, "EOF during readiness")
                    reply += part
                ensure(reply == b"+PONG\r\n", "invalid readiness response")
                return
        except (ConnectionRefusedError, TimeoutError):
            time.sleep(.01)
    raise ValueError("server readiness deadline")


def measure(binary, client, variant, budget, requests, pipeline, cpus, output):
    # Reserve an ephemeral address, then let the owned child bind it. Any bind
    # race/failure fails closed; never attach to or kill an unrelated process.
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    if variant == "rillcache":
        command = [str(binary), "--bind", f"127.0.0.1:{port}"]
    else:
        command = [str(binary), "--bind", "127.0.0.1", "--port", str(port), "--save", "", "--appendonly", "no",
                   "--daemonize", "no", "--loglevel", "warning", "--dir", str(output.parent)]
    if budget == "one-cpu":
        command = ["taskset", "-c", str(cpus[0]), *command]
    server = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        wait_ready(server, port)
        expected = {cpus[0]} if budget == "one-cpu" else set(cpus)
        ensure(os.sched_getaffinity(server.pid) == expected, "actual server affinity differs")
        command = [str(client), "--addr", f"127.0.0.1:{port}", "--label", variant, "--server-pid", str(server.pid),
                   "--profile-idle-ms", "1000", "--requests", str(requests), "--clients", "8", "--pipeline", str(pipeline),
                   "--keyspace", "1024", "--value-size", "64", "--warmup", "1000", "--runs", "1"]
        if budget == "one-cpu":
            command = ["taskset", "-c", ",".join(map(str, cpus[1:])), *command]
        result = subprocess.run(command, capture_output=True, check=True, timeout=180)
        ensure(server.poll() is None, "server exited during measurement")
        data = strict_json(result.stdout.decode())
        records = validate(data, requests, pipeline, server.pid)
        output.write_bytes(result.stdout)
        output.with_suffix(".log").write_bytes(result.stderr)
        return records, server.pid
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=5)


def save(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")


def load_cohort(directory):
    directory = Path(directory)
    manifest = strict_json((directory / "manifest.json").read_text())
    ensure(manifest["schema_version"] == 1 and manifest["complete"] is True, "incomplete gate evidence")
    ensure(type(manifest["runs"]) is int and manifest["runs"] == RUNS and manifest["profiles"] == [list(p) for p in PROFILES], "gate policy differs")
    hashes = manifest["binary_sha256"]
    ensure(set(hashes) == {"candidate", "valkey", "client"} and all(type(value) is str and re.fullmatch(r"[0-9a-f]{64}", value) for value in hashes.values()), "invalid binary identities")
    cpus = manifest["allowed_cpus"]
    ensure(len(cpus) >= 2 and all(type(cpu) is int and cpu >= 0 for cpu in cpus) and len(set(cpus)) == len(cpus), "invalid CPU allocation")
    ensure(all(re.fullmatch(SHA, manifest[key]) for key in ("base_source", "candidate_source", "head_source")), "invalid source identity")
    order = manifest["measurement_order"]
    ensure(len(order) == len(PROFILES) * 2 * RUNS, "missing measurement")
    expected_names = {f"{b}-pipeline-{p}-run-{r}-{v}.json" for b, p, _ in PROFILES
                      for r in range(1, RUNS + 1) for v in ("rillcache", "valkey")}
    ensure({entry["file"] for entry in order} == expected_names, "duplicate or unexpected measurement")
    ensure(set(manifest["evidence_sha256"]) == expected_names, "missing evidence hash")
    samples = []
    for entry in order:
        name = entry["file"]
        ensure(digest(directory / name) == manifest["evidence_sha256"][name], "evidence checksum differs")
        budget, pipeline, run, variant = entry["budget"], entry["pipeline"], entry["run"], entry["variant"]
        ensure(name == f"{budget}-pipeline-{pipeline}-run-{run}-{variant}.json", "measurement identity differs")
        requests = next(n for b, p, n in PROFILES if (b, p) == (budget, pipeline))
        ensure(type(entry["pid"]) is int and entry["pid"] > 0, "invalid launcher PID")
        records = validate(strict_json((directory / name).read_text()), requests, pipeline, entry["pid"])
        samples.extend((budget, pipeline, variant, run, record) for record in records)
    rows = evaluate(samples)
    ensure(manifest["records"] == len(samples) == 120, "wrong record count")
    ensure(type(manifest["gate_passed"]) is bool and manifest["gate_passed"] == all(r["passed"] for r in rows), "gate outcome differs")
    ensure((directory / "summary.md").read_text() == summary(rows), "gate table differs")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("candidate", "valkey", "client", "base-source", "candidate-source", "head-source", "valkey-version", "out"):
        parser.add_argument("--" + key, required=True)
    args = parser.parse_args()
    ensure(sys.platform == "linux", "Linux procfs/taskset required")
    ensure(all(re.fullmatch(SHA, getattr(args, key)) for key in ("base_source", "candidate_source", "head_source")), "invalid source SHA")
    ensure(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", args.valkey_version), "invalid Valkey pin")
    cpus = sorted(os.sched_getaffinity(0))
    ensure(len(cpus) >= 2, "separate server/client logical CPUs required")
    output = Path(args.out)
    ensure(not output.exists(), "output exists; never reuse stale evidence")
    output.mkdir(parents=True)
    binaries = {name: Path(getattr(args, name)).resolve(strict=True) for name in ("candidate", "valkey", "client")}
    hashes = {name: digest(path) for name, path in binaries.items()}
    version = subprocess.check_output([str(binaries["valkey"]), "--version"], timeout=10).decode().strip()
    ensure(re.search(r"\bv=" + re.escape(args.valkey_version) + r"(?:\s|$)", version), "Valkey version differs from protected pin")
    manifest = dict(schema_version=1, complete=False, gate_passed=False, base_source=args.base_source,
                    candidate_source=args.candidate_source, head_source=args.head_source, binary_sha256=hashes,
                    valkey_version=version, allowed_cpus=cpus, host=platform.platform(),
                    date=datetime.datetime.now(datetime.timezone.utc).isoformat(), runs=RUNS,
                    profiles=PROFILES, records=0, measurement_order=[], evidence_sha256={})
    save(output / "manifest.json", manifest)
    samples = []
    try:
        for index, (budget, pipeline, requests) in enumerate(PROFILES):
            for run in range(1, RUNS + 1):
                variants = ("rillcache", "valkey") if (run + index) % 2 else ("valkey", "rillcache")
                for variant in variants:
                    ensure(all(digest(path) == hashes[name] for name, path in binaries.items()), "binary changed during gate")
                    name = f"{budget}-pipeline-{pipeline}-run-{run}-{variant}.json"
                    binary = binaries["candidate" if variant == "rillcache" else "valkey"]
                    records, pid = measure(binary, binaries["client"], variant, budget, requests, pipeline, cpus, output / name)
                    samples.extend((budget, pipeline, variant, run, row) for row in records)
                    manifest["measurement_order"].append(dict(file=name, budget=budget, pipeline=pipeline, run=run, variant=variant, pid=pid))
                    manifest["evidence_sha256"][name] = digest(output / name)
                    manifest["records"] = len(samples)
                    save(output / "manifest.json", manifest)
                    print(name, "validated", flush=True)
        ensure(all(digest(path) == hashes[name] for name, path in binaries.items()), "binary changed after gate")
        rows = evaluate(samples)
        (output / "summary.md").write_text(summary(rows))
        manifest.update(complete=True, gate_passed=all(row["passed"] for row in rows))
        save(output / "manifest.json", manifest)
        load_cohort(output)
    except Exception as error:
        manifest.update(complete=False, gate_passed=False, error=str(error))
        save(output / "manifest.json", manifest)
        raise


class GateTests(unittest.TestCase):
    def fixture(self):
        rows = matched.ValidationTests().fixture()["results"]
        samples = []
        for b, p, _ in PROFILES:
            for variant in ("rillcache", "valkey"):
                for run in range(1, RUNS + 1):
                    for row in copy.deepcopy(rows):
                        row["ops_per_second"] = 101 if variant == "rillcache" else 100
                        samples.append((b, p, variant, run, row))
        return samples

    def test_all_twenty_conditions_must_win(self):
        rows = evaluate(self.fixture())
        self.assertEqual(len(rows), 20)
        self.assertTrue(all(row["passed"] for row in rows))
        self.assertIn("PASS: 20/20", summary(rows).replace("**", ""))

    def test_one_tie_or_loss_cannot_hide_in_an_average(self):
        for rate in (100, 99):
            samples = self.fixture()
            for b, p, v, _, row in samples:
                if (b, p, v, row["workload"]) == ("one-cpu", 1, "rillcache", "ping"):
                    row["ops_per_second"] = rate
            rows = evaluate(samples)
            self.assertEqual(sum(row["passed"] for row in rows), 19)
            self.assertIn("FAIL: 19/20", summary(rows).replace("**", ""))

    def test_median_not_best_sample(self):
        samples = self.fixture()
        for b, p, v, n, row in samples:
            if (b, p, v, row["workload"]) == ("one-cpu", 1, "rillcache", "ping"):
                row["ops_per_second"] = 1000000 if n == 1 else 99
        self.assertFalse(all(row["passed"] for row in evaluate(samples)))

    def test_missing_duplicate_or_wrong_repetition_fails_closed(self):
        for change in (lambda s: s.pop(), lambda s: s.__setitem__(-1, s[0]),
                       lambda s: s.__setitem__(0, (*s[0][:3], 4, s[0][4]))):
            samples = self.fixture(); change(samples)
            with self.assertRaises(ValueError): evaluate(samples)

    def test_invalid_rates_fail_closed(self):
        for value in (True, -1, 0, float("nan"), float("inf")):
            samples = self.fixture(); samples[0][4]["ops_per_second"] = value
            with self.assertRaises(ValueError): evaluate(samples)

    def cohort_fixture(self, directory):
        samples, order, hashes = [], [], {}
        for budget, pipeline, requests in PROFILES:
            for variant in ("rillcache", "valkey"):
                for run in range(1, RUNS + 1):
                    data = matched.ValidationTests().fixture(pipeline)
                    for row in data["results"]:
                        rate = 1100000 if variant == "rillcache" else 1000000
                        duration = requests / rate + .1
                        row.update(requests=requests, batches=8 * ((requests // 8 + pipeline - 1) // pipeline),
                                   elapsed_seconds=requests / rate, ops_per_second=rate)
                        row["server_profile"].update(observation_seconds=duration, cpu_percent_one_logical_cpu=3 / duration)
                    name = f"{budget}-pipeline-{pipeline}-run-{run}-{variant}.json"
                    save(directory / name, data)
                    hashes[name] = digest(directory / name)
                    order.append(dict(file=name, budget=budget, pipeline=pipeline, run=run, variant=variant, pid=47))
                    samples.extend((budget, pipeline, variant, run, row) for row in data["results"])
        manifest = dict(schema_version=1, complete=True, gate_passed=True, runs=RUNS, profiles=PROFILES,
                        base_source="a"*40, candidate_source="b"*40, head_source="c"*40, records=120,
                        binary_sha256={name: "d"*64 for name in ("candidate", "valkey", "client")},
                        allowed_cpus=[0, 1], measurement_order=order, evidence_sha256=hashes)
        save(directory / "manifest.json", manifest)
        (directory / "summary.md").write_text(summary(evaluate(samples)))
        return manifest

    def test_saved_cohort_revalidates_and_corruption_never_passes(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest = self.cohort_fixture(root)
            self.assertTrue(all(row["passed"] for row in load_cohort(root)))
            for key, value in (("complete", False), ("gate_passed", False), ("records", 119),
                               ("base_source", "bad"), ("allowed_cpus", [True, 1]), ("runs", True)):
                bad = copy.deepcopy(manifest); bad[key] = value; save(root / "manifest.json", bad)
                with self.subTest(key=key), self.assertRaises(ValueError): load_cohort(root)
            save(root / "manifest.json", manifest)
            first = root / manifest["measurement_order"][0]["file"]
            first.write_text(first.read_text() + " ")
            with self.assertRaisesRegex(ValueError, "checksum"): load_cohort(root)
            manifest["evidence_sha256"][first.name] = digest(first)
            save(root / "manifest.json", manifest)
            (root / "summary.md").write_text("fabricated pass")
            with self.assertRaisesRegex(ValueError, "table differs"): load_cohort(root)

    def test_readiness_failure_and_client_timeout_reap_owned_server(self):
        for readiness_failure in (True, False):
            server = mock.Mock(); server.poll.return_value = None
            with mock.patch.object(subprocess, "Popen", return_value=server), \
                 mock.patch(__name__ + ".wait_ready", side_effect=ValueError("deadline") if readiness_failure else None), \
                 mock.patch.object(os, "sched_getaffinity", return_value={0, 1}), \
                 mock.patch.object(subprocess, "run", side_effect=subprocess.TimeoutExpired("client", 180)):
                with self.assertRaises((ValueError, subprocess.TimeoutExpired)):
                    measure(Path("candidate"), Path("client"), "rillcache", "inherited", 80, 16, [0, 1], Path("unused.json"))
            server.terminate.assert_called_once()
            server.wait.assert_called_once_with(timeout=5)

    def test_permanent_reference_cohort_reproduces_twenty_scoped_wins(self):
        root = Path(__file__).resolve().parents[1] / "benchmarks/studies/client-handoffs/reference-0157177"
        samples = []
        for budget, pipeline, _ in PROFILES:
            profile = f'{"single-core-" if budget == "one-cpu" else ""}pipeline-{pipeline}'
            for variant in ("rillcache", "valkey"):
                data = strict_json((root / profile / (variant + ".json")).read_text())
                samples.extend((budget, pipeline, variant, row["run"], row) for row in data["results"])
        self.assertTrue(all(row["passed"] for row in evaluate(samples)))


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    elif len(sys.argv) == 3 and sys.argv[1] == "--check":
        rows = load_cohort(sys.argv[2])
        print(summary(rows))
        sys.exit(0 if all(row["passed"] for row in rows) else 1)
    else:
        main()

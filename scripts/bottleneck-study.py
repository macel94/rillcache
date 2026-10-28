#!/usr/bin/env python3
"""Reproduce/check Store scaling diagnostics; never TCP or merge-gate evidence."""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import sys
import tempfile
import unittest

WORKLOADS = ("ping", "hget", "hset", "hlen", "scard", "info-server", "memory-stats")
SIZES = (128, 1024, 8192, 32768)
REPETITIONS = 3
SCOPE = "in-process Store; not TCP/gate"


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
    def invalid(_):
        raise ValueError("nonfinite JSON")
    def number(text):
        value = float(text)
        ensure(math.isfinite(value), "nonfinite JSON")
        return value
    return json.loads(text, object_pairs_hook=pairs, parse_constant=invalid, parse_float=number)


def count(workload):
    return 10000 if workload in ("ping", "hget") else 500


def validate(row, workload, size):
    ensure(row["scope"] == SCOPE and row["workload"] == workload, "wrong workload/scope")
    for key, expected in (("size", size), ("operations", count(workload))):
        ensure(type(row[key]) is int and row[key] == expected, "parameter mismatch")
    ensure(row["replies_validated"] is True, "unvalidated replies")
    for key in ("elapsed_ns", "handler_p50_ns", "handler_p99_ns", "handler_max_ns"):
        ensure(type(row[key]) is int and row[key] > 0, "invalid duration")
    ensure(row["handler_p50_ns"] <= row["handler_p99_ns"] <= row["handler_max_ns"], "unordered percentiles")
    ensure(row["elapsed_ns"] >= row["handler_max_ns"], "handler exceeds whole-loop timer")
    return row


def summarize(samples):
    expected = {(rep, workload, size) for rep in range(1, REPETITIONS + 1)
                for workload in WORKLOADS for size in SIZES}
    actual = [(r["repetition"], r["workload"], r["size"]) for r in samples]
    ensure(len(actual) == len(expected) and set(actual) == expected, "incomplete/duplicate matrix")
    for row in samples:
        ensure(type(row["repetition"]) is int and 1 <= row["repetition"] <= REPETITIONS, "invalid repetition")
        validate(row, row["workload"], row["size"])
    lines = ["# Store component scaling", "", "No sockets, session observers, or reference servers. Not TCP/Valkey acceptance.",
             "Handler percentiles include two Instant reads/black-box/assertion overhead; whole-loop elapsed also includes reply validation. Seeding and 16 warmups are excluded. Shared-host observations, not SLOs.", "",
             "| Operation | Elements/keys | Handler p50 median ns [min–max] | Median handler p99 ns | Whole loop ns/call |",
             "|---|---:|---:|---:|---:|"]
    for workload in WORKLOADS:
        for size in SIZES:
            rows = [r for r in samples if (r["workload"], r["size"]) == (workload, size)]
            p50 = [r["handler_p50_ns"] for r in rows]
            p99 = statistics.median(r["handler_p99_ns"] for r in rows)
            loop = statistics.median(r["elapsed_ns"] / r["operations"] for r in rows)
            lines.append(f"| {workload} | {size} | {statistics.median(p50):.0f} [{min(p50)}–{max(p50)}] | {p99:.0f} | {loop:.0f} |")
    return "\n".join(lines) + "\n"


def check(root):
    manifest = strict_json((root / "manifest.json").read_text())
    ensure(manifest["complete"] is True, "incomplete study")
    ensure(re.fullmatch(r"[0-9a-f]{40}", manifest["application_source"]) is not None, "invalid source identity")
    ensure(manifest["scope"] == SCOPE and manifest["schema_version"] == 1, "wrong scope/schema")
    ensure(set(manifest["binary_sha256"]) == {"probe", "library"}, "missing binary identity")
    ensure(all(re.fullmatch(r"[0-9a-f]{64}", v) for v in manifest["binary_sha256"].values()), "invalid binary hash")
    files = manifest["evidence_sha256"]
    ensure(set(files) == {"samples.json", "summary.md", "environment.json"}, "missing evidence files")
    for name, expected in files.items():
        ensure(digest(root / name) == expected, "changed evidence: " + name)
    samples = strict_json((root / "samples.json").read_text())
    ensure(summarize(samples) == (root / "summary.md").read_text(), "summary mismatch")
    ensure(manifest["records"] == len(samples) == 84, "record count mismatch")
    ensure(manifest["measurement_order"] == [f'r{r["repetition"]}-{r["workload"]}-{r["size"]}' for r in samples], "measurement order mismatch")
    print("PASS: all 84 validated component samples, hashes and 28 summary rows")


def collect(args):
    root = args.out.resolve()
    ensure(not root.exists(), "output already exists; retain earlier evidence")
    ensure(re.fullmatch(r"[0-9a-f]{40}", args.source) is not None, "need exact source SHA")
    repo = Path(__file__).resolve().parents[1]
    # This diagnostic may change, but application/build inputs must match source.
    subprocess.run(["git", "diff", "--exit-code", args.source, "--", "src", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo"], cwd=repo, check=True, capture_output=True)
    cpus = sorted(os.sched_getaffinity(0))
    cpu = cpus[0] if args.cpu is None else args.cpu
    ensure(cpu in cpus, "requested CPU not allowed")
    binaries = {"probe": args.probe.resolve(strict=True), "library": args.library.resolve(strict=True)}
    hashes = {name: digest(path) for name, path in binaries.items()}
    root.mkdir(parents=True)
    manifest = dict(schema_version=1, scope=SCOPE, complete=False, application_source=args.source,
                    cpu=cpu, binary_sha256=hashes, measurement_order=[], records=0,
                    diagnostic_sha256={"probe.rs": digest(repo / "benchmarks/studies/current-bottlenecks/probe.rs"),
                                       "bottleneck-study.py": digest(__file__)})
    environment = dict(platform=platform.platform(), architecture=platform.machine(), allowed_cpus=cpus,
                       date=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                       rustc=subprocess.check_output(["rustc", "-vV"], text=True),
                       cpuinfo=Path("/proc/cpuinfo").read_text().split("\n\n", 1)[0],
                       sampling="No profiler attached to these elapsed-time samples")
    (root / "environment.json").write_text(json.dumps(environment, indent=2) + "\n")
    samples = []
    try:
        cases = [(workload, size) for workload in WORKLOADS for size in SIZES]
        for rep in range(1, REPETITIONS + 1):
            order = cases if rep == 1 else list(reversed(cases)) if rep == 2 else cases[14:] + cases[:14]
            for workload, size in order:
                ensure(all(digest(path) == hashes[name] for name, path in binaries.items()), "changed binary")
                stem = f"r{rep}-{workload}-{size}"
                command = ["taskset", "-c", str(cpu), str(binaries["probe"]), workload, str(size), str(count(workload))]
                result = subprocess.run(command, capture_output=True, timeout=120)
                (root / (stem + ".stdout")).write_bytes(result.stdout)
                (root / (stem + ".stderr")).write_bytes(result.stderr)
                result.check_returncode()
                row = validate(strict_json(result.stdout.decode()), workload, size)
                row["repetition"] = rep
                samples.append(row)
                manifest["measurement_order"].append(stem)
                (root / "samples.json").write_text(json.dumps(samples, indent=2) + "\n")
                print(stem, "validated", flush=True)
        ensure(all(digest(path) == hashes[name] for name, path in binaries.items()), "changed final binary")
        (root / "summary.md").write_text(summarize(samples))
        manifest.update(complete=True, records=len(samples), evidence_sha256={
            name: digest(root / name) for name in ("samples.json", "summary.md", "environment.json")})
    finally:
        manifest["records"] = len(samples)
        (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    check(root)


class Tests(unittest.TestCase):
    def rows(self):
        return [dict(scope=SCOPE, workload=w, size=s, operations=count(w), elapsed_ns=100000000,
                     handler_p50_ns=100, handler_p99_ns=200, handler_max_ns=300,
                     replies_validated=True, repetition=r)
                for r in range(1, 4) for w in WORKLOADS for s in SIZES]

    def test_complete_deterministic_table(self):
        rows = self.rows()
        text = summarize(rows)
        self.assertEqual(text, summarize(list(reversed(rows))))
        self.assertEqual(sum(line.startswith("| ") for line in text.splitlines()), 29)

    def test_missing_duplicate_and_mismatched_samples_fail(self):
        for change in (lambda r: r.pop(), lambda r: r.__setitem__(0, r[1]),
                       lambda r: r[0].update(operations=True), lambda r: r[0].update(replies_validated=False),
                       lambda r: r[0].update(handler_p99_ns=0), lambda r: r[0].update(elapsed_ns=1),
                       lambda r: r[0].update(repetition=True)):
            rows = self.rows(); change(rows)
            with self.assertRaises(ValueError):
                summarize(rows)

    def test_nonfinite_and_duplicate_json_fail(self):
        for text in ('{"x":1,"x":2}', '{"x":NaN}', '{"x":Infinity}', '{"x":1e999}'):
            with self.assertRaises(ValueError):
                strict_json(text)

    def test_tampered_files_and_order_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = self.rows()
            for name, text in (("samples.json", json.dumps(rows)), ("summary.md", summarize(rows)),
                               ("environment.json", "{}")):
                (root / name).write_text(text)
            manifest = dict(complete=True, schema_version=1, scope=SCOPE, application_source="0" * 40,
                            binary_sha256=dict(probe="1" * 64, library="2" * 64), records=84,
                            measurement_order=[f'r{r["repetition"]}-{r["workload"]}-{r["size"]}' for r in rows],
                            evidence_sha256={n: digest(root / n) for n in ("samples.json", "summary.md", "environment.json")})
            (root / "manifest.json").write_text(json.dumps(manifest))
            check(root)
            manifest["measurement_order"].reverse()
            (root / "manifest.json").write_text(json.dumps(manifest))
            with self.assertRaises(ValueError):
                check(root)
            manifest["measurement_order"].reverse()
            (root / "manifest.json").write_text(json.dumps(manifest))
            (root / "samples.json").write_text("[]")
            with self.assertRaises(ValueError):
                check(root)

    def test_incomplete_manifest_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "manifest.json").write_text('{"complete":false}')
            with self.assertRaises(ValueError):
                check(root)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--check", type=Path)
        parser.add_argument("--probe", type=Path)
        parser.add_argument("--library", type=Path)
        parser.add_argument("--source")
        parser.add_argument("--cpu", type=int)
        parser.add_argument("--out", type=Path)
        args = parser.parse_args()
        if args.check:
            check(args.check)
        else:
            if not all((args.probe, args.library, args.source, args.out)):
                parser.error("collection requires --probe --library --source --out")
            collect(args)

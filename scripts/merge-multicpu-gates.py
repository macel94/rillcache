#!/usr/bin/env python3
"""Merge validated per-workload partial cohorts into one required gate result.

A parallel PR workflow runs one workload per job (Valkey and Dragonfly cohorts
separately) so wall-clock feedback shrinks from ~31 minutes to roughly the build
plus one row. This script is the single decision point: it re-loads every partial
cohort through its own harness (which revalidates raw CSV, argv, affinity,
lifecycle and hashes), refuses incomplete or inconsistent inputs, and only then
emits the combined ten-row table and exit status.

Failure is closed on purpose:
* every workload must appear exactly once across the partials;
* sources, reference version, repetitions, pipelines, duration and binary hashes
  must agree across partials;
* any losing row, missing partial, duplicate row or hash/summary mismatch fails.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
SHA = r"[0-9a-f]{40}"


def ensure(condition, message):
    if not condition:
        raise ValueError(message)


def load_harness(path):
    spec = importlib.util.spec_from_file_location("partial_gate", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_json(path):
    with open(path) as stream:
        return json.load(stream)


def save(path, value):
    with open(path, "w") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def identity_fields(first):
    """Fields that must match across the partial cohorts of one server."""
    reference = "dragonfly_version" if "dragonfly_version" in first else "valkey_version"
    return ("base_source", "head_source", "candidate_source", "runs", "pipelines",
            "duration_seconds", "warmup_requests"), reference


def gather(harness, partials):
    manifests = []
    rows = []
    seen = []
    for directory in partials:
        root = Path(directory).resolve()
        manifest = read_json(root / "manifest.json")
        partial_rows = harness.load_cohort(root)
        workloads = tuple(manifest["workloads"])
        seen.extend(workloads)
        manifests.append({
            "workloads": list(workloads),
            "artifact_directory": str(root),
            "hardware_fingerprint": manifest.get("hardware_fingerprint"),
            "reference_version": manifest.get("valkey_version") or manifest.get("dragonfly_version"),
            "binary_sha256": manifest["binary_sha256"],
            "evidence_sha256": manifest.get("evidence_sha256", {}),
            "gate_passed": all(r["passed"] for r in partial_rows),
        })
        rows.extend(partial_rows)

    ensure(manifests, "no partial cohorts were provided")
    expected = list(harness.WORKLOADS)
    ensure(sorted(seen) == sorted(expected), f"workload coverage differs: {sorted(seen)} != {sorted(expected)}")
    ensure(len(seen) == len(set(seen)), f"a workload was measured by more than one partial: {seen}")

    first = read_json(Path(partials[0]) / "manifest.json")
    fields, reference = identity_fields(first)
    for manifest in manifests:
        current = read_json(Path(manifest["artifact_directory"]) / "manifest.json")
        for field in fields:
            ensure(current[field] == first[field], f"partial {manifest['artifact_directory']} differs in {field}")
        ensure(re.fullmatch(SHA, current["candidate_source"]), "invalid candidate source")
        ensure(current["binary_sha256"] == first["binary_sha256"], "partial cohorts were built from different binaries")
        ensure(current.get(reference) == first.get(reference), f"partial cohorts use different {reference}")

    order = [(workload, pipeline) for workload in harness.WORKLOADS for pipeline in harness.PIPELINES]
    keyed = {(row["workload"], row["pipeline"]): row for row in rows}
    ensure(len(keyed) == len(rows), "duplicate workload/pipeline row across partials")
    ensure(set(keyed) == set(order), "merged cohort is missing a workload/pipeline row")
    merged = [keyed[item] for item in order]

    record = {
        "schema_version": 1,
        "complete": True,
        "gate_passed": all(row["passed"] for row in merged),
        "base_source": first["base_source"],
        "head_source": first["head_source"],
        "candidate_source": first["candidate_source"],
        "reference_version": first.get(reference),
        "runs": first["runs"],
        "pipelines": first["pipelines"],
        "duration_seconds": first["duration_seconds"],
        "warmup_requests": first["warmup_requests"],
        "binary_sha256": first["binary_sha256"],
        "partials": manifests,
        "rows": merged,
    }
    return record, merged


def report(harness, rows):
    text = harness.summary(rows)
    if not any("Parallel partial cohorts" in line for line in text.splitlines()):
        text += (
            "Each workload row above was measured by its own partial job on its own\n"
            "allocated four-CPU runner (same machine type, not one host), with both\n"
            "servers alternating inside that job. Per-row host fingerprints are in the\n"
            "merged manifest; the paired comparison and the strict median rule are\n"
            "unchanged, and a missing or inconsistent partial fails the gate.\n"
        )
    return text


def merge(harness, partials, output):
    output = Path(output).resolve()
    ensure(not output.exists(), "output already exists; never reuse stale evidence")
    output.mkdir(parents=True)
    record, rows = gather(harness, partials)
    save(output / "manifest.json", record)
    save(output / "results.json", rows)
    (output / "summary.md").write_text(report(harness, rows))
    return record, rows


def check(harness, partials, merged):
    record, rows = gather(harness, partials)
    root = Path(merged).resolve()
    saved = read_json(root / "manifest.json")
    ensure(saved == record, "merged manifest differs from the revalidated partials")
    ensure(read_json(root / "results.json") == rows, "merged results differ")
    ensure((root / "summary.md").read_text() == report(harness, rows), "merged summary differs")
    return record, rows


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--harness", required=True)
    parser.add_argument("--partial", action="append", required=True)
    parser.add_argument("--out")
    parser.add_argument("--check")
    args = parser.parse_args(argv)
    harness = load_harness(args.harness)
    if args.check:
        record, rows = check(harness, args.partial, args.check)
        print(report(harness, rows))
        return 0 if record["gate_passed"] else 1
    ensure(args.out, "--out is required to merge partial cohorts")
    ensure(os.environ.get("GITHUB_ACTIONS") == "true", "merging happens in the parallel PR workflow")
    record, rows = merge(harness, args.partial, args.out)
    print(report(harness, rows))
    return 0 if record["gate_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Matched, sequential native TCP comparison using one upstream load generator."""

import csv
import io
import itertools
import json
import math
import os
import statistics
import time

from common import (Client, EVIDENCE, SCOPE, WORK, command, digest, environment,
                    process_stats, save, server, source, summary)

SERVERS = ("rillcache", "redis", "valkey")
CONFIG = SCOPE["performance"]
VALUE = b"v" * CONFIG["value_bytes"]
DIRECTORY = EVIDENCE / "performance"


def key(index, prefix="data"):
    return f"{prefix}:{index:012d}"


def dataset(port, workload, *, load=False, total=None):
    """Validate every possible generated hit key, not a probabilistic sample."""
    with Client(port) as client:
        if client.call("PING") != b"PONG":
            raise ValueError("PING validation failed")
        if workload == "ping":
            return {"keys": 0, "validated": True}
        prefix = "counter" if workload == "incr" else "absent" if workload == "get-miss" else "data"
        expected = b"0" if workload == "incr" else VALUE
        if load and workload != "get-miss":
            for start in range(0, CONFIG["keyspace"], 128):
                replies = client.batch(("SET", key(i, prefix), expected) for i in range(start, start + 128))
                if any(reply != b"OK" for reply in replies):
                    raise ValueError("preload SET did not return OK")
        values = []
        for start in range(0, CONFIG["keyspace"], 128):
            values += client.batch(("GET", key(i, prefix)) for i in range(start, start + 128))
        if workload == "incr":
            if any(value is None for value in values) or sum(int(value) for value in values) != (total or 0):
                raise ValueError("INCR aggregate count mismatch")
            if any(int(value) < 0 for value in values):
                raise ValueError("negative counter")
        elif workload == "get-miss":
            if any(value is not None for value in values):
                raise ValueError("miss namespace unexpectedly populated")
        elif any(value != VALUE for value in values):
            raise ValueError("hit dataset value mismatch (must never benchmark empty GETs)")
        return {"keys": len(values), "validated": True, "counter_total": total,
                "expected_value_bytes": len(VALUE) if workload in ("get-hit", "set") else None}


def generator_argv(port, workload, pipeline, requests, client_cpu):
    cmd = {"ping": ["PING"], "get-hit": ["GET", "data:__rand_int__"],
           "get-miss": ["GET", "absent:__rand_int__"],
           "set": ["SET", "data:__rand_int__", VALUE.decode()],
           "incr": ["INCR", "counter:__rand_int__"]}[workload]
    return ["taskset", "-c", str(client_cpu), str(source("redis") / "src/redis-benchmark"),
            "-h", "127.0.0.1", "-p", str(port), "-c", str(CONFIG["clients"]), "-P", str(pipeline),
            "-n", str(requests), "-r", str(CONFIG["keyspace"]), "-k", "1", "--threads", "1",
            "--seed", str(CONFIG["seed"]), "--csv", *cmd]


def parse_csv(text, workload):
    rows = list(csv.DictReader(io.StringIO(text)))
    if len(rows) != 1 or not rows[0].get("test", "").split() or rows[0]["test"].split()[0].lower() != workload.split("-")[0]:
        raise ValueError("expected exactly one matching benchmark result")
    required = ("rps", "avg_latency_ms", "min_latency_ms", "p50_latency_ms", "p95_latency_ms", "p99_latency_ms", "max_latency_ms")
    try:
        values = {field: float(rows[0][field]) for field in required}
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError("missing or malformed benchmark metrics") from error
    if any(not math.isfinite(v) or v < 0 for v in values.values()) or values["rps"] <= 0:
        raise ValueError("invalid/zero throughput or latency")
    if not values["min_latency_ms"] <= values["p50_latency_ms"] <= values["p95_latency_ms"] <= values["p99_latency_ms"] <= values["max_latency_ms"]:
        raise ValueError("non-monotonic latency percentiles")
    return values


def generator(port, workload, pipeline, requests, directory, client_cpu):
    argv = generator_argv(port, workload, pipeline, requests, client_cpu)
    metrics = directory / "generator-resources.json"
    result = command(["/usr/bin/time", "-f", '{"user_s":%U,"system_s":%S,"peak_rss_kib":%M,"exit_code":%x}',
                      "-o", str(metrics), *argv], directory,
                     timeout=CONFIG["process_timeout_seconds"])
    if result["exit_code"] != 0 or result.get("timed_out"):
        raise ValueError(f"load generator failed; see {directory}")
    stderr = (directory / "stderr.log").read_text()
    # The pinned generator exits at the FIRST workload error, so a failed run
    # has an unknown total error count. Never report that as zero errors.
    warnings = [line for line in stderr.splitlines() if line.strip()]
    allowed = "WARNING: Could not fetch server CONFIG"
    if any(line != allowed for line in warnings):
        raise ValueError(f"unexpected generator stderr; see {directory}")
    result.update(parse_csv((directory / "stdout.log").read_text(), workload))
    result["resources"] = json.loads(metrics.read_text())
    result["completed_requests"] = requests  # Pinned upstream reports only after all requests finish; first error exits nonzero.
    result["workload_errors"] = 0
    result["metadata_probe_warnings"] = warnings
    save(directory / "parsed.json", result)
    return result


def sample(name, workload, pipeline, repetition, server_cpu, client_cpu):
    directory = DIRECTORY / f"{workload}-p{pipeline}" / f"r{repetition}-{name}"
    directory.mkdir(parents=True, exist_ok=True)
    result = {"server": name, "workload": workload, "pipeline": pipeline, "repetition": repetition,
              "status": "invalid", "workload_errors": None}
    try:
        with server(name, directory / "instance", affinity=server_cpu) as (proc, port):
            result["preload"] = dataset(port, workload, load=True)
            result["warmup"] = generator(port, workload, pipeline, CONFIG["warmup_requests"], directory / "warmup", client_cpu)
            result["before_validation"] = dataset(port, workload, total=CONFIG["warmup_requests"] if workload == "incr" else None)
            before = process_stats(proc.pid)
            start = time.monotonic()
            measured = generator(port, workload, pipeline, CONFIG["requests"], directory / "measured", client_cpu)
            elapsed = time.monotonic() - start
            after = process_stats(proc.pid)
            result["after_validation"] = dataset(port, workload, total=(CONFIG["warmup_requests"] + CONFIG["requests"]) if workload == "incr" else None)
            if proc.poll() is not None:
                raise ValueError("server exited during measurement")
            result.update(measured)
            result.update({"status": "valid", "server_cpu_seconds": after["cpu_seconds"] - before["cpu_seconds"],
                           "server_cpu_percent_one_core": 100 * (after["cpu_seconds"] - before["cpu_seconds"]) / elapsed,
                           "server_rss_kib": after["rss_kib"], "server_peak_rss_kib": after["peak_rss_kib"],
                           "server_threads": after["threads"], "resource_window_wall_seconds": elapsed})
    except Exception as error:
        result["error"] = str(error)
    finally:
        save(directory / "sample.json", result)
    return result


def aggregate(samples):
    tables = []
    for workload, pipeline in itertools.product(CONFIG["workloads"], CONFIG["pipelines"]):
        cohort = [s for s in samples if s["workload"] == workload and s["pipeline"] == pipeline]
        identities = [(s["server"], s["repetition"]) for s in cohort]
        expected = [(name, repetition) for name in SERVERS for repetition in range(CONFIG["repetitions"])]
        matched = sorted(identities) == sorted(expected) and all(s["status"] == "valid" for s in cohort)
        for name in SERVERS:
            records = [s for s in cohort if s["server"] == name and s["status"] == "valid"]
            row = {"workload": workload, "pipeline": pipeline, "server": name, "valid_samples": len(records),
                   "expected_samples": CONFIG["repetitions"], "matched_cohort_valid": matched}
            if records:
                rates = [s["rps"] for s in records]
                row.update({"rps_median": statistics.median(rates), "rps_min": min(rates), "rps_max": max(rates),
                            "rps_cv_percent": statistics.stdev(rates) / statistics.mean(rates) * 100 if len(rates) > 1 else 0,
                            "workload_errors": 0})
                for field in ("avg_latency_ms", "p50_latency_ms", "p95_latency_ms", "p99_latency_ms", "server_cpu_seconds",
                              "server_cpu_percent_one_core", "server_rss_kib", "server_peak_rss_kib"):
                    row[field + "_median"] = statistics.median(s[field] for s in records)
                row["generator_cpu_seconds_median"] = statistics.median(s["resources"]["user_s"] + s["resources"]["system_s"] for s in records)
                row["generator_peak_rss_kib_median"] = statistics.median(s["resources"]["peak_rss_kib"] for s in records)
            if not matched:
                row["workload_errors"] = None
            tables.append(row)
    return tables


def report(samples, metadata, error=None):
    rows = aggregate(samples)
    save(DIRECTORY / "results.json", {"metadata": metadata, "samples": samples, "aggregates": rows, "error": error})
    text = "## Upstream-generator TCP performance (advisory, not a merge gate)\n\n"
    valid = sum(s["status"] == "valid" for s in samples)
    expected = len(CONFIG["workloads"]) * len(CONFIG["pipelines"]) * len(SERVERS) * CONFIG["repetitions"]
    text += f"Valid samples: **{valid}/{expected}**; invalid/missing: **{expected - valid}**. Six repetitions; rotated server order; one runner/job.\n\n"
    if error:
        text += f"Infrastructure interruption: `{error}`\n\n"
    text += "| Workload | P | Server | n | Median req/s | CV % | Mean / p50 / p95 / p99 ms² | Server CPU % | RSS / peak KiB³ | Errors |\n|---|---:|---|---:|---:|---:|---|---:|---|---|\n"
    for row in rows:
        text += f"| {row['workload']} | {row['pipeline']} | {row['server']} | {row['valid_samples']} | "
        if "rps_median" not in row:
            text += "n/a | n/a | n/a | n/a | n/a | unknown |\n"
            continue
        latencies = " / ".join(f"{row[k + '_median']:.3f}" for k in ("avg_latency_ms", "p50_latency_ms", "p95_latency_ms", "p99_latency_ms"))
        errors = "0" if row["matched_cohort_valid"] else "unknown; INVALID cohort"
        text += f"{row['rps_median']:.0f} | {row['rps_cv_percent']:.2f} | {latencies} | {row['server_cpu_percent_one_core_median']:.1f} | {row['server_rss_kib_median']:.0f} / {row['server_peak_rss_kib_median']:.0f} | {errors} |\n"
    text += "\n### Matched throughput deltas (Rillcache vs reference)\n\n"
    for workload, pipeline in itertools.product(CONFIG["workloads"], CONFIG["pipelines"]):
        cohort = [row for row in rows if row["workload"] == workload and row["pipeline"] == pipeline]
        if not all(row["matched_cohort_valid"] for row in cohort):
            text += f"- {workload} P={pipeline}: **INVALID/INCOMPLETE**; no ratios or winner.\n"
            continue
        candidate = next(r for r in cohort if r["server"] == "rillcache")
        for reference in (r for r in cohort if r["server"] != "rillcache"):
            delta = (candidate["rps_median"] / reference["rps_median"] - 1) * 100
            threshold = max(CONFIG["advisory_difference_percent"], 2 * candidate["rps_cv_percent"], 2 * reference["rps_cv_percent"])
            label = "within advisory/noise band; inconclusive" if abs(delta) <= threshold else "descriptive difference only; NOT statistical significance"
            text += f"- {workload} P={pipeline} vs {reference['server']}: {delta:+.2f}% — {label}.\n"
    text += "\n² Medians of upstream per-run latency summaries, not pooled percentiles. At P>1 the generator measures pipeline latency; not a per-command latency claim. "
    text += "³ RSS after measured load; peak is the fresh server lifetime including preload/warmup, read from /proc. Server CPU delta excludes preload/warmup; CPU% uses outer generator wall time. "
    text += "Generator CPU seconds/peak RSS, throughput min/max, samples, commands, exit statuses and source/environment metadata are in JSON/log artifacts. "
    text += "A failed generator exits at first error: its total error count is unknown, never zero. A known CONFIG metadata-probe warning is retained separately, not counted as a workload error.\n\n"
    text += CONFIG["limits"] + " Shared-runner noise and client CPU can dominate; no universal speed/compatibility or significance claim. Existing all-20-case Valkey merge gate is unchanged.\n"
    summary(text, DIRECTORY / "summary.md")
    return valid == expected and error is None and all(row["matched_cohort_valid"] for row in rows)


def main():
    samples = []
    metadata = {}
    error = None
    try:
        if os.environ.get("GITHUB_ACTIONS") != "true":
            raise RuntimeError("Performance workloads run remotely in GitHub Actions only")
        if CONFIG["repetitions"] < 5:
            raise ValueError("at least five repetitions are mandatory")
        metadata = environment()
        metadata["build"] = json.loads((WORK / "build.json").read_text())
        metadata["generator_binary_sha256"] = digest(source("redis") / "src/redis-benchmark")
        cpus = sorted(os.sched_getaffinity(0))
        if len(cpus) < 2:
            raise RuntimeError("need two allowed CPUs for disjoint generator/server placement")
        metadata.update({"server_cpu": cpus[0], "client_cpu": cpus[1], "orders": []})
        save(DIRECTORY / "environment.json", metadata)
        for workload, pipeline in itertools.product(CONFIG["workloads"], CONFIG["pipelines"]):
            for repetition in range(CONFIG["repetitions"]):
                offset = repetition % len(SERVERS)
                order = SERVERS[offset:] + SERVERS[:offset]
                metadata["orders"].append({"workload": workload, "pipeline": pipeline, "repetition": repetition, "order": order})
                for name in order:
                    print(f"{workload} P={pipeline} repetition={repetition} server={name}", flush=True)
                    samples.append(sample(name, workload, pipeline, repetition, cpus[0], cpus[1]))
                    save(DIRECTORY / "progress.json", {"metadata": metadata, "samples": samples})
    except (Exception, KeyboardInterrupt) as exc:
        error = str(exc)
    return 0 if report(samples, metadata, error) else 2


if __name__ == "__main__":
    raise SystemExit(main())

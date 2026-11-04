#!/usr/bin/env python3
"""Read-only daily five-server comparison. No assumed universal winner or merge gate."""
import argparse
import contextlib
import itertools
import math
import os
from pathlib import Path
import re
import runpy
import statistics
import subprocess
import time

from common import (Client, EVIDENCE, PINS as NATIVE_PINS, ROOT, WORK, available_port, command, digest,
                    environment, process_stats, save, summary, terminate)
from daily_prepare import PINS, runtime_env, tree_hashes

# Reuse the unchanged official-generator parser, workload and dataset contracts;
# do not change either protected PR harness to add advisory server adapters.
G = runpy.run_path(str(ROOT / "scripts/pr-dragonfly-performance.py"))
SERVERS = ("rillcache", "redis", "valkey", "dragonfly", "garnet")
REFERENCES = SERVERS[1:]
RUNS = 5
SCHEMA = 1
RAW = G["RAW_FILES"]
ensure = G["ensure"]
read_json = G["read_json"]


def identities():
    for workload, pipeline in itertools.product(G["WORKLOADS"], G["PIPELINES"]):
        for repetition in range(RUNS):
            # Each server occupies every order position exactly once per row.
            order = SERVERS[repetition:] + SERVERS[:repetition]
            for variant in order:
                yield workload, pipeline, repetition + 1, variant


def sample_name(identity):
    workload, pipeline, repetition, variant = identity
    return f"{workload}-p{pipeline}-r{repetition}-{variant}"


def server_argv(binary, variant, port, cpus, directory):
    ensure(variant in SERVERS, "unknown server role")
    if variant == "rillcache":
        args = [binary, "--bind", f"127.0.0.1:{port}"]
    elif variant in ("redis", "valkey"):
        args = [binary, "--bind", "127.0.0.1", "--port", str(port), "--save", "", "--appendonly", "no",
                "--daemonize", "no", "--loglevel", "warning", "--dir", str(directory)]
    elif variant == "dragonfly":
        args = [binary, "--bind", "127.0.0.1", "--port", str(port), "--proactor_threads", str(len(cpus)),
                "--dbfilename=", "--logtostderr", "--conn_use_incoming_cpu"]
    else:
        # Garnet's default main log is 16 GiB. Size it explicitly for CI; this
        # is not an equal-RSS claim or a Redis maxmemory equivalent.
        args = [binary, "--bind", "127.0.0.1", "--port", str(port), "--memory", "64m", "--page", "1m",
                "--index", "4m", "--checkpoint-freq", "0", "--logger-level", "Warning"]
        # v2.2.5 defaults AOF, storage tiering, recovery and cluster to disabled.
    return ["taskset", "-c", G["cpu_list"](cpus), *map(str, args)]


@contextlib.contextmanager
def owned_server(binaries, variant, cpus, directory):
    directory.mkdir(parents=True)
    port = available_port()
    argv = server_argv(binaries[variant], variant, port, cpus, directory)
    identity = {"argv": argv, "port": port, "cpus": cpus}
    with (directory / "server.log").open("wb") as log:
        proc = subprocess.Popen(argv, cwd=directory, stdout=log, stderr=subprocess.STDOUT,
                                start_new_session=True, env=runtime_env())
        identity["pid"] = proc.pid
        try:
            deadline = time.monotonic() + 30
            while True:
                ensure(proc.poll() is None, f"{variant} exited before readiness")
                try:
                    with Client(port) as client:
                        ensure(client.call("PING") == b"PONG", "bad readiness reply")
                    break
                except (OSError, ConnectionError):
                    ensure(time.monotonic() < deadline, "server readiness deadline")
                    time.sleep(.02)
            identity["actual_cpus_before"] = sorted(os.sched_getaffinity(proc.pid))
            ensure(identity["actual_cpus_before"] == cpus, "server affinity differs")
            yield proc, port
            ensure(proc.poll() is None, "server exited during load")
            identity["actual_cpus_after"] = sorted(os.sched_getaffinity(proc.pid))
            ensure(identity["actual_cpus_after"] == cpus, "server affinity changed")
        finally:
            identity["exit_before_cleanup"] = proc.poll()
            identity["exit_after_cleanup"] = terminate(proc)
            identity["joined"] = True
            save(directory / "server.json", identity)


def sample(build, identity, cpus, client_cpus, directory):
    workload, pipeline, _, variant = identity
    result = {"valid": False}
    try:
        with owned_server(build["binaries"], variant, cpus, directory) as (proc, port):
            result.update(port=port, pid=proc.pid)
            result["preload"] = G["validate_dataset"](port, workload, preload=True)
            for phase, count in (("warmup", G["WARMUP"]), ("measured", None)):
                argv = G["generator_argv"](build["binaries"]["generator"], port, workload, pipeline, count, client_cpus)
                before = process_stats(proc.pid)
                started = time.monotonic()
                command(argv, directory / phase, timeout=180, env=runtime_env())
                result[phase] = G["read_generator"](directory / phase, workload, argv)
                elapsed = time.monotonic() - started
                after = process_stats(proc.pid)
                result[phase + "_validation"] = G["validate_dataset"](port, workload, after=True)
                if phase == "measured":
                    result.update(server_cpu_seconds=after["cpu_seconds"] - before["cpu_seconds"],
                                  resource_window_seconds=elapsed, server_rss_kib=after["rss_kib"],
                                  server_peak_rss_kib=after["peak_rss_kib"], server_threads=after["threads"])
        result["valid"] = True
    except BaseException as error:
        result["error"] = str(error)
        raise
    finally:
        save(directory / "sample.json", result)


def evaluate(samples):
    actual = [(s["workload"], s["pipeline"], s["run"], s["variant"]) for s in samples]
    ensure(actual == list(identities()), "incomplete, duplicate or reordered samples")
    rows = []
    for workload, pipeline in itertools.product(G["WORKLOADS"], G["PIPELINES"]):
        values = {}
        for name in SERVERS:
            records = [s for s in samples if (s["workload"], s["pipeline"], s["variant"]) == (workload, pipeline, name)]
            rates = [s["measured"]["rps"] for s in records]
            ensure(all(type(r) in (int, float) and math.isfinite(r) and r > 0 for r in rates), "invalid rate")
            values[name] = {"median": statistics.median(rates), "min": min(rates), "max": max(rates),
                            "cv_percent": statistics.stdev(rates) / statistics.mean(rates) * 100,
                            "p99_ms": statistics.median(s["measured"]["p99_latency_ms"] for s in records),
                            "cpu_seconds": statistics.median(s["server_cpu_seconds"] for s in records),
                            "rss_kib": statistics.median(s["server_rss_kib"] for s in records),
                            "peak_rss_kib": statistics.median(s["server_peak_rss_kib"] for s in records)}
        ranking = sorted(REFERENCES, key=lambda name: (-values[name]["median"], name))
        top = values[ranking[0]]
        decisive = all(top["median"] > values[name]["median"] and
                       (top["median"] / values[name]["median"] - 1) * 100 >
                       max(5, 2 * top["cv_percent"], 2 * values[name]["cv_percent"]) for name in ranking[1:])
        # Keep every pair, not just Rillcache's favorable comparisons.
        pairs = [{"left": a, "right": b, "left_rate_lead_percent":
                  (values[a]["median"] / values[b]["median"] - 1) * 100}
                 for a, b in itertools.combinations(SERVERS, 2)]
        rows.append({"workload": workload, "pipeline": pipeline, "servers": values,
                     "reference_ranking": ranking, "reference_leader_clear_of_noise_band": decisive,
                     "pairwise": pairs})
    return rows


def report(rows, error=None):
    text = ["# Daily five-server comparison (advisory)", ""]
    if error is not None:
        return "\n".join(text + ["**INCOMPLETE / INVALID: no rankings or winner.**", "", str(error), ""])
    clear = {row["reference_ranking"][0] for row in rows if row["reference_leader_clear_of_noise_band"]}
    champion = next(iter(clear)) if len(clear) == 1 and all(row["reference_leader_clear_of_noise_band"] for row in rows) else None
    text += ["**Complete: 250/250 validated samples.**", "",
             f"Reference leader across every measured row, beyond the advisory noise band: **{champion or 'none established'}**.", "",
             "Same runner, one fresh owned server at a time, four logical server CPUs; five balanced rotated repetitions. "
             "Same unchanged official valkey-benchmark, eight clients, two generator threads, 1024 keys, 64-byte values, "
             "seed 42, 8000 warmup commands, ten-second measured samples at P1/P16. Persistence/TLS off. "
             "Generator CPUs are disjoint when available, otherwise share the server budget; see manifest.", "",
             "| Workload | P | Reference order (fastest first) | Leader outside noise band? |", "| --- | ---: | --- | --- |"]
    for row in rows:
        text.append(f"| {row['workload']} | {row['pipeline']} | {' > '.join(row['reference_ranking'])} | {row['reference_leader_clear_of_noise_band']} |")
    text += ["", "| Workload | P | Server | Median [min–max] req/s | CV % | p99 ms | CPU seconds | RSS / peak KiB |",
             "| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |"]
    for row in rows:
        for name, v in row["servers"].items():
            text.append(f"| {row['workload']} | {row['pipeline']} | {name} | {v['median']:.2f} [{v['min']:.2f}–{v['max']:.2f}] | "
                        f"{v['cv_percent']:.2f} | {v['p99_ms']:.3f} | {v['cpu_seconds']:.2f} | {v['rss_kib']:.0f} / {v['peak_rss_kib']:.0f} |")
    text += ["", "All ten pairwise comparisons per condition are in results.json; ties/losses are retained. "
             "The advisory noise band is max(5%, twice either sample CV), not statistical significance. "
             "No aggregate score, universal fastest claim, automatic reference promotion or change to PR protection.", "",
             "p99 is upstream pipeline latency at P>1, not per-command latency; CPU/RSS windows include generator orchestration. "
             "Only non-error replies are checked by the upstream generator; full-keyspace semantic probes run before/after load. "
             "Garnet uses a pinned private .NET runtime with a 64 MiB log/1 MiB page/4 MiB index; this is not equal RSS "
             "or a maxmemory equivalence. Fixed warmup does not prove steady-state JIT/GC. Dragonfly is BSL-1.1, "
             "non-production benchmark use only. Binaries are not redistributed. Shared-runner noise/client contention, "
             "small string workloads and short runs do not establish production performance or compatibility.", ""]
    return "\n".join(text)


def finite_nonnegative(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def load_cohort(output):
    output = Path(output)
    m = read_json(output / "manifest.json")
    ensure(type(m["schema"]) is int and m["schema"] == SCHEMA and m["complete"] is True, "incomplete daily evidence")
    ensure(m["servers"] == list(SERVERS) and m["runs"] == RUNS and m["duration_seconds"] == G["DURATION_SECONDS"]
           and m["warmup_requests"] == G["WARMUP"], "daily policy differs")
    ensure(m["workloads"] == list(G["WORKLOADS"]) and m["pipelines"] == list(G["PIPELINES"]), "missing workload")
    ensure(re.fullmatch(r"[0-9a-f]{40}", m["build"]["source"]), "invalid source identity")
    ensure(m["build"]["tool_pins"] == PINS, "tool pin identity differs")
    ensure(m["build"]["comparison_generator"] == {"upstream": "valkey", "source_pin": NATIVE_PINS["upstreams"]["valkey"]},
           "official comparison generator identity differs")
    ensure(m["build"]["native_build"]["pins"] == NATIVE_PINS and
           m["build"]["native_build"]["built_git_head"] == m["build"]["source"] ==
           m["environment"]["git_head"]["stdout"].strip() == m["environment"]["GITHUB_SHA"], "native/source identity differs")
    hashes = m["build"]["binary_sha256"]
    ensure(set(hashes) == {*SERVERS, "generator"} and len(set(hashes.values())) == 6 and
           all(re.fullmatch(r"[0-9a-f]{64}", h) for h in hashes.values()), "binary role/hash mismatch")
    cpus, clients = G["placement"](m["allowed_cpus"])
    ensure(m["server_cpus"] == cpus and m["client_cpus"] == clients and
           m["environment"]["cpu_affinity"] == m["allowed_cpus"], "CPU allocation differs")
    ensure(m["order"] == [sample_name(i) for i in identities()], "sample order differs")
    expected = {f"{sample_name(i)}/{file}" for i in identities() for file in RAW}
    ensure(set(m["evidence_sha256"]) == expected, "missing/unexpected raw files")
    samples = []
    for identity in identities():
        workload, pipeline, repetition, variant = identity
        directory = output / sample_name(identity)
        for file in RAW:
            ensure(digest(directory / file) == m["evidence_sha256"][f"{directory.name}/{file}"], "raw checksum differs")
        s, server = read_json(directory / "sample.json"), read_json(directory / "server.json")
        ensure(s["valid"] is True and server["joined"] is True and server["exit_before_cleanup"] is None
               and type(server["exit_after_cleanup"]) is int, "invalid lifecycle")
        ensure(type(s["pid"]) is int and s["pid"] > 0 and s["pid"] == server["pid"], "wrong PID")
        port = s["port"]
        ensure(type(port) is int and 0 < port < 65536 and server["port"] == port, "invalid port")
        ensure(server["cpus"] == cpus == server["actual_cpus_before"] == server["actual_cpus_after"], "server affinity differs")
        ensure(server["argv"] == server_argv(m["build"]["binaries"][variant], variant, port, cpus,
                                            Path(m["launch_directory"]) / directory.name), "server argv differs")
        for phase, count in (("warmup", G["WARMUP"]), ("measured", None)):
            argv = G["generator_argv"](m["build"]["binaries"]["generator"], port, workload, pipeline, count, clients)
            ensure(s[phase] == G["read_generator"](directory / phase, workload, argv), "metrics differ from raw CSV")
            ensure(s[phase + "_validation"] == {"validated_keys": 0 if workload == "ping" else G["KEYSPACE"], "ping": True}, "missing semantic probes")
        ensure(s["preload"] == s["warmup_validation"], "invalid preload")
        ensure(all(finite_nonnegative(s[k]) for k in ("server_cpu_seconds", "resource_window_seconds", "server_rss_kib", "server_peak_rss_kib"))
               and s["resource_window_seconds"] >= G["DURATION_SECONDS"] and type(s["server_threads"]) is int
               and s["server_threads"] > 0, "invalid resources")
        samples.append(dict(workload=workload, pipeline=pipeline, run=repetition, variant=variant, **s))
    rows = evaluate(samples)
    ensure(read_json(output / "results.json") == rows, "derived comparison differs")
    ensure((output / "summary.md").read_text() == report(rows), "derived report differs")
    return rows


def run(output, smoke=False):
    ensure(os.environ.get("GITHUB_ACTIONS") == "true", "Native comparisons run only in Actions")
    ensure(not output.exists(), "never reuse stale output")
    output.mkdir(parents=True)
    manifest = {"schema": SCHEMA, "complete": False}
    save(output / "manifest.json", manifest)
    try:
        build = read_json(WORK / "daily-build.json")
        ensure(build["runtime_files_sha256"] == tree_hashes(), "tool/runtime files changed since build")
        allowed = sorted(os.sched_getaffinity(0))
        cpus, clients = G["placement"](allowed)
        metadata = environment()
        metadata.pop("scope", None)  # Legacy manual scope is not this daily protocol.
        manifest.update(build=build, environment=metadata, allowed_cpus=allowed, server_cpus=cpus, client_cpus=clients,
                        servers=SERVERS, runs=RUNS, workloads=G["WORKLOADS"], pipelines=G["PIPELINES"],
                        duration_seconds=G["DURATION_SECONDS"], warmup_requests=G["WARMUP"],
                        launch_directory=str(output.resolve()), order=[], evidence_sha256={})
        save(output / "manifest.json", manifest)
        if smoke:
            for name in SERVERS:
                with owned_server(build["binaries"], name, cpus, output / name) as (_, port):
                    for workload in G["WORKLOADS"]:
                        G["validate_dataset"](port, workload, preload=True)
                        G["validate_dataset"](port, workload, after=True)
            save(output / "smoke.json", {"valid": True, "servers": SERVERS, "not_performance_evidence": True})
            return
        samples = []
        for identity in identities():
            ensure(all(digest(path) == build["binary_sha256"][name] for name, path in build["binaries"].items()), "binary changed")
            directory = output / sample_name(identity)
            sample(build, identity, cpus, clients, directory)
            workload, pipeline, repetition, variant = identity
            s = read_json(directory / "sample.json")
            samples.append(dict(workload=workload, pipeline=pipeline, run=repetition, variant=variant, **s))
            manifest["order"].append(directory.name)
            manifest["evidence_sha256"].update({f"{directory.name}/{f}": digest(directory / f) for f in RAW})
            save(output / "manifest.json", manifest)
            print(directory.name, "validated", flush=True)
        ensure(build["runtime_files_sha256"] == tree_hashes() and all(
            digest(path) == build["binary_sha256"][name] for name, path in build["binaries"].items()), "tools changed after load")
        rows = evaluate(samples)
        save(output / "results.json", rows)
        (output / "summary.md").write_text(report(rows))
        manifest["complete"] = True
        save(output / "manifest.json", manifest)
        load_cohort(output)
        summary(report(rows), output / "summary.md")
    except BaseException as error:
        manifest.update(complete=False, error=str(error))
        save(output / "manifest.json", manifest)
        summary(report([], str(error)), output / "summary.md")
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", type=Path)
    parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args()
    if args.check:
        print(report(load_cohort(args.check)))
    else:
        run(EVIDENCE / ("daily-smoke" if args.smoke else "daily"), args.smoke)

#!/usr/bin/env python3
"""Same-job hosted reactor diagnostic; NOT fixed-host screening or a merge gate."""
import argparse
import contextlib
import importlib.util
import math
import os
from pathlib import Path
import re
import runpy
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("reactor_build_tools", ROOT / "scripts/dev-performance.py")
D = importlib.util.module_from_spec(spec)
spec.loader.exec_module(D)
G = runpy.run_path(str(ROOT / "scripts/pr-dragonfly-performance.py"))
C = D.COMMON
OUT = ROOT / "target/upstream/evidence/reactor-study"
VARIANTS = ("baseline", "candidate", "valkey")
MODES = ("default", "one-worker")
WORKLOADS = ("ping", "get")
PIPELINES = (1, 16)
SAFE_ENV = {"PATH": os.environ["PATH"], "LANG": "C", "LC_ALL": "C"}


def identities():
    for mode in MODES:
        for workload in WORKLOADS:
            for pipeline in PIPELINES:
                for run in range(1, 4):
                    offset = run - 1
                    for variant in VARIANTS[offset:] + VARIANTS[:offset]:
                        yield mode, workload, pipeline, run, variant


def name_for(identity):
    mode, workload, pipeline, run, variant = identity
    return f"{mode}-{workload}-p{pipeline}-r{run}-{variant}"


def snapshot(pid):
    group = D.snapshot(pid)
    tasks = {}
    for path in sorted(Path(f"/proc/{pid}/task").iterdir()):
        try:
            stat = (path / "stat").read_text().rsplit(") ", 1)[1].split()
            status = dict(line.split(":", 1) for line in (path / "status").read_text().splitlines())
            tasks[path.name] = {"start_ticks": int(stat[19]),
                                "voluntary": int(status["voluntary_ctxt_switches"]),
                                "involuntary": int(status["nonvoluntary_ctxt_switches"])}
        except FileNotFoundError:
            # A retired thread is not a zero counter. Coverage is checked below.
            continue
    group["tasks"] = tasks
    return group


def resources(before, after, elapsed):
    G["ensure"](before["pid"] == after["pid"] and before["start_ticks"] == after["start_ticks"], "process identity changed")
    ticks = before["clock_ticks_per_second"]
    cpu = ((after["user_ticks"] + after["system_ticks"]) - (before["user_ticks"] + before["system_ticks"])) / ticks
    G["ensure"](cpu >= 0 and math.isfinite(elapsed) and elapsed > 0, "invalid CPU/window")
    a, b = before["tasks"], after["tasks"]
    covered = bool(a) and set(a) == set(b) and len(a) == before["threads"] == after["threads"] and all(
        a[k]["start_ticks"] == b[k]["start_ticks"] for k in a)
    switches = {}
    if covered:
        for field in ("voluntary", "involuntary"):
            delta = sum(b[k][field] - a[k][field] for k in a)
            G["ensure"](delta >= 0, "scheduler counters decreased")
            switches[field + "_per_second"] = delta / elapsed
    return {"cpu_seconds": cpu, "matched_thread_coverage": covered, "switches": switches}


def server_argv(binary, variant, mode, port, cpus, directory):
    # Keep the protected Valkey launcher, with a one-worker Rillcache control.
    args = D.G["server_argv"](binary, "valkey" if variant == "valkey" else "rillcache", port, cpus, directory)
    if variant != "valkey" and mode == "one-worker":
        args += ["--socket-workers", "1"]
    return args


@contextlib.contextmanager
def owned_server(binary, variant, mode, cpus, directory):
    port = C.available_port()
    argv = server_argv(binary, variant, mode, port, cpus, directory)
    saved = {"argv": argv, "port": port, "cpus": cpus, "minimal_environment": True}
    with (directory / "server.log").open("wb") as log:
        proc = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT, start_new_session=True, env=SAFE_ENV)
        saved["pid"] = proc.pid
        try:
            deadline = time.monotonic() + 10
            while True:
                G["ensure"](proc.poll() is None, "server exited before readiness")
                try:
                    with C.Client(port) as client:
                        G["ensure"](client.call("PING") == b"PONG", "invalid readiness")
                    break
                except (OSError, ConnectionError):
                    G["ensure"](time.monotonic() < deadline, "readiness deadline")
                    time.sleep(.01)
            saved["actual_cpus_before"] = sorted(os.sched_getaffinity(proc.pid))
            G["ensure"](saved["actual_cpus_before"] == cpus, "server affinity differs")
            yield proc, port
            G["ensure"](proc.poll() is None, "server exited during load")
            saved["actual_cpus_after"] = sorted(os.sched_getaffinity(proc.pid))
            G["ensure"](saved["actual_cpus_after"] == cpus, "server affinity changed")
        finally:
            saved["exit_before_cleanup"] = proc.poll()
            saved["exit_after_cleanup"] = C.terminate(proc)
            saved["joined"] = True
            C.save(directory / "server.json", saved)


def sample(binaries, identity, cpus, directory):
    mode, workload, pipeline, run, variant = identity
    directory.mkdir(parents=True)
    result = {"valid": False, "identity": list(identity)}
    try:
        with owned_server(binaries[variant], variant, mode, cpus, directory) as (proc, port):
            result.update(pid=proc.pid, port=port)
            result["preload"] = G["validate_dataset"](port, workload, preload=True)
            warm = G["generator_argv"](binaries["generator"], port, workload, pipeline, G["WARMUP"], cpus)
            C.command(warm, directory / "warmup", timeout=180, env=SAFE_ENV)
            result["warmup"] = G["read_generator"](directory / "warmup", workload, warm)
            result["warmup_validation"] = G["validate_dataset"](port, workload, after=True)
            before = snapshot(proc.pid)
            started = time.monotonic()
            argv = G["generator_argv"](binaries["generator"], port, workload, pipeline, None, cpus)
            C.command(argv, directory / "measured", timeout=180, env=SAFE_ENV)
            elapsed = time.monotonic() - started
            after = snapshot(proc.pid)
            result.update(before=before, after=after, window_seconds=elapsed, resources=resources(before, after, elapsed))
            result["metrics"] = G["read_generator"](directory / "measured", workload, argv)
            result["measured_validation"] = G["validate_dataset"](port, workload, after=True)
        result["valid"] = True
    finally:
        C.save(directory / "sample.json", result)


def rows_for(samples):
    G["ensure"]([s["identity"] for s in samples] == [list(i) for i in identities()], "incomplete/reordered diagnostic")
    rows = []
    for mode in MODES:
        for workload in WORKLOADS:
            for pipeline in PIPELINES:
                values = {}
                for variant in VARIANTS:
                    records = [s for s in samples if s["identity"][:3] == [mode, workload, pipeline] and s["identity"][4] == variant]
                    rates = [s["metrics"]["rps"] for s in records]
                    values[variant] = {"median": statistics.median(rates), "min": min(rates), "max": max(rates),
                                       "cpu_seconds": statistics.median(s["resources"]["cpu_seconds"] for s in records),
                                       "p99_ms": statistics.median(s["metrics"]["p99_latency_ms"] for s in records)}
                    for kind in ("voluntary", "involuntary"):
                        values[variant][kind + "_per_second"] = statistics.median(
                            s["resources"]["switches"][kind + "_per_second"] for s in records) if all(
                            s["resources"]["matched_thread_coverage"] for s in records) else None
                rows.append({"mode": mode, "workload": workload, "pipeline": pipeline, "servers": values})
    return rows


def report(rows):
    text = ["# Same-job reactor A/B diagnostic — NOT a merge gate", "",
            "72 uninstrumented ten-second samples; baseline/candidate/Valkey in three Latin-rotated repetitions; "
            "PING/GET P1/P16, default vs one Rillcache socket worker, same shared four logical CPUs and official generator. "
            "Hardware/software fingerprint is captured BEFORE building and checked unchanged afterward; hosted runners "
            "are not dedicated/stable hardware. This is distinct from isolated fixed-host screening.", "",
            "| Mode | Workload | P | Server | Median [min–max] req/s | p99 ms | CPU seconds | Voluntary / involuntary switches/s |",
            "| --- | --- | ---: | --- | ---: | ---: | ---: | ---: |"]
    for row in rows:
        for name, v in row["servers"].items():
            a, b = v["voluntary_per_second"], v["involuntary_per_second"]
            switches = f"{a:.1f} / {b:.1f}" if a is not None and b is not None else "unknown (thread coverage changed)"
            text.append(f"| {row['mode']} | {row['workload']} | {row['pipeline']} | {name} | "
                        f"{v['median']:.2f} [{v['min']:.2f}–{v['max']:.2f}] | {v['p99_ms']:.3f} | {v['cpu_seconds']:.3f} | {switches} |")
    text += ["", "All losses/spreads retained. /proc counters bracket the generator window; scheduler deltas require "
             "the same live thread identities at both boundaries. They are not per-request costs or energy measurements. "
             "p99 is upstream pipeline latency, not per-command latency at P>1. There are no profiles or host tuning. "
             "A subset improvement cannot replace any current-head protected check, any of the original 20 or four-CPU "
             "10 Valkey medians, or the existing Dragonfly comparisons. No cross-host before/after speedup claim.", ""]
    return "\n".join(text)


def check(root):
    root = Path(root)
    m = G["read_json"](root / "manifest.json")
    G["ensure"](m["complete"] is True and m["diagnostic_only"] is True, "incomplete diagnostic")
    G["ensure"](m["evidence_sha256"] == D.evidence_hashes(root), "raw evidence hash differs")
    D.HARDWARE["require_match"](m["hardware_contract"], m["hardware_fingerprint"])
    D.validate_build_environment(m["hardware_contract"])
    G["ensure"](m["base_source"] != m["candidate_source"] and all(re.fullmatch(r"[0-9a-f]{40}", m[k])
           for k in ("base_source", "candidate_source")), "source identity differs")
    G["ensure"](m["cpus"] == m["hardware_contract"]["selected_cpus"], "placement differs")
    G["ensure"](set(m["binaries"]) == {*VARIANTS, "generator"} == set(m["binary_sha256"]), "binary roles differ")
    G["ensure"](len(set(m["binary_sha256"].values())) == 4, "binary roles reused")
    for name, expected in m["binary_sha256"].items():
        G["ensure"](C.digest(root / "frozen-binaries" / name) == expected, "frozen binary changed")
    for name in ("baseline", "candidate"):
        G["ensure"]("Compiling rillcache v" in (root / "build" / name / "stderr.log").read_text(), "application not rebuilt")
    ref = G["read_json"](root / "reference-identity.json")
    G["ensure"](ref["identity"]["hardware_fingerprint"] == m["hardware_fingerprint"] and
                ref["identity"]["source_archive_sha256"] == C.PINS["upstreams"]["valkey"]["sha256"] and
                ref["binary_sha256"] == {k: m["binary_sha256"][k] for k in ("valkey", "generator")}, "reference identity differs")
    samples = []
    for identity in identities():
        mode, workload, pipeline, _, variant = identity
        directory = root / name_for(identity)
        s, server = G["read_json"](directory / "sample.json"), G["read_json"](directory / "server.json")
        G["ensure"](s["valid"] is True and s["identity"] == list(identity), "invalid sample")
        G["ensure"](server["joined"] is True and server["exit_before_cleanup"] is None and
                    server["pid"] == s["pid"] and server["port"] == s["port"], "invalid lifecycle")
        cpus = m["cpus"]
        G["ensure"](server["cpus"] == cpus == server["actual_cpus_before"] == server["actual_cpus_after"], "affinity differs")
        launch = Path(m["launch_directory"]) / directory.name
        G["ensure"](server["argv"] == server_argv(m["binaries"][variant], variant, mode, s["port"], cpus, launch), "server command differs")
        G["ensure"](s["before"]["actual_cpus"] == cpus == s["after"]["actual_cpus"] and
                    s["before"]["pid"] == s["pid"] == s["after"]["pid"], "resource identity differs")
        G["ensure"](s["resources"] == resources(s["before"], s["after"], s["window_seconds"]), "resource counters differ")
        valid = {"validated_keys": 0 if workload == "ping" else G["KEYSPACE"], "ping": True}
        G["ensure"](s["preload"] == s["warmup_validation"] == s["measured_validation"] == valid, "dataset validation differs")
        for phase, count in (("warmup", G["WARMUP"]), ("measured", None)):
            argv = G["generator_argv"](m["binaries"]["generator"], s["port"], workload, pipeline, count, cpus)
            metrics = G["read_generator"](directory / phase, workload, argv)
            G["ensure"](s["warmup" if phase == "warmup" else "metrics"] == metrics, "CSV differs")
        samples.append(s)
    rows = rows_for(samples)
    G["ensure"](G["read_json"](root / "results.json") == rows and (root / "summary.md").read_text() == report(rows), "derived results differ")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base")
    parser.add_argument("--candidate")
    parser.add_argument("--expected-hardware")
    parser.add_argument("--check", type=Path)
    args = parser.parse_args()
    if args.check:
        print(report(check(args.check)))
        return
    G["ensure"](os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch", "manual Actions diagnostic only")
    G["ensure"](all(re.fullmatch(r"[0-9a-f]{40}", value or "") for value in (args.base, args.candidate)) and args.base != args.candidate, "distinct exact source SHAs required")
    G["ensure"](subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip() == args.candidate, "checkout is not candidate source")
    OUT.mkdir(parents=True, exist_ok=False)
    m = {"diagnostic_only": True, "complete": False, "base_source": args.base, "candidate_source": args.candidate}
    C.save(OUT / "manifest.json", m)
    try:
        contract = D.HARDWARE["probe"]()
        fingerprint = D.HARDWARE["require_match"](contract, args.expected_hardware)
        D.validate_build_environment(contract)
        m.update(hardware_contract=contract, hardware_fingerprint=fingerprint, cpus=contract["selected_cpus"], launch_directory=str(OUT))
        C.save(OUT / "manifest.json", m)
        D.OUT, D.BIN, D.BASE, D.CANDIDATE = OUT, OUT / "frozen-binaries", args.base, args.candidate
        binaries = D.build_sources()
        hashes = {name: C.digest(path) for name, path in binaries.items()}
        m.update(binaries=binaries, binary_sha256=hashes)
        samples = []
        for identity in identities():
            G["ensure"](all(C.digest(path) == hashes[name] for name, path in binaries.items()), "binary changed during diagnostic")
            directory = OUT / name_for(identity)
            sample(binaries, identity, m["cpus"], directory)
            samples.append(G["read_json"](directory / "sample.json"))
            print(name_for(identity), "validated", flush=True)
        D.HARDWARE["require_match"](D.HARDWARE["probe"](), fingerprint)
        rows = rows_for(samples)
        C.save(OUT / "results.json", rows)
        (OUT / "summary.md").write_text(report(rows))
        m.update(complete=True, evidence_sha256=D.evidence_hashes(OUT))
        C.save(OUT / "manifest.json", m)
        check(OUT)
        C.summary(report(rows), OUT / "summary.md")
    except BaseException as error:
        m.update(complete=False, error=str(error))
        C.save(OUT / "manifest.json", m)
        raise


if __name__ == "__main__":
    main()

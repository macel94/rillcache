#!/usr/bin/env python3
"""Isolated normal-build large-string RTT diagnostic; not a protected gate.

This tool-only branch uses the existing read-only manual native-study workflow.
It is not proposed for merge. Earlier reactor studies keep their own harness.
"""
import argparse
import hashlib
import importlib.util
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("string_build_tools", ROOT / "scripts/dev-performance.py")
D = importlib.util.module_from_spec(spec)
spec.loader.exec_module(D)
G, C = D.G, D.COMMON
OUT = ROOT / "target/upstream/evidence/reactor-study"
VARIANTS = ("baseline", "candidate", "valkey")
CASES = (("append-64", 64, "APPEND", 8, 200),
         ("setrange-64", 64, "SETRANGE", 8, 200),
         ("append-1m", 1024 * 1024, "APPEND", 64, 200),
         ("setrange-1m", 1024 * 1024, "SETRANGE", 16, 200),
         ("append-32m", 32 * 1024 * 1024, "APPEND", 64, 100),
         ("setrange-32m", 32 * 1024 * 1024, "SETRANGE", 16, 100),
         ("getrange-32m", 32 * 1024 * 1024, "GETRANGE", 16, 100))


def identities():
    for case in CASES:
        for round_index in range(3):
            for variant in VARIANTS[round_index:] + VARIANTS[:round_index]:
                yield case, round_index + 1, variant


def initial_value(size):
    return (bytes(range(256)) * ((size + 255) // 256))[:size]


def expected_value(case):
    _, size, verb, count, repetitions = case
    value = bytearray(initial_value(size))
    if verb == "APPEND":
        value.extend(bytes([0xa5]) * count * repetitions)
    elif verb == "SETRANGE":
        value[:count] = bytes([0xa5]) * count
    return value


def command_for(case):
    _, _, verb, count, _ = case
    if verb == "APPEND":
        return ("APPEND", "k", bytes([0xa5]) * count)
    if verb == "SETRANGE":
        return ("SETRANGE", "k", "0", bytes([0xa5]) * count)
    return ("GETRANGE", "k", "0", str(count - 1))


def expected_reply(case, index):
    _, size, verb, count, _ = case
    if verb == "APPEND":
        return size + count * (index + 1)
    if verb == "SETRANGE":
        return size
    return initial_value(count).hex()


def percentiles(latencies):
    assert latencies and all(type(x) is int and x > 0 for x in latencies)
    ordered = sorted(latencies)
    return {"p50_ms": statistics.median(ordered) / 1e6,
            "p99_ms": ordered[math.ceil(.99 * len(ordered)) - 1] / 1e6,
            "max_ms": ordered[-1] / 1e6}


def name_for(case, round_number, variant):
    return f"{case[0]}-r{round_number}-{variant}"


def sample(binaries, identity, cpus):
    case, round_number, variant = identity
    directory = OUT / name_for(*identity)
    directory.mkdir()
    result = {"case": list(case), "round": round_number, "variant": variant,
              "valid": False, "instrumented": False, "latencies_ns": [], "replies": []}
    try:
        with D.owned_server(binaries[variant], variant, cpus, directory) as (server, port):
            result["port"] = port
            with G["Client"](port) as client:
                # Bounded requests; setup, full-value verification and readiness
                # are outside timers. No operation warmup hides the first growth.
                seed = initial_value(case[1])
                for offset in range(0, len(seed), 512 * 1024):
                    end = min(offset + 512 * 1024, len(seed))
                    assert client.call("APPEND", "k", seed[offset:end]) == end
                assert client.call("STRLEN", "k") == len(seed)
                result["generator_before"] = D.snapshot(os.getpid())
                result["before"] = D.snapshot(server.pid)
                args = command_for(case)
                for index in range(case[4]):
                    start = time.perf_counter_ns()
                    reply = client.call(*args)
                    elapsed = time.perf_counter_ns() - start
                    encoded = reply.hex() if isinstance(reply, bytes) else reply
                    result["latencies_ns"].append(elapsed)
                    result["replies"].append(encoded)
                    assert encoded == expected_reply(case, index), "measured reply differs"
                result["after"] = D.snapshot(server.pid)
                result["generator_after"] = D.snapshot(os.getpid())
                expected = expected_value(case)
                assert client.call("STRLEN", "k") == len(expected)
                actual_hash = hashlib.sha256()
                for offset in range(0, len(expected), 512 * 1024):
                    end = min(offset + 512 * 1024, len(expected))
                    chunk = client.call("GETRANGE", "k", str(offset), str(end - 1))
                    assert chunk == expected[offset:end], "full final-value verification failed"
                    actual_hash.update(chunk)
                result.update(expected_sha256=hashlib.sha256(expected).hexdigest(),
                              actual_sha256=actual_hash.hexdigest(), final_size=len(expected))
        result.update(valid=True, metrics=percentiles(result["latencies_ns"]))
        return result
    finally:
        C.save(directory / "sample.json", result)


def report(samples):
    lines = ["# Normal-build matched large-string RTT diagnostic", "",
             "Advisory; NOT a protected throughput gate. Same host/toolchain, four shared logical CPUs, "
             "one sequential Python RESP connection, no perf/trace instrumentation or operation warmup. "
             "Includes first growth. 200 calls for small/1 MiB cases, 100 for 32 MiB. "
             "Three Latin-rotated baseline/candidate/Valkey rounds. Setup and full byte verification excluded. "
             "RTT includes Python encode/parse and scheduling; not isolated engine latency. "
             "All rounds, p99 and max retained; no selected or pooled favorable samples.", "",
             "| Case | Round | Baseline p50 / p99 / max ms | Candidate p50 / p99 / max ms | Valkey p50 / p99 / max ms |",
             "| --- | ---: | ---: | ---: | ---: |"]
    for case in CASES:
        for round_number in range(1, 4):
            values = []
            for variant in VARIANTS:
                sample = next(s for s in samples if (s["case"][0], s["round"], s["variant"]) == (case[0], round_number, variant))
                metrics = sample["metrics"]
                values.append(" / ".join(f"{metrics[k]:.6f}" for k in ("p50_ms", "p99_ms", "max_ms")))
            lines.append(f"| {case[0]} | {round_number} | " + " | ".join(values) + " |")
    return "\n".join(lines) + "\n"


def check(root):
    root = Path(root)
    manifest = G["read_json"](root / "manifest.json")
    assert manifest["complete"] and manifest["diagnostic_only"] and manifest["study"] == "in-place-strings-v1"
    assert manifest["instrumented"] is False and manifest["protected_gate"] is False
    assert all(re.fullmatch(r"[0-9a-f]{40}", manifest[k]) for k in ("base_source", "candidate_source", "runtime_candidate"))
    assert manifest["base_source"] != manifest["candidate_source"]
    assert manifest["evidence_sha256"] == D.evidence_hashes(root)
    hardware = manifest["hardware"]
    D.validate_build_environment(hardware)
    assert D.HARDWARE["fingerprint"](hardware) == manifest["hardware_fingerprint"]
    cpus = hardware["selected_cpus"]
    assert len(set(cpus)) == len(cpus) == 4
    assert set(manifest["binaries"]) == {"baseline", "candidate", "valkey", "generator"}
    assert set(manifest["binary_sha256"]) == set(manifest["binaries"])
    assert len(list(root.glob("*-r*/sample.json"))) == 63
    for name, digest in manifest["binary_sha256"].items():
        assert C.digest(root / "frozen-binaries" / name) == digest
    samples = []
    for identity in identities():
        case, round_number, variant = identity
        directory = root / name_for(*identity)
        s = G["read_json"](directory / "sample.json")
        assert s["valid"] is True and s["instrumented"] is False
        assert (s["case"], s["round"], s["variant"]) == (list(case), round_number, variant)
        assert len(s["latencies_ns"]) == case[4]
        assert s["replies"] == [expected_reply(case, i) for i in range(case[4])]
        assert s["metrics"] == percentiles(s["latencies_ns"])
        expected = expected_value(case)
        assert s["final_size"] == len(expected)
        assert s["actual_sha256"] == s["expected_sha256"] == hashlib.sha256(expected).hexdigest()
        before, after = s["before"], s["after"]
        assert before["pid"] == after["pid"] and before["start_ticks"] == after["start_ticks"]
        assert before["actual_cpus"] == after["actual_cpus"] == cpus
        driver_before, driver_after = s["generator_before"], s["generator_after"]
        assert driver_before["pid"] == driver_after["pid"]
        assert driver_before["start_ticks"] == driver_after["start_ticks"]
        assert driver_before["actual_cpus"] == driver_after["actual_cpus"] == cpus
        server = G["read_json"](directory / "server.json")
        launch = Path(manifest["launch_directory"]) / name_for(*identity)
        kind = "valkey" if variant == "valkey" else "rillcache"
        assert server["argv"] == G["server_argv"](manifest["binaries"][variant], kind, s["port"], cpus, launch)
        assert server["pid"] == before["pid"] and server["port"] == s["port"]
        assert server["joined"] and server["exit_before_cleanup"] is None and server["exit_after_cleanup"] is not None
        samples.append(s)
    assert G["read_json"](root / "samples.json") == samples
    assert (root / "summary.md").read_text() == report(samples)
    return samples


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base")
    parser.add_argument("--candidate")
    parser.add_argument("--mechanism-control")
    parser.add_argument("--expected-hardware")
    parser.add_argument("--check", type=Path)
    args = parser.parse_args()
    if args.check:
        check(args.check)
        print("Complete normal-build large-string evidence revalidated; NOT a protected gate pass.")
        return
    assert os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch"
    assert not args.mechanism_control
    assert all(re.fullmatch(r"[0-9a-f]{40}", x or "") for x in (args.base, args.candidate))
    assert args.base != args.candidate
    assert subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip() == args.candidate
    OUT.mkdir(parents=True, exist_ok=False)
    manifest = {"study": "in-place-strings-v1", "diagnostic_only": True, "protected_gate": False,
                "instrumented": False, "complete": False, "base_source": args.base, "candidate_source": args.candidate,
                "runtime_candidate": subprocess.check_output(["git", "log", "-1", "--format=%H", "--", "src", "Cargo.toml", "Cargo.lock", "vendor"], cwd=ROOT).decode().strip(),
                "launch_directory": str(OUT)}
    try:
        hardware = D.HARDWARE["probe"]()
        fingerprint = D.HARDWARE["require_match"](hardware, args.expected_hardware)
        D.validate_build_environment(hardware)
        manifest.update(hardware=hardware, hardware_fingerprint=fingerprint)
        D.OUT, D.BIN, D.BASE, D.CANDIDATE = OUT, OUT / "frozen-binaries", args.base, args.candidate
        binaries = D.build_sources()
        hashes = {name: C.digest(path) for name, path in binaries.items()}
        manifest.update(binaries=binaries, binary_sha256=hashes)
        samples = []
        os.sched_setaffinity(0, hardware["selected_cpus"])
        for identity in identities():
            assert all(C.digest(path) == hashes[name] for name, path in binaries.items())
            samples.append(sample(binaries, identity, hardware["selected_cpus"]))
            print(name_for(*identity), "validated", flush=True)
        D.HARDWARE["require_match"](D.HARDWARE["probe"](), fingerprint)
        C.save(OUT / "samples.json", samples)
        (OUT / "summary.md").write_text(report(samples))
        manifest.update(complete=True, evidence_sha256=D.evidence_hashes(OUT))
        C.save(OUT / "manifest.json", manifest)
        check(OUT)
        C.summary(report(samples), OUT / "summary.md")
    except BaseException as error:
        manifest.update(complete=False, error=str(error))
        raise
    finally:
        C.save(OUT / "manifest.json", manifest)


if __name__ == "__main__":
    main()

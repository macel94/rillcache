#!/usr/bin/env python3
"""Actions-only owned P1 CPU/syscall captures; NEVER normal performance evidence."""
import argparse
import importlib.util
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("reactor_profile_tools", ROOT / "scripts/dev-performance.py")
D = importlib.util.module_from_spec(spec)
spec.loader.exec_module(D)
G, C = D.G, D.COMMON
OUT = ROOT / "target/upstream/evidence/reactor-profile"
ORIGINAL_TERMINATE = G["terminate"]


def identities():
    return [(mode, workload, variant) for mode in ("syscalls", "cpu")
            for workload in ("ping", "get") for variant in ("rillcache", "valkey")]


def load_argv(generator, port, workload, cpus):
    argv = G["generator_argv"](generator, port, workload, 1, None, cpus)
    # The existing collectors attach after 0.5s and run for 8s. Keep both owned
    # targets alive well past capture completion; these rates are quarantined.
    argv[argv.index("--duration") + 1] = "20"
    return argv


def terminate_owned(proc):
    args = list(proc.args) if isinstance(proc.args, (list, tuple)) else []
    if len(args) < 3 or args[:2] != ["sudo", "-n"] or Path(args[2]).name != "perf":
        return ORIGINAL_TERMINATE(proc)
    # Only our start_new_session=True sudo/perf collectors require privileged
    # signals. Never signal a caller-supplied PID or a shared runner group.
    assert isinstance(proc.pid, int) and proc.pid > 1
    if proc.poll() is None:
        assert os.getpgid(proc.pid) == proc.pid, "collector lost its owned session"
        subprocess.run(["sudo", "-n", "kill", "-TERM", "--", f"-{proc.pid}"],
                       check=False, timeout=10, env=D.SAFE_ENV)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            subprocess.run(["sudo", "-n", "kill", "-KILL", "--", f"-{proc.pid}"],
                           check=False, timeout=10, env=D.SAFE_ENV)
            proc.wait(timeout=5)
    # Perf's bounded sleep can outlive an abnormal collector exit.
    subprocess.run(["sudo", "-n", "kill", "-KILL", "--", f"-{proc.pid}"],
                   check=False, timeout=10, env=D.SAFE_ENV, capture_output=True)
    return proc.returncode


def build(source):
    D.BIN.mkdir(parents=True, exist_ok=False)
    C.checked(["git", "fetch", "--no-tags", "origin", source], OUT / "fetch", timeout=180)
    tree = Path(os.environ["RUNNER_TEMP"]) / "rillcache-profile-source"
    tree.mkdir()
    archive = subprocess.Popen(["git", "archive", source], cwd=ROOT, stdout=subprocess.PIPE)
    try:
        subprocess.run(["tar", "-x", "-C", str(tree)], stdin=archive.stdout, check=True, timeout=120)
    finally:
        archive.stdout.close()
        assert archive.wait(timeout=30) == 0, "source export failed"
    D.compile_application(tree, "profile-source")
    shutil.copy2(tree / "target/release/rillcache", D.BIN / "rillcache")
    D.build_reference()
    version = C.PINS["upstreams"]["valkey"]["version"]
    shutil.copy2(ROOT / f"target/competitors/valkey-{version}/COPYING", D.BIN / "VALKEY-COPYING")
    return {name: str((D.BIN / name).resolve()) for name in ("rillcache", "valkey", "generator")}


def sample(binaries, identity, cpus, perf):
    mode, workload, variant = identity
    directory = OUT / "captures" / "-".join(identity)
    directory.mkdir(parents=True)
    result = {"identity": list(identity), "instrumented": True, "valid": False}
    try:
        with D.owned_server(binaries[variant], variant, cpus, directory) as (server, port):
            G["validate_dataset"](port, workload, preload=True)
            warm = G["generator_argv"](binaries["generator"], port, workload, 1, G["WARMUP"], cpus)
            C.checked(warm, directory / "warmup", timeout=180, env=D.SAFE_ENV)
            G["read_generator"](directory / "warmup", workload, warm)
            before = D.snapshot(server.pid)
            argv = load_argv(binaries["generator"], port, workload, cpus)
            command = D.measured_load(argv, directory / "measured", server, mode, perf, "cpu-clock")
            after = D.snapshot(server.pid)
            assert before["start_ticks"] == after["start_ticks"]
            assert command["wall_seconds"] >= 20
            metrics = G["read_generator"](directory / "measured", workload, argv)
            G["validate_dataset"](port, workload, after=True)
            result.update(before=before, after=after, command=command, metrics=metrics)
        if mode == "cpu":
            for role in ("server", "generator"):
                data = directory / "measured" / f"{role}.perf.data"
                for children in (False, True):
                    suffix = "children" if children else "self"
                    path = directory / f"{role}-{suffix}.txt"
                    D.cpu_report(perf, data, path, children)
                    assert re.search(r"Samples:\s+[1-9]", path.read_text()), "no CPU samples"
        result["valid"] = True
        return result
    finally:
        G["save"](directory / "sample.json", result)


def report(samples):
    lines = ["# Instrumented P1 CPU/syscall diagnosis — NOT throughput qualification", "",
             "Normal production-release binaries; eight clients, two official-generator threads, "
             "64-byte values, 1024 keys, seed 42, P1; four shared logical CPUs. "
             "Each 20-second instrumented load contains an 8-second owned server/generator capture. "
             "No normal medians, rankings, energy or per-command costs are inferred. "
             "Hosted hardware is not a dedicated-host proof. CPU-clock sampling and tracepoints "
             "may perturb the workload. Unwinding/symbol coverage remains visible in raw reports.", "",
             "| Capture | Server user/system ticks during profiled load | Reports |", "| --- | ---: | --- |"]
    for s in samples:
        mode, workload, variant = s["identity"]
        before, after = s["before"], s["after"]
        name = "-".join(s["identity"])
        target = f"captures/{name}/server-self.txt" if mode == "cpu" else f"captures/{name}/measured/server-syscalls.txt"
        lines.append(f"| {name} | {after['user_ticks'] - before['user_ticks']} / "
                     f"{after['system_ticks'] - before['system_ticks']} | [raw]({target}) |")
    return "\n".join(lines) + "\n"


def check(root):
    root = Path(root)
    m = G["read_json"](root / "manifest.json")
    assert m["complete"] is True and m["instrumented"] is True and m["normal_performance_evidence"] is False
    assert re.fullmatch(r"[0-9a-f]{40}", m["application_source"])
    assert len(m["cpus"]) == len(set(m["cpus"])) == 4
    hardware = G["read_json"](root / "hardware.json")
    D.validate_build_environment(hardware)
    assert hardware["selected_cpus"] == m["cpus"]
    assert D.HARDWARE["fingerprint"](hardware) == m["hardware_fingerprint"]
    assert m["evidence_sha256"] == D.evidence_hashes(root), "evidence changed"
    assert set(m["binary_sha256"]) == {"rillcache", "valkey", "generator"}
    for name, digest in m["binary_sha256"].items():
        assert G["digest"](root / "frozen-binaries" / name) == digest
    assert len(m["samples"]) == 8
    assert {tuple(s["identity"]) for s in m["samples"]} == set(identities())
    for s in m["samples"]:
        directory = root / "captures" / "-".join(s["identity"])
        assert s == G["read_json"](directory / "sample.json")
        assert s["valid"] is True and s["instrumented"] is True
        before, after, command = s["before"], s["after"], s["command"]
        assert before["pid"] == after["pid"] and before["start_ticks"] == after["start_ticks"]
        assert before["actual_cpus"] == after["actual_cpus"] == m["cpus"]
        assert command["exit_code"] == 0 and not command.get("timed_out")
        assert command["wall_seconds"] >= 20
        assert command["argv"][command["argv"].index("--duration") + 1] == "20"
        assert s["metrics"] == G["read_generator"](directory / "measured", s["identity"][1], command["argv"])
        server = G["read_json"](directory / "server.json")
        assert command["argv"] == load_argv(m["binary_paths"]["generator"], server["port"], s["identity"][1], m["cpus"])
        original_directory = Path(m["launch_directory"]) / "captures" / "-".join(s["identity"])
        variant = s["identity"][2]
        kind = "valkey" if variant == "valkey" else "rillcache"
        assert server["argv"] == G["server_argv"](m["binary_paths"][variant], kind, server["port"], m["cpus"], original_directory)
        assert server["joined"] is True and server["pid"] == before["pid"]
        assert server["exit_before_cleanup"] is None and server["exit_after_cleanup"] is not None
        for role, pid in (("server", before["pid"]), ("generator", command["pid"])):
            capture = G["read_json"](directory / "measured" / f"{role}-profiler.json")
            assert capture["exit_code"] == 0
            argv = capture["argv"]
            assert argv[:2] == ["sudo", "-n"] and Path(argv[2]).name == "perf"
            assert argv[3] == ("record" if s["identity"][0] == "cpu" else "stat")
            assert argv[-3:] == ["--", "sleep", "8"]
            assert argv[argv.index("-p") + 1] == str(pid)
            assert command[f"{role}_at_capture"]["pid"] == pid
            if s["identity"][0] == "cpu":
                assert (directory / "measured" / f"{role}.perf.data").stat().st_size > 0
                for suffix in ("self", "children"):
                    assert re.search(r"Samples:\s+[1-9]", (directory / f"{role}-{suffix}.txt").read_text())
            else:
                text = (directory / "measured" / f"{role}-syscalls.txt").read_text()
                assert not any(x in text for x in ("<not counted>", "<not supported>"))
                assert all(x in text for x in ("sys_enter_epoll_wait", "sys_exit_recvfrom", "sys_enter_writev"))
    assert (root / "summary.md").read_text() == report(m["samples"])
    return m


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source")
    parser.add_argument("--expected-hardware")
    parser.add_argument("--check", type=Path)
    args = parser.parse_args(argv)
    if args.check:
        check(args.check)
        print("Complete instrumented evidence validated; NOT a performance pass.")
        return 0
    assert os.environ.get("GITHUB_ACTIONS") == "true", "native profiling is Actions-only"
    assert os.environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch", "explicit dispatch required"
    assert args.source and re.fullmatch(r"[0-9a-f]{40}", args.source), "exact application source required"
    OUT.mkdir(parents=True, exist_ok=False)
    D.OUT, D.BIN = OUT, OUT / "frozen-binaries"
    G["terminate"] = terminate_owned
    m = {"instrumented": True, "normal_performance_evidence": False, "complete": False,
         "application_source": args.source, "samples": []}
    try:
        hardware = D.HARDWARE["probe"]()
        m["hardware_fingerprint"] = D.HARDWARE["require_match"](hardware, args.expected_hardware)
        G["save"](OUT / "hardware.json", hardware)
        D.validate_build_environment(hardware)
        cpus = hardware["selected_cpus"]
        assert len(cpus) == 4
        # Capability probe precedes all source builds; a permission/tool blocker
        # must be explicit, never replaced with fabricated or partial profiles.
        perf, _ = D.perf_tool()
        C.checked([perf, "--version"], OUT / "perf-version", timeout=20)
        binaries = build(args.source)
        hashes = {name: G["digest"](path) for name, path in binaries.items()}
        m.update(cpus=cpus, binary_sha256=hashes, binary_paths=binaries,
                 launch_directory=str(OUT.resolve()), cpu_event="cpu-clock")
        for identity in identities():
            assert all(G["digest"](path) == hashes[name] for name, path in binaries.items())
            m["samples"].append(sample(binaries, identity, cpus, perf))
            G["save"](OUT / "manifest.json", m)
        D.HARDWARE["require_match"](D.HARDWARE["probe"](), args.expected_hardware)
        C.summary(report(m["samples"]), OUT / "summary.md")
        m.update(complete=True, evidence_sha256=D.evidence_hashes(OUT))
        G["save"](OUT / "manifest.json", m)
        check(OUT)
        return 0
    except BaseException as error:
        m.update(complete=False, error=str(error))
        raise
    finally:
        G["save"](OUT / "manifest.json", m)


if __name__ == "__main__":
    raise SystemExit(main())

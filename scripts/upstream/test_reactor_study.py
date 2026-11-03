"""Offline fixtures only: no native throughput claim or expensive local build."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import common

spec = importlib.util.spec_from_file_location("reactor_diagnostic", common.ROOT / "scripts/reactor-study.py")
R = importlib.util.module_from_spec(spec)
spec.loader.exec_module(R)


def fixture(root, control=None):
    variants = R.variants_for(control)
    contract = {"selected_cpus": [0, 1, 2, 3], "rust": f"rustc {common.PINS['rust']} (fixture)", "build_environment": {}}
    m = {"diagnostic_only": True, "complete": True, "base_source": "a" * 40, "candidate_source": "b" * 40,
         "hardware_contract": contract, "hardware_fingerprint": R.D.HARDWARE["fingerprint"](contract),
         "cpus": [0, 1, 2, 3], "launch_directory": str(root), "binaries": {}, "binary_sha256": {}}
    if control is not None:
        m["mechanism_control_source"] = control
    (root / "frozen-binaries").mkdir()
    for name in (*variants, "generator"):
        path = root / "frozen-binaries" / name
        path.write_text("synthetic fixture only: " + name)
        m["binaries"][name] = str(path)
        m["binary_sha256"][name] = common.digest(path)
    for name in variants:
        if name == "valkey":
            continue
        path = root / "build" / name
        path.mkdir(parents=True)
        (path / "stderr.log").write_text("Compiling rillcache v0.1.0 (fixture, not a real build)\n")
    notice = root / "frozen-binaries/VALKEY-COPYING"
    notice.write_text("synthetic notice fixture, not a redistributed binary")
    common.save(root / "reference-identity.json", {
        "notice_sha256": common.digest(notice),
        "identity": {"hardware_fingerprint": m["hardware_fingerprint"],
                     "source_archive_sha256": common.PINS["upstreams"]["valkey"]["sha256"]},
        "binary_sha256": {k: m["binary_sha256"][k] for k in ("valkey", "generator")}})
    samples = []
    for index, identity in enumerate(R.identities(variants)):
        mode, workload, pipeline, run, variant = identity
        directory = root / R.name_for(identity)
        directory.mkdir()
        before = {"pid": index + 1000, "start_ticks": 100, "user_ticks": 0, "system_ticks": 0,
                  "clock_ticks_per_second": 100, "actual_cpus": m["cpus"], "threads": 1,
                  "tasks": {"1": {"start_ticks": 100, "voluntary": 0, "involuntary": 0}}}
        after = copy.deepcopy(before)
        after["user_ticks"] = 900
        after["tasks"]["1"]["voluntary"] = 100
        after["tasks"]["1"]["involuntary"] = 10
        port = index + 20000
        metrics = {"rps": float(100 + variants.index(variant)), "min_latency_ms": .001,
                   "p50_latency_ms": .002, "p95_latency_ms": .003, "p99_latency_ms": .004,
                   "max_latency_ms": .005, "avg_latency_ms": .002}
        valid = {"validated_keys": 0 if workload == "ping" else 1024, "ping": True}
        s = {"identity": list(identity), "valid": True, "pid": before["pid"], "port": port,
             "before": before, "after": after, "window_seconds": 10.1,
             "resources": R.resources(before, after, 10.1), "warmup": metrics, "metrics": metrics,
             "preload": valid, "warmup_validation": valid, "measured_validation": valid}
        server = {"pid": s["pid"], "port": port, "cpus": m["cpus"], "actual_cpus_before": m["cpus"],
                  "actual_cpus_after": m["cpus"], "joined": True, "exit_before_cleanup": None,
                  "argv": R.server_argv(m["binaries"][variant], variant, mode, port, m["cpus"], directory)}
        common.save(directory / "server.json", server)
        common.save(directory / "sample.json", s)
        for phase, count in (("warmup", 8000), ("measured", None)):
            path = directory / phase
            path.mkdir()
            argv = R.G["generator_argv"](m["binaries"]["generator"], port, workload, pipeline, count, m["cpus"])
            common.save(path / "command.json", {"argv": argv, "exit_code": 0, "wall_seconds": 10.1})
            header = R.G["CSV_HEADER"]
            title = " ".join(R.G["workload_command"](workload))
            (path / "stdout.log").write_text(','.join(f'"{x}"' for x in header) + '\n' +
                f'"{title}",' + ','.join(str(metrics[x]) for x in header[1:]) + '\n')
            (path / "stderr.log").write_text("")
        samples.append(s)
    rows = R.rows_for(samples, variants)
    common.save(root / "results.json", rows)
    (root / "summary.md").write_text(R.report(rows))
    m["evidence_sha256"] = R.D.evidence_hashes(root)
    common.save(root / "manifest.json", m)
    return m, rows, samples


class ReactorStudyTests(unittest.TestCase):
    def test_balanced_complete_scope_includes_both_failing_commands_and_controls(self):
        ids = list(R.identities())
        self.assertEqual(108, len(ids))
        self.assertEqual(108, len(set(ids)))
        for mode in R.MODES:
            for workload in R.WORKLOADS:
                for pipeline in R.PIPELINES:
                    case = [i for i in ids if i[:3] == (mode, workload, pipeline)]
                    for position in range(3):
                        self.assertEqual(set(R.VARIANTS), {case[i][4] for i in range(position, 9, 3)})

    def test_optional_rejected_control_is_balanced_in_four_positions(self):
        variants = R.variants_for("c" * 40)
        ids = list(R.identities(variants))
        self.assertEqual(192, len(ids))
        self.assertEqual(192, len(set(ids)))
        for mode in R.MODES:
            for workload in R.WORKLOADS:
                for pipeline in R.PIPELINES:
                    case = [i for i in ids if i[:3] == (mode, workload, pipeline)]
                    for position in range(4):
                        self.assertEqual(set(variants), {case[i][4] for i in range(position, 16, 4)})

    def test_optional_control_fixture_requires_distinct_source_and_actual_rebuild(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            m, rows, samples = fixture(root, "c" * 40)
            self.assertEqual(192, len(samples))
            self.assertEqual(rows, R.check(root))
            m["mechanism_control_source"] = m["candidate_source"]
            common.save(root / "manifest.json", m)
            with self.assertRaisesRegex(ValueError, "source identity"):
                R.check(root)
            m["mechanism_control_source"] = "c" * 40
            (root / "build/mechanism-control/stderr.log").write_text("Finished release; stale fixture\n")
            m["evidence_sha256"] = R.D.evidence_hashes(root)
            common.save(root / "manifest.json", m)
            with self.assertRaisesRegex(ValueError, "not rebuilt"):
                R.check(root)

    def test_one_worker_control_never_changes_reference_or_default_launch(self):
        default = R.server_argv("/binary", "baseline", "default", 1234, [0, 1, 2, 3], Path("/tmp/sample"))
        one = R.server_argv("/binary", "candidate", "one-worker", 1234, [0, 1, 2, 3], Path("/tmp/sample"))
        self.assertNotIn("--socket-workers", default)
        self.assertEqual(default + ["--socket-workers", "1"], one)
        self.assertEqual(default + ["--socket-workers", "4"], R.server_argv(
            "/binary", "candidate", "four-workers", 1234, [0, 1, 2, 3], Path("/tmp/sample")))
        self.assertEqual(R.server_argv("/valkey", "valkey", "default", 1234, [0, 1, 2, 3], Path("/tmp/sample")),
                         R.server_argv("/valkey", "valkey", "one-worker", 1234, [0, 1, 2, 3], Path("/tmp/sample")))

    def test_complete_fixture_revalidates_and_rejects_missing_or_reordered_samples(self):
        with tempfile.TemporaryDirectory() as tmp:
            _, rows, samples = fixture(Path(tmp))
            self.assertEqual(rows, R.check(tmp))
            for bad in (samples[:-1], samples + samples[:1], list(reversed(samples))):
                with self.assertRaises(ValueError):
                    R.rows_for(bad)

    def test_thread_churn_is_unknown_not_invented_zero_and_pid_reuse_is_rejected(self):
        before = {"pid": 1, "start_ticks": 1, "user_ticks": 0, "system_ticks": 0,
                  "threads": 1, "clock_ticks_per_second": 100,
                  "tasks": {"1": {"start_ticks": 1, "voluntary": 0, "involuntary": 0}}}
        after = copy.deepcopy(before)
        after["tasks"]["1"]["start_ticks"] = 2
        result = R.resources(before, after, 10)
        self.assertFalse(result["matched_thread_coverage"])
        self.assertEqual({}, result["switches"])
        after["start_ticks"] = 2
        with self.assertRaises(ValueError):
            R.resources(before, after, 10)

    def test_raw_and_rehashed_corruption_cannot_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            m, _, _ = fixture(root)
            name = R.name_for(next(R.identities()))
            path = root / name / "sample.json"
            saved = path.read_bytes()
            for change in (lambda s: s.update(valid=False), lambda s: s["metrics"].update(rps=9999),
                           lambda s: s["after"].update(pid=42), lambda s: s["resources"].update(cpu_seconds=0)):
                sample = json.loads(saved)
                change(sample)
                common.save(path, sample)
                changed = copy.deepcopy(m)
                changed["evidence_sha256"] = R.D.evidence_hashes(root)
                common.save(root / "manifest.json", changed)
                with self.assertRaises(ValueError):
                    R.check(root)
                path.write_bytes(saved)
                common.save(root / "manifest.json", m)
            (root / "frozen-binaries/candidate").write_text("changed binary")
            with self.assertRaises(ValueError):
                R.check(root)

    def test_rehashed_reference_notice_corruption_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            m, _, _ = fixture(root)
            (root / "frozen-binaries/VALKEY-COPYING").write_text("changed notice")
            m["evidence_sha256"] = R.D.evidence_hashes(root)
            common.save(root / "manifest.json", m)
            with self.assertRaisesRegex(ValueError, "reference identity"):
                R.check(root)

    def test_manual_actions_guard_fails_before_builds(self):
        with patch.dict(os.environ, {}, clear=True), patch("sys.argv", ["reactor-study.py"]), \
             patch.object(R.D, "build_sources") as build:
            with self.assertRaises(ValueError):
                R.main()
            build.assert_not_called()


if __name__ == "__main__":
    unittest.main()

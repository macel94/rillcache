"""Offline daily-harness contracts; mocks/fixtures are NOT performance evidence."""
import copy
import io
import json
import os
from pathlib import Path
import re
import tarfile
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import common
import daily
import daily_prepare as prepare


def fixture(root):
    source = "a" * 40
    build = {"source": source, "tool_pins": prepare.PINS,
             "native_build": {"pins": common.PINS, "built_git_head": source},
             "comparison_generator": {"upstream": "valkey", "source_pin": common.PINS["upstreams"]["valkey"]},
             "binaries": {name: f"/frozen/{name}" for name in (*daily.SERVERS, "generator")},
             "binary_sha256": {name: f"{i + 1:064x}" for i, name in enumerate((*daily.SERVERS, "generator"))}}
    manifest = {"schema": daily.SCHEMA, "complete": True, "build": build,
                "environment": {"git_head": {"stdout": source + "\n"}, "GITHUB_SHA": source, "cpu_affinity": [0, 1, 2, 3]},
                "allowed_cpus": [0, 1, 2, 3], "server_cpus": [0, 1, 2, 3], "client_cpus": [0, 1, 2, 3],
                "servers": list(daily.SERVERS), "runs": daily.RUNS, "duration_seconds": 10, "warmup_requests": 8000,
                "workloads": list(daily.G["WORKLOADS"]), "pipelines": list(daily.G["PIPELINES"]),
                "launch_directory": str(root), "order": [], "evidence_sha256": {}}
    samples = []
    for index, identity in enumerate(daily.identities()):
        workload, pipeline, run, variant = identity
        directory = root / daily.sample_name(identity)
        directory.mkdir()
        # Rillcache loses every row: a valid advisory report must retain this.
        rate = 100 + daily.SERVERS.index(variant) * 20
        metrics = {"rps": float(rate), "min_latency_ms": .001, "p50_latency_ms": .002, "p95_latency_ms": .003,
                   "p99_latency_ms": .004, "max_latency_ms": .005, "avg_latency_ms": .002}
        valid = {"validated_keys": 0 if workload == "ping" else 1024, "ping": True}
        s = {"valid": True, "pid": 1000 + index, "port": 20000 + index, "preload": valid,
             "server_cpu_seconds": 8., "resource_window_seconds": 10.1, "server_rss_kib": 4096,
             "server_peak_rss_kib": 8192, "server_threads": 4}
        server = {"pid": s["pid"], "port": s["port"], "joined": True, "exit_before_cleanup": None,
                  "exit_after_cleanup": -15, "cpus": [0, 1, 2, 3], "actual_cpus_before": [0, 1, 2, 3],
                  "actual_cpus_after": [0, 1, 2, 3],
                  "argv": daily.server_argv(build["binaries"][variant], variant, s["port"], [0, 1, 2, 3], directory)}
        for phase, count in (("warmup", 8000), ("measured", None)):
            path = directory / phase
            path.mkdir()
            argv = daily.G["generator_argv"](build["binaries"]["generator"], s["port"], workload, pipeline, count, [0, 1, 2, 3])
            common.save(path / "command.json", {"argv": argv, "exit_code": 0, "wall_seconds": 10.1})
            header = daily.G["CSV_HEADER"]
            title = " ".join(daily.G["workload_command"](workload))
            (path / "stdout.log").write_text(
                ','.join(f'"{field}"' for field in header) + '\n' +
                f'"{title}",' + ','.join(str(metrics[field]) for field in header[1:]) + '\n')
            (path / "stderr.log").write_text("")
            s[phase], s[phase + "_validation"] = metrics, valid
        common.save(directory / "sample.json", s)
        common.save(directory / "server.json", server)
        (directory / "server.log").write_text("fixture only, not a real benchmark\n")
        manifest["order"].append(directory.name)
        manifest["evidence_sha256"].update({f"{directory.name}/{f}": common.digest(directory / f) for f in daily.RAW})
        samples.append(dict(workload=workload, pipeline=pipeline, run=run, variant=variant, **s))
    rows = daily.evaluate(samples)
    common.save(root / "manifest.json", manifest)
    common.save(root / "results.json", rows)
    (root / "summary.md").write_text(daily.report(rows))
    return manifest, rows, samples


class DailyTests(unittest.TestCase):
    def test_every_server_occupies_every_order_position_once_for_each_condition(self):
        identities = list(daily.identities())
        self.assertEqual(250, len(identities))
        self.assertEqual(250, len(set(identities)))
        for workload in daily.G["WORKLOADS"]:
            for pipeline in daily.G["PIPELINES"]:
                cohort = [i for i in identities if i[:2] == (workload, pipeline)]
                for position in range(5):
                    self.assertEqual(set(daily.SERVERS), {cohort[n][3] for n in range(position, 25, 5)})

    def test_launcher_contracts_and_sanitized_environment(self):
        for variant in daily.SERVERS:
            argv = daily.server_argv("/binary", variant, 12345, [2, 3, 4, 5], Path("/scratch"))
            self.assertEqual(["taskset", "-c", "2,3,4,5", "/binary"], argv[:4])
            self.assertIn("127.0.0.1" if variant != "rillcache" else "127.0.0.1:12345", argv)
            if variant == "garnet":
                self.assertEqual("64m", argv[argv.index("--memory") + 1])
                self.assertNotIn("--aof", argv)
                self.assertNotIn("--storage-tier", argv)
            if variant == "rillcache":
                self.assertNotIn("--threads", argv)
            if variant == "dragonfly":
                self.assertEqual("4", argv[argv.index("--proactor_threads") + 1])
        with self.assertRaises(ValueError):
            daily.server_argv("/binary", "unknown", 12345, [0, 1, 2, 3], Path("/scratch"))
        with patch.dict(os.environ, {"GH_TOKEN": "must-not-inherit", "DOTNET_ROOT": "/wrong"}):
            env = prepare.runtime_env()
            self.assertNotIn("GH_TOKEN", env)
            self.assertEqual(str(prepare.TOOLS / "dotnet"), env["DOTNET_ROOT"])

    def test_valid_report_keeps_all_losing_cases_and_all_pairwise_comparisons(self):
        with tempfile.TemporaryDirectory() as tmp:
            _, rows, samples = fixture(Path(tmp))
            self.assertEqual(rows, daily.load_cohort(tmp))
            self.assertTrue(all(len(row["pairwise"]) == 10 for row in rows))
            self.assertTrue(all(row["servers"]["rillcache"]["median"] < row["servers"]["valkey"]["median"] for row in rows))
            for invalid in (samples[:-1], samples + samples[:1], list(reversed(samples))):
                with self.assertRaises(ValueError):
                    daily.evaluate(invalid)

    def test_missing_corrupt_rehashed_and_mismatched_evidence_cannot_rank(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest, _, _ = fixture(root)
            first = manifest["order"][0]
            mutations = [
                ("manifest.json", lambda d: d.update(complete=False)),
                ("manifest.json", lambda d: d.update(runs=3)),
                ("manifest.json", lambda d: d.update(servers=list(daily.SERVERS[:-1]))),
                ("manifest.json", lambda d: d["order"].reverse()),
                ("manifest.json", lambda d: d["build"].update(source="b" * 40)),
                ("manifest.json", lambda d: d.update(client_cpus=[3])),
                (f"{first}/sample.json", lambda d: d.update(valid=False)),
                (f"{first}/sample.json", lambda d: d["measured"].update(rps=99999)),
                (f"{first}/sample.json", lambda d: d.update(server_cpu_seconds=-1)),
                (f"{first}/server.json", lambda d: d.update(joined=False)),
                (f"{first}/server.json", lambda d: d.update(actual_cpus_after=[0])),
                (f"{first}/measured/command.json", lambda d: d.update(exit_code=1)),
            ]
            for relative, mutate in mutations:
                with self.subTest(relative=relative, mutation=mutate):
                    path = root / relative
                    old = path.read_bytes()
                    data = json.loads(old)
                    mutate(data)
                    common.save(path, data)
                    changed = copy.deepcopy(manifest)
                    if relative != "manifest.json":
                        changed["evidence_sha256"][relative] = common.digest(path)
                        common.save(root / "manifest.json", changed)
                    with self.assertRaises(ValueError):
                        daily.load_cohort(root)
                    path.write_bytes(old)
                    common.save(root / "manifest.json", manifest)
            path = root / first / "measured/stdout.log"
            path.write_text("corrupt raw CSV")
            with self.assertRaises(ValueError):
                daily.load_cohort(root)

    def test_ties_variance_and_different_workload_leaders_establish_no_single_reference(self):
        with tempfile.TemporaryDirectory() as tmp:
            _, _, samples = fixture(Path(tmp))
            for s in samples:
                s["measured"] = dict(s["measured"], rps=100.)
            rows = daily.evaluate(samples)
            self.assertFalse(any(r["reference_leader_clear_of_noise_band"] for r in rows))
            self.assertIn("none established", daily.report(rows))
            for i, s in enumerate(samples):
                # Every row has a decisive but different reference leader.
                winner = "redis" if s["pipeline"] == 1 else "garnet"
                s["measured"]["rps"] = 1000. if s["variant"] == winner else 100.
            rows = daily.evaluate(samples)
            self.assertTrue(all(r["reference_leader_clear_of_noise_band"] for r in rows))
            self.assertIn("none established", daily.report(rows))
            self.assertIn("no rankings or winner", daily.report([], "missing Garnet"))
            for s in samples:
                s["measured"]["rps"] = (1, 500, 1000, 1500, 2000)[s["run"] - 1] if s["variant"] == "garnet" else (
                    900. if s["variant"] == "redis" else 100.)
            rows = daily.evaluate(samples)
            self.assertFalse(any(r["reference_leader_clear_of_noise_band"] for r in rows))

    def test_native_preparation_and_measurement_are_actions_only(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(prepare, "build") as build:
            with self.assertRaises(RuntimeError):
                prepare.prepare()
            build.assert_not_called()
            with tempfile.TemporaryDirectory() as tmp, self.assertRaises(ValueError):
                daily.run(Path(tmp) / "results")

    def test_checked_cache_and_safe_extraction_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(prepare, "ROOT", Path(tmp)), patch.object(prepare, "checked") as checked:
            cache = Path(tmp) / "target/upstream-downloads"
            cache.mkdir(parents=True)
            (cache / ("daily-test-" + "a" * 64)).write_text("corrupt")
            with self.assertRaises(ValueError):
                prepare.download({"url": "https://example.org/archive", "sha256": "a" * 64}, "test")
            checked.assert_not_called()
            archive = Path(tmp) / "bad.tar"
            with tarfile.open(archive, "w") as tar:
                info = tarfile.TarInfo("../escape")
                info.size = 1
                tar.addfile(info, io.BytesIO(b"x"))
            with self.assertRaises(tarfile.FilterError):
                prepare.extract(archive, Path(tmp) / "out")
            self.assertFalse((Path(tmp) / "escape").exists())

    def test_smoke_cannot_be_relabelled_as_complete_performance_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "smoke"
            build = {"runtime_files_sha256": {}, "binaries": {}}
            with patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}), \
                 patch.object(daily, "read_json", return_value=build), \
                 patch.object(daily, "tree_hashes", return_value={}), \
                 patch.object(daily, "environment", return_value={}), \
                 patch.object(daily.os, "sched_getaffinity", return_value={0, 1, 2, 3}), \
                 patch.object(daily, "owned_server") as server, \
                 patch.dict(daily.G, {"validate_dataset": MagicMock()}):
                server.return_value.__enter__.return_value = (None, 12345)
                daily.run(output, smoke=True)
                self.assertEqual(5, server.call_count)
            self.assertTrue(json.loads((output / "smoke.json").read_text())["not_performance_evidence"])
            self.assertFalse(json.loads((output / "manifest.json").read_text())["complete"])
            with self.assertRaises(ValueError):
                daily.load_cohort(output)

    def test_owned_server_is_joined_even_if_the_consumer_raises(self):
        with tempfile.TemporaryDirectory() as tmp, \
             patch.object(daily.subprocess, "Popen") as popen, \
             patch.object(daily, "Client") as client, \
             patch.object(daily.os, "sched_getaffinity", return_value={0, 1, 2, 3}), \
             patch.object(daily, "terminate", return_value=-15) as terminate:
            popen.return_value.poll.return_value = None
            popen.return_value.pid = 1234
            client.return_value.__enter__.return_value.call.return_value = b"PONG"
            directory = Path(tmp) / "owned"
            with self.assertRaisesRegex(ValueError, "consumer failed"):
                with daily.owned_server({"garnet": "/garnet"}, "garnet", [0, 1, 2, 3], directory):
                    raise ValueError("consumer failed")
            terminate.assert_called_once_with(popen.return_value)
            self.assertTrue(json.loads((directory / "server.json").read_text())["joined"])
            self.assertNotIn("GH_TOKEN", popen.call_args.kwargs["env"])

    def test_pins_and_dashboard_schedules_are_explicit_and_old_pr_checks_unchanged(self):
        targets = (common.ROOT / "benchmarks/targets.mk").read_text()
        for key in ("version", "sha256"):
            value = re.search(rf"^DRAGONFLY_{key.upper()} := (.+)$", targets, re.M)[1]
            self.assertEqual(value, prepare.PINS["dragonfly"][key])
        text = (common.ROOT / ".github/workflows/upstream-performance.yml").read_text()
        self.assertIn("cron: '17 5 * * *'", text)
        self.assertIn("python3 scripts/upstream/daily_prepare.py", text)
        self.assertIn("python3 scripts/upstream/daily.py --check", text)
        benchmarks = (common.ROOT / ".github/workflows/benchmarks.yml").read_text()
        self.assertIn("cron: '17 8 * * 2'", benchmarks)
        self.assertIn("benchmarks/README.md", (common.ROOT / "docs/benchmark-reports.md").read_text())
        freshness = (common.ROOT / ".github/workflows/openwiki-freshness.yml").read_text()
        self.assertIn("cron: '17 12 * * 2'", freshness)
        self.assertIn("check_openwiki_freshness.mjs", freshness)
        policy = json.loads((common.ROOT / ".github/pr-workflow-gates.json").read_text())
        self.assertEqual(14, sum(len(names) for w in policy["workflows"].values() for names in w["jobs"].values()))


if __name__ == "__main__":
    unittest.main()

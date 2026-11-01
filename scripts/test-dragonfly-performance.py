#!/usr/bin/env python3
"""Offline tests for official Dragonfly benchmark orchestration, not timings."""
import contextlib
import copy
import csv
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
HARNESS = Path(os.environ.get("RILLCACHE_GATE_HARNESS", Path(__file__).with_name("pr-dragonfly-performance.py")))
SPEC = importlib.util.spec_from_file_location("dragonfly_gate", HARNESS)
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)
HEADER = ("test", "rps", "avg_latency_ms", "min_latency_ms", "p50_latency_ms", "p95_latency_ms", "p99_latency_ms", "max_latency_ms")


def csv_record(workload, rate=101):
    text = io.StringIO()
    writer = csv.writer(text)
    writer.writerow(HEADER)
    writer.writerow([" ".join(gate.workload_command(workload)), rate, .2, .1, .2, .3, .4, .5])
    return text.getvalue()


class GateTests(unittest.TestCase):
    def samples(self):
        return [dict(workload=w, pipeline=p, run=r, variant=v, rps=101 if v == "rillcache" else 100,
                     p99_latency_ms=.4) for w, p, r, v in gate.identities()]

    def test_exact_four_cpu_budget_and_honest_shared_client_fallback(self):
        self.assertEqual(([2, 4, 6, 8], [2, 4, 6, 8]), gate.placement([2, 4, 6, 8]))
        self.assertEqual(([2, 4, 6, 8], [10, 12]), gate.placement([2, 4, 6, 8, 10, 12]))
        for cpus in ([0, 1], [0, 1, 2, True], [0, 1, 2, 2], [3, 2, 1, 0], [-1, 0, 1, 2]):
            with self.subTest(cpus=cpus), self.assertRaises(ValueError):
                gate.placement(cpus)

    def test_official_generator_options_and_conditional_commands(self):
        for w in gate.WORKLOADS:
            for p in gate.PIPELINES:
                args = gate.generator_argv("/valkey-benchmark", 1234, w, p, None, [4, 5])
                self.assertEqual(["taskset", "-c", "4,5", "/valkey-benchmark"], args[:4])
                # Its executable is the official Valkey generator; it is not
                # either compared server and never selects a Valkey server role.
                self.assertEqual(gate.workload_command(w), args[args.index("--") + 1:])
                for flag, expected in (("--duration", "10"), ("-P", str(p)), ("-c", "8"),
                                       ("-r", "1024"), ("--seed", "42"), ("--threads", "2")):
                    # taskset itself also has -c.
                    index = args.index(flag, 4)
                    self.assertEqual(expected, args[index + 1])
                self.assertNotIn("rillcache-bench", " ".join(args))
                self.assertNotIn("-n", args)
                warmup = gate.generator_argv("/valkey-benchmark", 1234, w, p, gate.WARMUP, [4, 5])
                self.assertNotIn("--duration", warmup)
                self.assertEqual("8000", warmup[warmup.index("-n") + 1])

    def test_only_rillcache_and_dragonfly_are_compared_server_roles(self):
        rillcache = gate.server_argv('/candidate', 'rillcache', 1234, [0, 1, 2, 3], Path('/run'))
        dragonfly = gate.server_argv('/dragonfly', 'dragonfly', 1234, [0, 1, 2, 3], Path('/run'))
        self.assertEqual(rillcache[3:], ['/candidate', '--bind', '127.0.0.1:1234'])
        self.assertIn('--proactor_threads', dragonfly)
        self.assertNotIn('--proactor_threads', rillcache)
        with self.assertRaisesRegex(ValueError, 'server role'):
            gate.server_argv('/valkey', 'valkey', 1234, [0, 1, 2, 3], Path('/run'))
        for valid in ('rillcache 0.1.0', 'rillcache 1.2.3-rc.1', 'rillcache 1.2.3+build', 'rillcache 1.2.3-rc.1+build'):
            gate.validate_candidate_version(valid)
        for wrong in ('Valkey server v=9.1.2', 'valkey-benchmark 9.1.2', 'dragonfly v2.0.2',
                      'rillcache', 'rillcache 0.1.0\nValkey', None, True):
            with self.subTest(version=wrong), self.assertRaisesRegex(ValueError, 'not Rillcache'):
                gate.validate_candidate_version(wrong)

    def test_mislabeled_valkey_candidate_fails_before_timed_traffic(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            argv = [str(HARNESS)]
            for role in ('candidate', 'dragonfly', 'generator'):
                binary = root / role
                binary.write_text(role)
                argv += ['--' + role, str(binary)]
            for role in ('base-source', 'head-source', 'candidate-source'):
                argv += ['--' + role, 'a' * 40]
            license_path = root / 'notice.txt'
            license_path.write_text('Dragonfly Business Source License 1.1\nDragonflyDB, Ltd.\n')
            output = root / 'evidence'
            argv += ['--dragonfly-version', '2.0.2', '--license', str(license_path), '--out', str(output)]
            with mock.patch.object(sys, 'argv', argv), \
                 mock.patch.object(os, 'sched_getaffinity', return_value={0, 1, 2, 3}), \
                 mock.patch.object(gate.subprocess, 'check_output', side_effect=lambda argv, **_: (
                     b'Valkey server v=9.1.2\n' if Path(argv[0]).name == 'candidate'
                     else b'Dragonfly v2.0.2\n')), \
                 mock.patch.object(gate.subprocess, 'Popen') as popen, \
                 mock.patch.object(gate, 'sample', side_effect=AssertionError(
                     'timed traffic reached for a mislabeled Valkey candidate')) as sample:
                with self.assertRaisesRegex(ValueError, 'not Rillcache'):
                    gate.main()
                popen.assert_not_called()
                sample.assert_not_called()
            manifest = gate.read_json(output / 'manifest.json')
            self.assertFalse(manifest['complete'])
            self.assertFalse(manifest['gate_passed'])
            self.assertEqual(manifest['candidate_binary_version'], 'Valkey server v=9.1.2')

    def test_workflow_builds_one_rillcache_for_both_comparisons_and_runs_after_valkey_loss(self):
        root = Path(__file__).resolve().parents[1]
        text = (root / '.github/workflows/pr-multicpu-quality.yml').read_text()
        self.assertIn('build --locked --offline --release --bin rillcache', text)
        self.assertEqual(text.count('--candidate "$RUNNER_TEMP/valkey-multicpu-tools/candidate"'), 2)
        step = text.split('      - name: Measure Rillcache against Dragonfly in all ten conditions\n', 1)[1].split('      - name:', 1)[0]
        self.assertIn("!cancelled() && steps.candidate.outcome == 'success'", step)
        self.assertIn("steps.dragonfly.outcome == 'success'", step)
        self.assertIn('--generator "$RUNNER_TEMP/valkey-multicpu-tools/valkey-benchmark"', step)
        self.assertIn('--dragonfly "$RUNNER_TEMP/dragonfly-tools/dragonfly"', step)
        self.assertIn('sha256sum --check --status', step)
        self.assertNotIn('--valkey ', step)
        self.assertNotIn('continue-on-error:', text)

    def test_runtime_children_do_not_receive_workflow_credentials(self):
        proc = mock.Mock(pid=1234)
        proc.poll.return_value = None
        client = mock.MagicMock()
        client.__enter__.return_value.call.return_value = b'PONG'
        with tempfile.TemporaryDirectory() as name, \
             mock.patch.dict(os.environ, {'GH_TOKEN': 'must-not-be-forwarded'}), \
             mock.patch.object(gate.subprocess, 'Popen', return_value=proc) as popen, \
             mock.patch.object(gate, 'Client', return_value=client), \
             mock.patch.object(os, 'sched_getaffinity', return_value={0, 1, 2, 3}), \
             mock.patch.object(gate, 'terminate', return_value=0):
            with gate.owned_server('/candidate', 'rillcache', [0, 1, 2, 3], Path(name)):
                pass
        self.assertEqual(set(popen.call_args.kwargs['env']), {'PATH', 'LANG', 'LC_ALL'})
        self.assertEqual(popen.call_args.kwargs['env']['LC_ALL'], 'C')

    def test_ten_strict_medians_no_single_loss_tie_or_best_sample_escape(self):
        rows = gate.evaluate(self.samples())
        self.assertEqual(10, len(rows))
        self.assertTrue(all(r["passed"] for r in rows))
        self.assertIn("PASS: 10/10", gate.summary(rows).replace("**", ""))
        for rate in (100, 99):
            samples = self.samples()
            for s in samples:
                if (s["workload"], s["pipeline"], s["variant"]) == ("ping", 1, "rillcache"):
                    s["rps"] = 100000 if s["run"] == 1 else rate
            rows = gate.evaluate(samples)
            self.assertEqual(9, sum(r["passed"] for r in rows))

    def test_incomplete_duplicate_nonfinite_and_boolean_samples_fail(self):
        for mutate in (lambda s: s.pop(), lambda s: s.__setitem__(-1, s[0]),
                       lambda s: s[0].update(run=4)):
            samples = self.samples()
            mutate(samples)
            with self.assertRaises(ValueError): gate.evaluate(samples)
        for value in (True, 0, -1, float("nan"), float("inf")):
            samples = self.samples()
            samples[0]["rps"] = value
            with self.assertRaises(ValueError): gate.evaluate(samples)

    def generator_fixture(self, directory, workload="set-nx-existing", rate=101, **changes):
        directory.mkdir(parents=True, exist_ok=True)
        argv = gate.generator_argv("/official", 1234, workload, 16, None, [0, 1, 2, 3])
        result = dict(argv=argv, exit_code=0, wall_seconds=10.1, **changes)
        gate.save(directory / "command.json", result)
        (directory / "stdout.log").write_text(csv_record(workload, rate))
        (directory / "stderr.log").write_text("")
        return argv

    def test_official_csv_exact_command_errors_timeout_warnings_and_metrics(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            args = self.generator_fixture(root)
            self.assertEqual(101, gate.read_generator(root, "set-nx-existing", args)["rps"])
            (root / "stderr.log").write_text("WARNING: Could not fetch server CONFIG\n")
            gate.read_generator(root, "set-nx-existing", args)
            for stderr in ("Error from server: ERR\n", "unknown warning\n"):
                (root / "stderr.log").write_text(stderr)
                with self.assertRaises(ValueError): gate.read_generator(root, "set-nx-existing", args)
            for text in (csv_record("set"), csv_record("set-nx-existing", "nan"),
                         csv_record("set-nx-existing") * 2, "", csv_record("set-nx-existing").replace("0.4", "0.01"),
                         csv_record("set-nx-existing").replace("p95_latency_ms", "p50_latency_ms")):
                self.generator_fixture(root)
                (root / "stdout.log").write_text(text)
                with self.assertRaises(ValueError): gate.read_generator(root, "set-nx-existing", args)
            for changes in ({"exit_code": 1}, {"exit_code": True}, {"timed_out": True}, {"argv": ["false"]}):
                self.generator_fixture(root)
                result = gate.read_json(root / "command.json")
                result.update(changes)
                gate.save(root / "command.json", result)
                with self.assertRaises(ValueError): gate.read_generator(root, "set-nx-existing", args)

    def test_short_old_quantized_samples_and_invalid_durations_cannot_pass(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            for elapsed in (None, True, -.1, .751, 1, 9.999, float("inf"), 181):
                args = self.generator_fixture(root)
                result = gate.read_json(root / "command.json")
                result["wall_seconds"] = elapsed
                if elapsed == float("inf"):
                    # Also reject syntactically valid JSON float overflow.
                    (root / "command.json").write_text('{"wall_seconds": 1e999}')
                else:
                    gate.save(root / "command.json", result)
                with self.subTest(elapsed=elapsed), self.assertRaises((ValueError, KeyError)):
                    gate.read_generator(root, "set-nx-existing", args)
            self.generator_fixture(root)
            self.assertEqual(101, gate.read_generator(root, "set-nx-existing", args)["rps"])

    def test_dataset_checks_all_hit_keys_and_nx_rejection(self):
        client = mock.MagicMock()
        client.__enter__.return_value = client
        client.call.side_effect = [b"PONG", None, gate.VALUE]
        client.batch.return_value = [gate.VALUE] * 128
        with mock.patch.object(gate, "Client", return_value=client):
            self.assertEqual(1024, gate.validate_dataset(1234, "set-nx-existing")["validated_keys"])
        self.assertEqual(8, client.batch.call_count)
        for bad in (None, b"corrupt", gate.REPLACEMENT):
            client.call.side_effect = [b"PONG"]
            client.batch.return_value = [bad] * 128
            with mock.patch.object(gate, "Client", return_value=client), self.assertRaises(ValueError):
                gate.validate_dataset(1234, "set-nx-existing", after=True)
        client.call.side_effect = [b"PONG", b"OK"]
        client.batch.return_value = [gate.VALUE] * 128
        with mock.patch.object(gate, "Client", return_value=client), self.assertRaisesRegex(ValueError, "NX existing"):
            gate.validate_dataset(1234, "set-nx-existing")

    def test_owned_server_is_reaped_on_readiness_affinity_and_load_failure(self):
        for failure in ("readiness", "affinity", "load"):
            with tempfile.TemporaryDirectory() as name:
                proc = mock.Mock(pid=1234)
                proc.poll.return_value = 1 if failure == "readiness" else None
                client = mock.MagicMock()
                client.__enter__.return_value.call.return_value = b"PONG"
                with mock.patch.object(gate.subprocess, "Popen", return_value=proc), \
                     mock.patch.object(gate, "Client", return_value=client), \
                     mock.patch.object(os, "sched_getaffinity", return_value={0} if failure == "affinity" else {0, 1, 2, 3}), \
                     mock.patch.object(gate, "terminate", return_value=0) as terminate:
                    with self.assertRaises(ValueError):
                        with gate.owned_server("/server", "rillcache", [0, 1, 2, 3], Path(name)):
                            raise ValueError("load failed")
                    terminate.assert_called_once_with(proc)
                self.assertTrue(gate.read_json(Path(name) / "server.json")["joined"])

    def cohort_fixture(self, root):
        cpus = [0, 1, 2, 3]
        binaries = {v: "/" + v for v in ("rillcache", "dragonfly", "generator")}
        manifest = dict(schema_version=gate.SCHEMA_VERSION, complete=True, gate_passed=True, allowed_cpus=cpus,
                        server_cpus=cpus, client_cpus=cpus, runs=3, workloads=gate.WORKLOADS,
                        pipelines=gate.PIPELINES, duration_seconds=10, warmup_requests=8000,
                        launch_directory="/original/runner/gate", binaries=binaries,
                        binary_sha256={v: char * 64 for v, char in zip(binaries, "abc")},
                        candidate_binary_version="rillcache 0.1.0", base_source="a" * 40,
                        head_source="b" * 40, candidate_source="c" * 40, order=[], evidence_sha256={})
        for identity in gate.identities():
            w, p, r, v = identity
            name = gate.name_for(*identity)
            directory = root / name
            directory.mkdir()
            server = dict(argv=gate.server_argv(binaries[v], v, 1234, cpus, Path(manifest["launch_directory"]) / name),
                          cpus=cpus, actual_cpus_before=cpus, actual_cpus_after=cpus,
                          pid=47, port=1234, joined=True, exit_before_cleanup=None)
            gate.save(directory / "server.json", server)
            (directory / "server.log").write_text("server log\n")
            validation = {"validated_keys": 0 if w == "ping" else gate.KEYSPACE, "ping": True}
            sample = dict(valid=True, pid=47, port=1234, preload=validation)
            for phase, count in (("warmup", gate.WARMUP), ("measured", None)):
                d = directory / phase
                args = gate.generator_argv(binaries["generator"], 1234, w, p, count, cpus)
                d.mkdir()
                gate.save(d / "command.json", dict(argv=args, exit_code=0, wall_seconds=10.1 if count is None else .1))
                (d / "stdout.log").write_text(csv_record(w, 101 if v == "rillcache" else 100))
                (d / "stderr.log").write_text("")
                sample[phase] = gate.read_generator(d, w, args)
                sample[phase + "_validation"] = validation
            gate.save(directory / "sample.json", sample)
            manifest["order"].append(name)
            manifest["evidence_sha256"].update({f"{name}/{f}": gate.digest(directory / f) for f in gate.RAW_FILES})
        license_path = root / "license.txt"
        license_path.write_text("Dragonfly Business Source License 1.1\nDragonflyDB, Ltd.\n")
        manifest["license_path"] = "license.txt"
        manifest["license_sha256"] = gate.digest(license_path)
        manifest["dragonfly_version"] = "2.0.2"
        manifest["dragonfly_binary_version"] = "dragonfly v2.0.2-test"
        gate.save(root / "manifest.json", manifest)
        (root / "summary.md").write_text(gate.summary(gate.evaluate(self.samples())))
        return manifest

    def test_relocated_cohort_replays_and_corruptions_fail_closed(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest = self.cohort_fixture(root)
            self.assertTrue(all(r["passed"] for r in gate.load_cohort(root)))
            for key, value in (("complete", False), ("gate_passed", False), ("server_cpus", [0]),
                               ("runs", True), ("head_source", "invalid"), ("order", []), ("evidence_sha256", {}),
                               ("schema_version", 0), ("schema_version", 1), ("duration_seconds", 1), ("warmup_requests", 1),
                               ("candidate_binary_version", "Valkey server v=9.1.2"),
                               ("candidate_binary_version", None),
                               ("binary_sha256", {v: "a" * 64 for v in manifest["binaries"]}),
                               ("dragonfly_version", "2.0"), ("license_sha256", "0" * 64),
                               ("license_path", "/tmp/license.txt")):
                bad = copy.deepcopy(manifest)
                bad[key] = value
                gate.save(root / "manifest.json", bad)
                with self.subTest(key=key), self.assertRaises(ValueError): gate.load_cohort(root)
            gate.save(root / "manifest.json", manifest)
            first = root / manifest["order"][0] / "measured/stdout.log"
            original_csv = first.read_bytes()
            first.write_bytes(original_csv + b" ")
            with self.assertRaisesRegex(ValueError, "checksum"): gate.load_cohort(root)
            first.write_bytes(original_csv)
            (root / "summary.md").write_text("fabricated pass")
            with self.assertRaisesRegex(ValueError, "summary"): gate.load_cohort(root)

    def test_check_cli_passes_complete_wins_but_retains_complete_losses_and_fails(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest = self.cohort_fixture(root)
            def check():
                return subprocess.run([sys.executable, str(HARNESS), "--check", str(root)],
                                      capture_output=True, text=True, timeout=15)
            self.assertEqual(0, check().returncode)
            samples = self.samples()
            for item in samples:
                if (item["workload"], item["pipeline"], item["variant"]) == ("ping", 1, "rillcache"):
                    item["rps"] = 99
                    d = root / gate.name_for(item["workload"], item["pipeline"], item["run"], item["variant"])
                    record = gate.read_json(d / "sample.json")
                    for phase in ("warmup", "measured"):
                        (d / phase / "stdout.log").write_text(csv_record("ping", 99))
                        record[phase]["rps"] = 99.0
                    gate.save(d / "sample.json", record)
                    for file in gate.RAW_FILES:
                        manifest["evidence_sha256"][f"{d.name}/{file}"] = gate.digest(d / file)
            manifest["gate_passed"] = False
            gate.save(root / "manifest.json", manifest)
            (root / "summary.md").write_text(gate.summary(gate.evaluate(samples)))
            result = check()
            self.assertEqual(1, result.returncode, result.stderr)
            self.assertIn("FAIL: 9/10", result.stdout.replace("**", ""))
            self.assertTrue(gate.read_json(root / "manifest.json")["complete"])
            (root / "manifest.json").write_text('{"complete": false}')
            self.assertNotEqual(0, check().returncode)

    def test_fewer_than_four_cpus_preserves_incomplete_manifest_without_spawning(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name) / "gate"
            license_path = Path(name) / "license.txt"
            license_path.write_text("Dragonfly Business Source License 1.1\nDragonflyDB, Ltd.\n")
            args = [str(HARNESS)]
            for k in ("candidate", "dragonfly", "generator"):
                args += ["--" + k, "/not-used"]
            for k in ("base-source", "head-source", "candidate-source"):
                args += ["--" + k, "a" * 40]
            args += ["--dragonfly-version", "2.0.2", "--license", str(license_path), "--out", str(root)]
            with mock.patch.object(sys, "argv", args), mock.patch.object(os, "sched_getaffinity", return_value={0, 1}), \
                 mock.patch.object(gate.subprocess, "Popen") as popen:
                with self.assertRaisesRegex(ValueError, "four"):
                    gate.main()
                popen.assert_not_called()
            evidence = gate.read_json(root / "manifest.json")
            self.assertFalse(evidence["complete"])
            self.assertFalse(evidence["gate_passed"])
            with mock.patch.object(sys, "argv", args), self.assertRaisesRegex(ValueError, "already exists"):
                gate.main()

    def test_generator_failure_preserves_invalid_sample_and_cannot_pass(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name) / "sample"
            proc = mock.Mock(pid=47)
            @contextlib.contextmanager
            def server(*_args):
                yield proc, 1234
            def failed_generator(argv, directory, **_kwargs):
                directory.mkdir()
                gate.save(directory / "command.json", dict(argv=argv, exit_code=1))
            with mock.patch.object(gate, "owned_server", server), \
                 mock.patch.object(gate, "validate_dataset", return_value={}), \
                 mock.patch.object(gate, "process_stats", return_value={}), \
                 mock.patch.object(gate, "command", side_effect=failed_generator):
                with self.assertRaisesRegex(ValueError, "load generator failed"):
                    gate.sample({"rillcache": "/server", "generator": "/official"}, "rillcache", "ping", 1,
                                [0, 1, 2, 3], [0, 1, 2, 3], root)
            evidence = gate.read_json(root / "sample.json")
            self.assertFalse(evidence["valid"])
            self.assertNotIn("measured", evidence)

    def test_timeout_cleanup_reaps_owned_process_group(self):
        # Reuse the existing real-process timeout test helper, not a server or benchmark.
        with tempfile.TemporaryDirectory() as name:
            result = gate.command([sys.executable, "-c", "import time; time.sleep(20)"], Path(name), timeout=.03)
            self.assertTrue(result["timed_out"])
            self.assertNotEqual(0, result["exit_code"])


    def test_dragonfly_license_evidence_is_retained_and_corruption_fails(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest = self.cohort_fixture(root)
            self.assertTrue(all(r["passed"] for r in gate.load_cohort(root)))
            license_path = root / "license.txt"
            license_path.write_text("not a license\n")
            with self.assertRaisesRegex(ValueError, "license evidence differs"):
                gate.load_cohort(root)
            manifest["license_sha256"] = gate.digest(license_path)
            gate.save(root / "manifest.json", manifest)
            with self.assertRaisesRegex(ValueError, "license evidence differs"):
                gate.load_cohort(root)


if __name__ == "__main__":
    unittest.main()

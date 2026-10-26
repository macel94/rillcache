"""Offline lightweight harness tests: no native builds or benchmark execution."""

import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import common
import compatibility as compat
import performance as perf
from prepare import validate_pins


class ScopeTests(unittest.TestCase):
    def test_existing_rust_and_competitor_pins_agree(self):
        validate_pins()

    def test_workflow_security_and_verified_actions(self):
        for name in ("upstream-compatibility", "upstream-performance"):
            text = (common.ROOT / f".github/workflows/{name}.yml").read_text()
            self.assertIn("runs-on: ubuntu-latest", text)
            self.assertNotIn("ubuntu-24.04", text)
            self.assertIn("contents: read", text)
            self.assertNotIn("write", text)
            self.assertNotIn("pull_request_target", text)
            self.assertNotIn("continue-on-error", text)
            self.assertIn("if: always()", text)
            self.assertIn("persist-credentials: false", text)
            for line in text.splitlines():
                if "uses: " not in line:
                    continue
                action, sha = line.split("uses: ")[1].split()[0].split("@")
                self.assertEqual(common.PINS["actions"][action]["sha"], sha)
        text = (common.ROOT / ".github/workflows/upstream-performance.yml").read_text()
        self.assertIn("  workflow_dispatch:", text)
        self.assertNotIn("  pull_request:", text)
        self.assertNotIn("matrix:", text)

    def test_manifest_selection_matches_matrix(self):
        text = (common.ROOT / ".github/workflows/upstream-compatibility.yml").read_text()
        self.assertIn("fail-fast: false", text)
        for suite in common.SCOPE["compatibility"]["suites"]:
            self.assertIn(f"path: {suite}}}", text)
        self.assertGreaterEqual(perf.CONFIG["repetitions"], 5)
        self.assertEqual(0, perf.CONFIG["keyspace"] % 128)
        for pipeline in perf.CONFIG["pipelines"]:
            self.assertEqual(0, perf.CONFIG["requests"] % pipeline)
            self.assertEqual(0, perf.CONFIG["warmup_requests"] % pipeline)


class ProcessTests(unittest.TestCase):
    def test_preserves_true_child_status_and_logs(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            result = common.command([sys.executable, "-c", "print('original log'); raise SystemExit(7)"], directory)
            self.assertEqual(7, result["exit_code"])
            self.assertEqual(7, json.loads((directory / "command.json").read_text())["exit_code"])
            self.assertIn("original log", (directory / "stdout.log").read_text())

    def test_timeout_is_not_a_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = common.command([sys.executable, "-c", "import time; time.sleep(30)"], Path(tmp), timeout=0.05)
            self.assertTrue(result["timed_out"])
            self.assertNotEqual(0, result["exit_code"])


class RespTests(unittest.TestCase):
    def test_binary_command(self):
        self.assertEqual(b"*2\r\n$4\r\nECHO\r\n$3\r\na\x00b\r\n", common.resp_command("ECHO", b"a\0b"))

    def test_reply_types(self):
        for wire, expected in ((b"+PONG\r\n", b"PONG"), (b":4\r\n", 4), (b"$-1\r\n", None),
                               (b"$3\r\na\0b\r\n", b"a\0b"), (b"*2\r\n:1\r\n+OK\r\n", [1, b"OK"])):
            self.assertEqual(expected, common.read_reply(io.BytesIO(wire)))

    def test_invalid_reply_never_becomes_success(self):
        for wire in (b"", b"+no", b"$999999999\r\n", b"$-5\r\n", b"$3\r\na", b"*999999\r\n", b"?wat\r\n"):
            with self.subTest(wire=wire), self.assertRaises((ValueError, ConnectionError)):
                common.read_reply(io.BytesIO(wire))
        with self.assertRaisesRegex(RuntimeError, "unknown command"):
            common.read_reply(io.BytesIO(b"-ERR unknown command\r\n"))


class CompatibilityTests(unittest.TestCase):
    def item(self, log="[ok]: Test (12 ms)\n[1/1 done]: unit/type/incr\n", **kwargs):
        return {"events": compat.events(log), "exit_code": 0, **kwargs}

    def test_success_requires_executed_tests_and_completed_unit(self):
        self.assertEqual("passed_selected_scope", compat.classify(self.item()))
        for log in ("All tests passed without errors!", "[ignore]: Not supported on external server"):
            self.assertEqual("zero_tests_or_harness_error", compat.classify(self.item(log)))

    def test_setup_failure_separate_from_test_failure(self):
        result = self.item('[exception]: ERR unknown command flushall\n', exit_code=1,
                           missing_prerequisites=["FLUSHALL"])
        self.assertEqual("prerequisite_blocked", compat.classify(result))
        result = self.item('[err]: Some assertion in tests/unit/type/incr.tcl\n[1/1 done]: unit/type/incr\n')
        self.assertEqual("test_failures", compat.classify(result))

    def test_timeout_crash_and_exception_cannot_pass(self):
        self.assertEqual("timeout", compat.classify(self.item(timed_out=True)))
        self.assertEqual("server_exit", compat.classify(self.item(server_alive=False)))
        self.assertEqual("harness_error_or_unreached_tests", compat.classify(self.item("[ok]: A (1 ms)\n[exception]: setup failed\n")))
        self.assertEqual("prerequisite_or_harness_error", compat.classify(self.item(exit_code=1)))

    def test_ansi_duplicates_and_skip_events(self):
        data = compat.events("[\x1b[32mok\x1b[0m]: A (3 ms)\n[err]: B in tests/unit/x.tcl\n*** [err]: B in tests/unit/x.tcl\n[skip]: C\n[ignore]: Tag: slow denied\n")
        self.assertEqual(["A"], data["ok"])
        self.assertEqual(["B"], data["err"])
        self.assertEqual(["C"], data["skip"])
        self.assertEqual(1, len(data["ignore"]))
        self.assertFalse(data["unit_completed"])

    def test_inventory_lists_unselected_modules_and_protocol(self):
        with tempfile.TemporaryDirectory() as tmp:
            tree = Path(tmp)
            for unit in ("unit/type/string", "unit/moduleapi/a", "unit/protocol", "integration/repl"):
                file = tree / f"tests/{unit}.tcl"
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("# original")
            inventory = compat.inventory(tree, "unit/type/string")
            self.assertEqual(4, len(inventory))
            self.assertEqual(1, sum(item["selected_in_this_job"] for item in inventory))
            self.assertTrue(all(item["reason"] for item in inventory))


class PerformanceTests(unittest.TestCase):
    HEADER = '"test","rps","avg_latency_ms","min_latency_ms","p50_latency_ms","p95_latency_ms","p99_latency_ms","max_latency_ms"\n'
    ROW = '"GET data:__rand_int__","100000","0.1","0.01","0.1","0.2","0.3","1.0"\n'

    def test_csv_valid_and_rejects_empty_multiple_wrong_test_nonfinite(self):
        self.assertEqual(100000, perf.parse_csv(self.HEADER + self.ROW, "get-hit")["rps"])
        for text in (self.HEADER, self.HEADER + self.ROW * 2,
                     (self.HEADER + self.ROW).replace('GET data:__rand_int__', ''),
                     (self.HEADER + self.ROW).replace('"p95_latency_ms"', '"wrong_field"'),
                     (self.HEADER + self.ROW).replace("GET data", "SET data"),
                     (self.HEADER + self.ROW).replace("100000", "nan"),
                     (self.HEADER + self.ROW).replace("100000", "0"),
                     (self.HEADER + self.ROW).replace('"0.3"', '"0.02"')):
            with self.subTest(text=text), self.assertRaises(ValueError):
                perf.parse_csv(text, "get-hit")

    def test_generator_options_identical_except_endpoint(self):
        args = perf.generator_argv(1234, "get-hit", 16, 300000, 1)
        self.assertIn("--csv", args)
        self.assertIn("data:__rand_int__", args)
        self.assertEqual(str(perf.CONFIG["seed"]), args[args.index("--seed") + 1])
        self.assertEqual("1", args[args.index("--threads") + 1])

    def test_incomplete_cohort_cannot_be_valid_or_zero_errors(self):
        result = perf.aggregate([])
        self.assertEqual(30, len(result))
        self.assertTrue(all(not r["matched_cohort_valid"] for r in result))
        self.assertTrue(all(r["workload_errors"] is None for r in result))

    def test_complete_and_duplicate_cohorts(self):
        samples = []
        for name in perf.SERVERS:
            for repetition in range(perf.CONFIG["repetitions"]):
                item = {"workload": "ping", "pipeline": 1, "server": name, "repetition": repetition,
                        "status": "valid", "rps": 100, "resources": {"user_s": 1, "system_s": 1, "peak_rss_kib": 1024}}
                for field in ("avg_latency_ms", "p50_latency_ms", "p95_latency_ms", "p99_latency_ms", "server_cpu_seconds",
                              "server_cpu_percent_one_core", "server_rss_kib", "server_peak_rss_kib"):
                    item[field] = 1
                samples.append(item)
        rows = [r for r in perf.aggregate(samples) if r["workload"] == "ping" and r["pipeline"] == 1]
        self.assertTrue(all(r["matched_cohort_valid"] for r in rows))
        samples[-1] = samples[0]
        rows = [r for r in perf.aggregate(samples) if r["workload"] == "ping" and r["pipeline"] == 1]
        self.assertTrue(all(not r["matched_cohort_valid"] for r in rows))
        self.assertTrue(all(r["workload_errors"] is None for r in rows))

    def test_hit_dataset_never_accepts_missing_values(self):
        client = unittest.mock.MagicMock()
        client.__enter__.return_value = client
        client.call.return_value = b"PONG"
        client.batch.return_value = [None] * 128
        with patch.object(perf, "Client", return_value=client), self.assertRaisesRegex(ValueError, "hit dataset"):
            perf.dataset(1, "get-hit")
        client.batch.return_value = [perf.VALUE] * 128
        with patch.object(perf, "Client", return_value=client):
            self.assertEqual(perf.CONFIG["keyspace"], perf.dataset(1, "get-hit")["keys"])

    def test_incr_and_miss_validation(self):
        client = unittest.mock.MagicMock()
        client.__enter__.return_value = client
        client.call.return_value = b"PONG"
        client.batch.return_value = [None] * 128
        with patch.object(perf, "Client", return_value=client):
            self.assertTrue(perf.dataset(1, "get-miss")["validated"])
            with self.assertRaisesRegex(ValueError, "aggregate"):
                perf.dataset(1, "incr", total=3)
        client.batch.return_value = [b"1"] * 128
        with patch.object(perf, "Client", return_value=client):
            self.assertTrue(perf.dataset(1, "incr", total=perf.CONFIG["keyspace"])["validated"])
            with self.assertRaisesRegex(ValueError, "aggregate"):
                perf.dataset(1, "incr", total=3)

    def test_no_local_expensive_execution(self):
        with patch.dict("os.environ", {"GITHUB_ACTIONS": "false"}):
            import prepare
            with self.assertRaisesRegex(RuntimeError, "Expensive builds"):
                prepare.build()


if __name__ == "__main__":
    unittest.main()

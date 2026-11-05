#!/usr/bin/env python3
"""Offline tests for the partial-cohort merger; no timed traffic."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("merge_gates", ROOT / "scripts/merge-multicpu-gates.py")
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


def fake_harness():
    class Harness:
        WORKLOADS = ("ping", "set", "get", "set-nx-existing", "set-xx-existing")
        PIPELINES = (1, 16)
        KEYSPACE = 1024

        @staticmethod
        def summary(rows):
            wins = sum(row["passed"] for row in rows)
            return f"# synthetic merged gate\n\n**{wins}/{len(rows)}**\n"

        @staticmethod
        def load_cohort(root):
            manifest = M.read_json(Path(root) / "manifest.json")
            rows = []
            for workload in manifest["workloads"]:
                for pipeline in Harness.PIPELINES:
                    rows.append({"workload": workload, "pipeline": pipeline, "passed": True,
                                 "rillcache": {"median": 2.0}, "valkey": {"median": 1.0}})
            return rows

    return Harness


def partial(root, workloads, rate=1.0, **overrides):
    root.mkdir(parents=True)
    manifest = {
        "schema_version": 3, "complete": True, "gate_passed": True,
        "base_source": "a" * 40, "head_source": "b" * 40, "candidate_source": "c" * 40,
        "valkey_version": "valkey-server v=9.1.2", "runs": 5, "pipelines": [1, 16],
        "duration_seconds": 10, "warmup_requests": 8000,
        "binary_sha256": {"rillcache": "1" * 64, "valkey": "2" * 64, "generator": "3" * 64},
        "workloads": list(workloads), "hardware_fingerprint": "host-" + workloads[0],
    }
    manifest.update(overrides)
    M.save(root / "manifest.json", manifest)


class MergeTests(unittest.TestCase):
    def setUp(self):
        self.harness = fake_harness()

    def test_complete_disjoint_partials_merge_into_one_gate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dirs = []
            for index, workload in enumerate(self.harness.WORKLOADS):
                directory = root / f"p{index}"
                partial(directory, [workload])
                dirs.append(str(directory))
            out = root / "merged"
            record, rows = M.merge(self.harness, dirs, out)
            self.assertEqual(10, len(rows))
            self.assertTrue(record["gate_passed"])
            self.assertTrue((out / "summary.md").read_text().startswith("# synthetic merged gate"))
            self.assertEqual((record, rows), M.check(self.harness, dirs, out))

    def test_missing_duplicate_or_inconsistent_partials_fail_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dirs = []
            for index, workload in enumerate(self.harness.WORKLOADS):
                directory = root / f"p{index}"
                partial(directory, [workload])
                dirs.append(str(directory))
            cases = {
                "missing workload": dirs[:-1],
                "duplicate workload": dirs + [dirs[0]],
            }
            for name, partials in cases.items():
                with self.subTest(name), self.assertRaises(ValueError):
                    M.merge(self.harness, partials, root / ("out-" + name.replace(" ", "-")))
            for field, value in (("candidate_source", "d" * 40), ("binary_sha256", {"x": "0" * 64}),
                                 ("valkey_version", "v=9.9.9"), ("runs", 3)):
                partial(root / f"bad-{field}", ["get"], **{field: value})
                with self.subTest(field), self.assertRaises(ValueError):
                    M.merge(self.harness, [*dirs[1:], str(root / f"bad-{field}")],
                            root / ("out-" + field))

    def test_a_losing_row_or_rehashed_summary_cannot_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dirs = []
            for index, workload in enumerate(self.harness.WORKLOADS):
                directory = root / f"p{index}"
                partial(directory, [workload])
                dirs.append(str(directory))

            original = self.harness.load_cohort

            def losing(directory):
                rows = original(directory)
                for row in rows:
                    if row["workload"] == "ping":
                        row["passed"] = False
                return rows

            with mock.patch.object(self.harness, "load_cohort", side_effect=losing):
                record, _ = M.merge(self.harness, dirs, root / "merged-loss")
            self.assertFalse(record["gate_passed"])
            (root / "merged-loss" / "summary.md").write_text("fabricated pass")
            with self.assertRaises(ValueError):
                M.check(self.harness, dirs, root / "merged-loss")

    def test_merging_requires_actions_context(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, {}, clear=True), \
             mock.patch.object(M, "load_harness", return_value=self.harness):
            argv = ["--harness", "/tmp/x", "--out", str(Path(temporary) / "out"),
                    "--partial", str(Path(temporary) / "p")]
            with self.assertRaises(ValueError):
                M.main(argv)


if __name__ == "__main__":
    unittest.main()

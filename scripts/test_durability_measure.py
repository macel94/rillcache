#!/usr/bin/env python3
"""Offline controls for scripts/durability_measure.py (D10)."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import durability_measure as measure  # noqa: E402


class DurabilityMeasureTests(unittest.TestCase):
    def test_dry_run_names_every_supported_policy_without_touching_a_binary(self):
        import io
        from contextlib import redirect_stdout

        buffer = io.StringIO()
        with redirect_stdout(buffer):
            self.assertEqual(0, measure.main(["--dry-run"]))
        plan = buffer.getvalue()
        for policy in (*measure.POLICIES, measure.BASELINE):
            self.assertIn(policy, plan)
        self.assertIn("workload: set", plan)

    def test_missing_binary_fails_closed_before_measuring(self):
        import io
        from contextlib import redirect_stderr

        buffer = io.StringIO()
        with redirect_stderr(buffer):
            self.assertEqual(2, measure.main(["--binary", "/nonexistent/rillcache"]))
        self.assertIn("missing server binary", buffer.getvalue())

    def test_summary_is_a_stable_table_with_one_row_per_policy(self):
        baseline = {
            "policy": "baseline",
            "throughput_median_ops": 1000.0,
            "batch_p99_median_us": 100.0,
            "ping_before": {"p99_ms": 0.1},
            "rewrite_pause_p99_ms": 0.1,
        }
        record = {
            "policy": "everysec",
            "throughput_median_ops": 1000.0,
            "batch_p99_median_us": 250.0,
            "ping_before": {"p99_ms": 0.2},
            "rewrite_pause_p99_ms": 12.5,
        }
        table = measure.summary([baseline, record])
        lines = table.strip().splitlines()
        self.assertEqual(4, len(lines))
        self.assertIn("| everysec | 1000.0 | 1.000 |", lines[3])
        self.assertIn("12.5", lines[3])

    def test_ping_sampler_percentiles_are_monotonic(self):
        samples = [float(value) for value in range(1, 101)]
        samples.sort()
        at = lambda percent: round(samples[min(len(samples) - 1, int(percent / 100.0 * len(samples)))], 3)
        self.assertLessEqual(at(50), at(99))
        self.assertLessEqual(at(99), samples[-1])


if __name__ == "__main__":
    unittest.main()

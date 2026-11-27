#!/usr/bin/env python3
"""Latency-unit contract for the advisory performance report (readiness P3)."""

import unittest

from performance import latency_unit, per_request_tails


class LatencyUnitTest(unittest.TestCase):
    def test_pipeline_one_is_per_request(self):
        self.assertEqual(latency_unit(1), "per-request")

    def test_batch_pipelines_are_per_batch(self):
        for pipeline in (2, 16, 64):
            self.assertEqual(latency_unit(pipeline), "per-batch")

    def test_tails_come_only_from_pipeline_one(self):
        rows = [
            {"workload": "get-hit", "pipeline": 16, "server": "rillcache", "p99_latency_ms": 9.0},
            {"workload": "get-hit", "pipeline": 1, "server": "rillcache", "p99_latency_ms": 0.25},
            {"workload": "get-hit", "pipeline": 1, "server": "redis", "p99_latency_ms": 0.30},
            {"workload": "set", "pipeline": 16, "server": "redis", "p99_latency_ms": 4.0},
        ]
        tails = per_request_tails(rows)
        self.assertEqual(tails["get-hit"], 0.25)
        self.assertNotIn("set", tails)

    def test_missing_tail_is_skipped_not_zero(self):
        rows = [{"workload": "ping", "pipeline": 1, "server": "rillcache", "rps_median": 1.0}]
        self.assertEqual(per_request_tails(rows), {})


if __name__ == "__main__":
    unittest.main()

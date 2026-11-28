#!/usr/bin/env python3
"""Offline controls for scripts/soak.py (O9)."""

from __future__ import annotations

import io
import sys
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import soak  # noqa: E402


class SoakTests(unittest.TestCase):
    def test_dry_run_describes_the_plan_without_a_binary(self):
        buffer = io.StringIO()
        with redirect_stdout(buffer):
            self.assertEqual(0, soak.main(["--dry-run", "--duration", "10"]))
        plan = buffer.getvalue()
        self.assertIn("duration=10s", plan)
        self.assertIn("clients=", plan)
        self.assertIn("sleep_ms=", plan)

    def test_missing_binary_fails_closed(self):
        buffer = io.StringIO()
        with redirect_stderr(buffer):
            self.assertEqual(2, soak.main(["--binary", "/nonexistent/rillcache"]))
        self.assertIn("missing server binary", buffer.getvalue())

    def test_summary_reports_growth_recovery_and_errors(self):
        report = {
            "duration_seconds": 60,
            "clients": 4,
            "samples": [
                {"rss_kib": 1000, "journal_bytes": 100, "free_disk_bytes": 5000,
                 "journal_files": ["appendonly.aof"]},
                {"rss_kib": 4000, "journal_bytes": 900, "free_disk_bytes": 3000,
                 "journal_files": ["appendonly.aof", "appendonly.aof.1"]},
                {"rss_kib": 2000, "journal_bytes": 400, "free_disk_bytes": 1000,
                 "journal_files": ["appendonly.aof", "appendonly.aof.1"]},
            ],
            "rewrites": 3,
            "recovered_keys": 12,
            "graceful_stop": True,
            "errors": ["worker 1: boom"],
        }
        table = soak.summarize(report)
        self.assertIn("| RSS first/peak/last (KiB) | 1000 / 4000 / 2000 |", table)
        self.assertIn("| Recovered keys | 12 |", table)
        self.assertIn("| Journal first/peak/last (bytes) | 100 / 900 / 400 |", table)
        self.assertIn("| Free disk first/last (bytes) | 5000 / 1000 |", table)
        self.assertIn("appendonly.aof.1", table)
        self.assertIn("| error | worker 1: boom |", table)

    def test_operation_matrix_is_covered(self):
        self.assertEqual(
            {"set", "get", "incr", "expire", "lpush", "lrange", "hset", "hget"},
            set(soak.OPERATIONS),
        )


if __name__ == "__main__":
    unittest.main()

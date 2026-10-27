#!/usr/bin/env python3
"""Tests for the advisory lane map checker."""

from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import lane_check  # noqa: E402


class ClassifyTest(unittest.TestCase):
    def test_feature_paths_map_to_their_lane(self) -> None:
        cases = {
            "src/engine/streams.rs": "streams",
            "src/engine/collection_commands.rs": "collections",
            "src/engine/data.rs": "strings-keyspace",
            "src/engine/auth.rs": "security-clients",
            "src/engine/journal.rs": "persistence",
            "src/engine/slowlog.rs": "observability-admin",
            "src/engine/pubsub.rs": "pubsub",
            "src/server.rs": "io-server",
            "scripts/pr-performance-gate.py": "tooling",
        }
        for path, lane in cases.items():
            with self.subTest(path=path):
                self.assertEqual(lane_check.classify(path), lane)

    def test_shared_surfaces_are_not_claimed_by_a_feature_lane(self) -> None:
        for path in (
            "src/engine.rs",
            "src/session.rs",
            "src/engine/discovery.rs",
            "src/engine/discovery/mod.rs",
            "docs/requirements.md",
            "docs/requirements/streams.md",
            "docs/production-readiness.md",
            "README.md",
            "tests/discovery.rs",
        ):
            with self.subTest(path=path):
                self.assertTrue(lane_check.is_shared_surface(path))
                self.assertEqual(lane_check.classify(path), "surface")

    def test_post_split_discovery_family_files_belong_to_feature_lanes(self) -> None:
        cases = {
            "src/engine/discovery/streams.rs": "streams",
            "src/engine/discovery/strings.rs": "strings-keyspace",
            "src/engine/discovery/zsets.rs": "collections",
            "src/engine/discovery/pubsub.rs": "pubsub",
            "src/engine/discovery/unsupported.rs": "observability-admin",
        }
        for path, lane in cases.items():
            with self.subTest(path=path):
                self.assertEqual(lane_check.classify(path), lane)

    def test_unknown_paths_are_unowned(self) -> None:
        self.assertEqual(lane_check.classify("src/unknown.rs"), "unowned")
        self.assertEqual(lane_check.classify("docs/notes.md"), "unowned")


class SummarizeTest(unittest.TestCase):
    def test_groups_lanes_and_reports_shared_surfaces(self) -> None:
        report = lane_check.summarize(
            "origin/main",
            [
                "src/engine/streams.rs",
                "src/engine/streams.rs",
                "src/engine.rs",
                "src/unknown.rs",
            ],
        )
        self.assertEqual(report.files, ("src/engine.rs", "src/engine/streams.rs", "src/unknown.rs"))
        self.assertEqual(report.lanes["streams"], ("src/engine/streams.rs",))
        self.assertEqual(report.lanes["surface"], ("src/engine.rs",))
        self.assertEqual(report.shared, ("src/engine.rs",))
        self.assertEqual(report.unowned, ("src/unknown.rs",))

    def test_strict_failure_honors_allow_shared(self) -> None:
        report = lane_check.summarize("origin/main", ["src/engine.rs"])
        self.assertTrue(lane_check.strict_failure(report, allow_shared=False))
        self.assertFalse(lane_check.strict_failure(report, allow_shared=True))

    def test_unowned_fails_strict_even_when_shared_is_allowed(self) -> None:
        report = lane_check.summarize("origin/main", ["src/unknown.rs"])
        self.assertTrue(lane_check.strict_failure(report, allow_shared=True))

    def test_empty_report_is_clean(self) -> None:
        report = lane_check.summarize("origin/main", [])
        self.assertEqual(report.files, ())
        self.assertFalse(lane_check.strict_failure(report, allow_shared=False))


class ScriptSyntaxTest(unittest.TestCase):
    def test_worktree_helper_parses(self) -> None:
        script = Path(__file__).resolve().parent / "new-lane-worktree.sh"
        subprocess.run(["bash", "-n", str(script)], check=True)


if __name__ == "__main__":
    unittest.main()

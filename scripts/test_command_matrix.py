#!/usr/bin/env python3
"""Offline invariants for the C5 command matrix, without a running server.

`tests/command_matrix.rs` runs the live check against the tested head. These
tests keep the invariants themselves honest: they fail if the check would accept
a silently answered absent command, an undocumented named exclusion, a stale
pinned inventory or a matrix that records volatile payloads instead of reply
classes.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import command_matrix
from command_matrix import Observation, Row

PINNED = {
    "REDIS_VERSION": "8.10.2",
    "REDIS_SHA256": "5" * 64,
    "VALKEY_VERSION": "9.1.2",
    "VALKEY_SHA256": "6" * 64,
}


def fixture_text(rows: str, digest: str = PINNED["REDIS_SHA256"]) -> str:
    return (
        "# redis-version: 8.10.2\n"
        f"# redis-tarball-sha256: {digest}\n"
        "# redis-command-rows: 1\n"
        "# valkey-version: 9.1.2\n"
        f"# valkey-tarball-sha256: {PINNED['VALKEY_SHA256']}\n"
        "# valkey-command-rows: 1\n"
        "# union-rows: 1\n"
        f"{rows}"
    )


class MatrixTests(unittest.TestCase):
    def scratch(self) -> Path:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "benchmarks").mkdir()
        (root / "docs").mkdir()
        (root / command_matrix.UNSUPPORTED).write_text("no exclusions here\n", encoding="utf-8")
        (root / command_matrix.PROFILE).write_text("no exclusions here\n", encoding="utf-8")
        (root / command_matrix.TARGETS).write_text(
            "".join(f"{key} := {value}\n" for key, value in PINNED.items()),
            encoding="utf-8",
        )
        return root

    def test_absent_command_that_answers_fails_the_check(self):
        root = self.scratch()
        rows = [Row("eval", -3, -3)]
        observations = {"eval": Observation("eval", "status", "", "answered-status")}
        errors = command_matrix.verify_observations(root, rows, {}, observations)
        self.assertTrue(any("eval: absent command answered" in error for error in errors), errors)

    def test_absent_command_with_an_error_passes(self):
        root = self.scratch()
        rows = [Row("eval", -3, -3)]
        observations = {
            "eval": Observation("eval", "error", "ERR unknown command", "unknown-command")
        }
        self.assertEqual(command_matrix.verify_observations(root, rows, {}, observations), [])

    def test_unprobed_absent_name_fails(self):
        root = self.scratch()
        errors = command_matrix.verify_observations(root, [Row("eval", -3, -3)], {}, {})
        self.assertTrue(any("eval: not probed" in error for error in errors), errors)

    def test_undocumented_named_exclusion_fails(self):
        root = self.scratch()
        rows = [Row("dump", 2, 2)]
        observation = Observation(
            "dump", "error", "ERR DUMP is not supported by this server", "named-exclusion"
        )
        errors = command_matrix.verify_observations(root, rows, {}, {"dump": observation})
        self.assertTrue(any("not recorded in" in error for error in errors), errors)
        (root / command_matrix.UNSUPPORTED).write_text(
            "`DUMP` is not supported\n", encoding="utf-8"
        )
        self.assertEqual(
            command_matrix.verify_observations(root, rows, {}, {"dump": observation}), []
        )

    def test_documented_exclusion_through_a_container_parent_passes(self):
        root = self.scratch()
        (root / command_matrix.UNSUPPORTED).write_text(
            "## CLUSTER\n\nThis instance has cluster support disabled\n", encoding="utf-8"
        )
        rows = [Row("cluster", -2, -2), Row("cluster|slots", -2, -2)]
        observation = Observation(
            "cluster|slots",
            "error",
            "ERR This instance has cluster support disabled",
            "named-exclusion",
        )
        self.assertEqual(
            command_matrix.verify_observations(
                root, rows, {"cluster": (-2, ())}, {"cluster|slots": observation}
            ),
            [],
        )

    def test_advertised_name_outside_the_inventory_fails(self):
        root = self.scratch()
        errors = command_matrix.verify_observations(
            root, [Row("eval", -3, -3)], {"invented": (1, ())}, {}
        )
        self.assertTrue(any("invented" in error for error in errors), errors)

    def test_stale_pin_header_is_rejected(self):
        root = self.scratch()
        path = root / "inventory.tsv"
        path.write_text(fixture_text("eval\t-3\t-3\n", digest="0" * 64), encoding="utf-8")
        with self.assertRaises(SystemExit) as raised:
            command_matrix.load_fixture(path, root)
        self.assertIn("redis-tarball-sha256", str(raised.exception))

    def test_reference_row_counts_are_enforced(self):
        root = self.scratch()
        path = root / "inventory.tsv"
        path.write_text(fixture_text("eval\t-3\t-3\n"), encoding="utf-8")
        command_matrix.load_fixture(path, root)
        path.write_text(
            fixture_text("eval\t-3\t-3\n").replace("union-rows: 1", "union-rows: 2"),
            encoding="utf-8",
        )
        with self.assertRaises(SystemExit):
            command_matrix.load_fixture(path, root)

    def test_unsorted_or_duplicate_rows_are_rejected(self):
        root = self.scratch()
        path = root / "inventory.tsv"
        for rows in ("get\t2\t2\nacl\t-2\t-2\n", "get\t2\t2\nget\t2\t2\n"):
            path.write_text(fixture_text(rows).replace("union-rows: 1", "union-rows: 2"), encoding="utf-8")
            with self.assertRaises(SystemExit):
                command_matrix.load_fixture(path, root)

    def test_probe_arguments_use_the_reference_minimum_arity(self):
        self.assertEqual(command_matrix.probe_arguments("ping", 1), ["ping"])
        self.assertEqual(
            command_matrix.probe_arguments("config|get", -3), ["config", "get", "x"]
        )
        self.assertEqual(
            command_matrix.probe_arguments("VADD", -5), ["VADD", "x", "x", "x", "x"]
        )

    def test_render_records_reply_classes_not_payloads(self):
        rows = [Row("acl", -2, -2), Row("acl|genpass", -2, -2), Row("eval", -3, -3)]
        observations = {
            "acl|genpass": Observation(
                "acl|genpass", "bulk", "generated-password-payload", "answered-bulk"
            ),
            "eval": Observation("eval", "error", "ERR unknown command", "unknown-command"),
        }
        rendered = command_matrix.render(
            rows, {"acl": (-2, ("admin",))}, observations, PINNED
        )
        self.assertIn("`eval`", rendered)
        self.assertIn("ERR unknown command", rendered)
        self.assertIn("answered: `bulk`", rendered)
        self.assertNotIn("generated-password-payload", rendered)


if __name__ == "__main__":
    unittest.main(verbosity=2)

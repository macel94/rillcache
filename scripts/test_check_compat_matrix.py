#!/usr/bin/env python3
"""Regression tests for the compatibility-matrix checker (readiness P7)."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import check_compat_matrix as check

REPO_ROOT = Path(__file__).resolve().parent.parent

PINS = {
    "schema": 1,
    "upstreams": {
        "redis": {"version": "8.10.2", "revision": "a" * 40},
        "valkey": {"version": "9.1.2", "revision": "b" * 40},
    },
}
DAILY = {"schema": 1, "dragonfly": {"version": "2.0.2"}}
CLIENTS = {
    "schema": 1,
    "clients": [{"id": "redis-py", "version": "5.2.1"}],
}

DOC = """# Tested version matrix

| Upstream | Version | Pinned source | Exercised by |
| --- | --- | --- | --- |
| Redis | 8.10.2 | `ci/upstream/pins.json` | `.github/workflows/compat.yml` |
| Valkey | 9.1.2 | `ci/upstream/pins.json` | `.github/workflows/compat.yml` |
| Dragonfly | 2.0.2 | `ci/daily-pins.json` | `.github/workflows/daily.yml` |
| redis-py | 5.2.1 | `ci/upstream/clients-v1.json` | `.github/workflows/compat.yml` |

A single hardware generation is exercised.

There is no saturation model.

Competitor rows never a gate.

The support window lives in the release contract.
"""


class MatrixCase(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self._write("ci/upstream/pins.json", json.dumps(PINS))
        self._write("ci/daily-pins.json", json.dumps(DAILY))
        self._write("ci/upstream/clients-v1.json", json.dumps(CLIENTS))
        self._write("docs/compatibility-matrix.md", DOC)
        for workflow in ("compat.yml", "daily.yml"):
            self._write(f".github/workflows/{workflow}", "name: test\n")

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def _write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def _mutate(self, relative: str, old: str, new: str) -> None:
        path = self.root / relative
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1), encoding="utf-8")

    def assert_invalid(self, fragment: str) -> None:
        with self.assertRaises(check.Invalid) as raised:
            check.validate(self.root)
        self.assertIn(fragment, str(raised.exception))


class ValidCase(MatrixCase):
    def test_fixture_is_valid(self) -> None:
        self.assertEqual(len(check.validate(self.root)), 4)

    def test_repository_matrix_is_valid(self) -> None:
        rows = check.validate(REPO_ROOT)
        self.assertIn("redis", rows)
        self.assertIn("valkey", rows)


class DriftCase(MatrixCase):
    def test_missing_pinned_version_fails(self) -> None:
        self._mutate("docs/compatibility-matrix.md", "| Dragonfly | 2.0.2 |", "| Dragonfly | 2.0.1 |")
        self.assert_invalid("but pinned as")

    def test_absent_pinned_entry_fails(self) -> None:
        self._mutate(
            "docs/compatibility-matrix.md",
            "| redis-py | 5.2.1 | `ci/upstream/clients-v1.json` | `.github/workflows/compat.yml` |\n",
            "",
        )
        self.assert_invalid("is not in the matrix")

    def test_row_without_pin_fails(self) -> None:
        self._mutate(
            "docs/compatibility-matrix.md",
            "| Dragonfly | 2.0.2 |",
            "| KeyDB | 6.3.4 | `ci/daily-pins.json` | `.github/workflows/daily.yml` |\n| Dragonfly | 2.0.2 |",
        )
        self.assert_invalid("rows without a pin")

    def test_missing_workflow_file_fails(self) -> None:
        (self.root / ".github/workflows/daily.yml").unlink()
        self.assert_invalid("missing workflow")

    def test_missing_pin_reference_fails(self) -> None:
        self._mutate(
            "docs/compatibility-matrix.md",
            "| Dragonfly | 2.0.2 | `ci/daily-pins.json` |",
            "| Dragonfly | 2.0.2 | the daily pins |",
        )
        self.assert_invalid("must name ci/daily-pins.json")

    def test_dropped_limit_statement_fails(self) -> None:
        self._mutate(
            "docs/compatibility-matrix.md",
            "There is no saturation model.",
            "Latency is measured.",
        )
        self.assert_invalid("retained limits must still state")

    def test_no_workflow_named_fails(self) -> None:
        path = self.root / "docs/compatibility-matrix.md"
        text = path.read_text(encoding="utf-8")
        path.write_text(
            text.replace("`.github/workflows/compat.yml`", "the compatibility workflow")
            .replace("`.github/workflows/daily.yml`", "the daily workflow"),
            encoding="utf-8",
        )
        self.assert_invalid("no exercising workflow")

    def test_missing_version_field_in_pins_fails(self) -> None:
        self._write("ci/upstream/pins.json", json.dumps({"schema": 1, "upstreams": {"redis": {}}}))
        self.assert_invalid("has no version")


if __name__ == "__main__":
    unittest.main()

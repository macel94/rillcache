#!/usr/bin/env python3
"""Regression tests for the package-visibility checker (readiness L7)."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_package_visibility as check

REPO_ROOT = Path(__file__).resolve().parent.parent
DIGEST = "sha256:" + "a" * 64
OTHER = "sha256:" + "b" * 64

CHANGELOG = """# Changelog

## 0.1.0 — 2026-11-27
"""

HEADER = (
    "| Version | amd64 digest | arm64 digest | Visibility | Verified by | "
    "Verified at | Method |\n"
    "| --- | --- | --- | --- | --- | --- | --- |\n"
)


def row(state: str = "public-verified", version: str = "0.1.0") -> str:
    if state == "public-verified":
        return (
            f"| {version} | {DIGEST} | {OTHER} | public-verified | Maintainer | "
            "2026-11-27 | Anonymous `docker pull` on both platforms |\n"
        )
    return f"| {version} | - | - | unverified | - | - | Not verified yet. |\n"


class VisibilityCase(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self._write("CHANGELOG.md", CHANGELOG)
        self.set_record(HEADER + row())

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def _write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def set_record(self, table: str) -> None:
        self._write("docs/acceptance/package-visibility.md", "# Record\n\n" + table)

    def assert_invalid(self, fragment: str, require_public: bool = False) -> None:
        with self.assertRaises(check.Invalid) as raised:
            check.validate(self.root, require_public=require_public, version=None)
        self.assertIn(fragment, str(raised.exception))


class ValidCase(VisibilityCase):
    def test_public_row_is_valid(self) -> None:
        self.assertEqual(
            check.validate(self.root, require_public=True, version=None), ["0.1.0"]
        )

    def test_unverified_row_is_valid_in_record_mode(self) -> None:
        self.set_record(HEADER + row("unverified"))
        self.assertEqual(
            check.validate(self.root, require_public=False, version=None), ["0.1.0"]
        )

    def test_repository_record_is_valid(self) -> None:
        self.assertEqual(
            check.validate(REPO_ROOT, require_public=False, version=None), ["0.1.0"]
        )


class RejectCase(VisibilityCase):
    def test_unverified_row_blocks_publish(self) -> None:
        self.set_record(HEADER + row("unverified"))
        self.assert_invalid("must be recorded as public-verified", require_public=True)

    def test_missing_version_row_fails(self) -> None:
        self._write("CHANGELOG.md", CHANGELOG + "\n## 0.2.0 — 2026-12-01\n")
        self.assert_invalid("has no package-visibility row")

    def test_unreleased_row_fails(self) -> None:
        self.set_record(HEADER + row() + row(version="9.9.9"))
        self.assert_invalid("is not a released version")

    def test_unknown_state_fails(self) -> None:
        self.set_record(HEADER + row().replace("public-verified | Maintainer", "public | Maintainer"))
        self.assert_invalid("unknown visibility state")

    def test_bad_digest_fails(self) -> None:
        self.set_record(HEADER + row().replace(DIGEST, "sha256:short"))
        self.assert_invalid("must be a sha256 digest")

    def test_missing_verifier_fails(self) -> None:
        self.set_record(HEADER + row().replace("| Maintainer |", "| - |"))
        self.assert_invalid("needs the verifying maintainer")

    def test_missing_date_fails(self) -> None:
        self.set_record(HEADER + row().replace("2026-11-27 | Anonymous", "yesterday | Anonymous"))
        self.assert_invalid("needs an ISO date")

    def test_short_method_fails(self) -> None:
        self.set_record(
            HEADER
            + row().replace(
                "Anonymous `docker pull` on both platforms", "pull"
            )
        )
        self.assert_invalid("needs the verification method")

    def test_duplicate_row_fails(self) -> None:
        self.set_record(HEADER + row() + row())
        self.assert_invalid("duplicate row")

    def test_missing_table_fails(self) -> None:
        self.set_record("no table here\n")
        self.assert_invalid("missing the record table header")

    def test_empty_table_fails(self) -> None:
        self.set_record(HEADER)
        self.assert_invalid("has no rows")

    def test_version_filter_without_match_fails(self) -> None:
        with self.assertRaises(check.Invalid):
            check.validate(self.root, require_public=False, version="0.2.0")

    def test_malformed_cell_count_fails(self) -> None:
        self.set_record(HEADER + "| 0.1.0 | only two |\n")
        self.assert_invalid("cells, expected")


if __name__ == "__main__":
    unittest.main()

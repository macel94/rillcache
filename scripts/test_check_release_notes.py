#!/usr/bin/env python3
"""Regression tests for the release-notes checker (readiness L4)."""

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

import check_release_notes as check

REPO_ROOT = Path(__file__).resolve().parent.parent
CHECKLIST = """# Production readiness gaps

- [ ] **S1. Security.** [P0]
- [ ] **S5. Certificates.** [P0]
- [x] **L1. License.** [P0]
- [ ] **V3. Sign-offs.** [P0]
- [ ] **V6. Runbook.** [P1]
"""

NOTES = """# v0.1.0 release notes

## Compatibility profile

Profile row.

## Migration

No migration; the journal format is unchanged.

## Rollback

Rollback to the same journal format is supported.

## Support window

Twelve months.

## Accepted limitations

- `S1` — open.
- `S5` — open.
- `V3` — open.

## Verification

- `scripts/upgrade_drill.py` exercises upgrade and rollback.
- `scripts/restore_drill.py` exercises restore.
"""

CHANGELOG = """# Changelog

## 0.1.0 — 2026-11-27

Release notes: [docs/releases/v0.1.0.md](docs/releases/v0.1.0.md).
"""


class ReleaseNotesCase(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self._write("docs/production-readiness-gaps.md", CHECKLIST)
        self._write("CHANGELOG.md", CHANGELOG)
        self._write("docs/releases/v0.1.0.md", NOTES)
        for drill in check.REQUIRED_DRILLS:
            self._write(drill, "# drill\n")

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


class ValidCase(ReleaseNotesCase):
    def test_fixture_is_valid(self) -> None:
        self.assertEqual(check.validate(self.root), ["0.1.0"])

    def test_repository_release_notes_are_valid(self) -> None:
        versions = check.validate(REPO_ROOT)
        self.assertIn("0.1.0", versions)


class DriftCase(ReleaseNotesCase):
    def test_missing_notes_file_fails(self) -> None:
        (self.root / "docs/releases/v0.1.0.md").unlink()
        self.assert_invalid("missing release notes")

    def test_changelog_without_notes_link_fails(self) -> None:
        self._mutate(
            "CHANGELOG.md",
            "Release notes: [docs/releases/v0.1.0.md](docs/releases/v0.1.0.md).",
            "Notes: see the production readiness checklist.",
        )
        self.assert_invalid("does not link")

    def test_missing_section_fails(self) -> None:
        self._mutate("docs/releases/v0.1.0.md", "## Support window", "## Window")
        self.assert_invalid("section 'Support window' is missing")

    def test_empty_section_fails(self) -> None:
        self._mutate("docs/releases/v0.1.0.md", "Twelve months.", "")
        self.assert_invalid("is missing or empty")

    def test_open_p0_omitted_fails(self) -> None:
        self._mutate("docs/releases/v0.1.0.md", "- `V3` — open.\n", "")
        self.assert_invalid("omit open P0 gaps")

    def test_closed_gap_listed_as_open_fails(self) -> None:
        self._mutate(
            "docs/releases/v0.1.0.md",
            "- `V3` — open.\n",
            "- `V3` — open.\n- `L1` — open.\n",
        )
        self.assert_invalid("not open")

    def test_missing_drill_reference_fails(self) -> None:
        self._mutate(
            "docs/releases/v0.1.0.md",
            "- `scripts/upgrade_drill.py` exercises upgrade and rollback.\n",
            "- The upgrade path is exercised.\n",
        )
        self.assert_invalid("does not name the exercised drill")

    def test_reference_to_missing_script_fails(self) -> None:
        self._mutate(
            "docs/releases/v0.1.0.md",
            "- `scripts/restore_drill.py` exercises restore.",
            "- `scripts/restore_drill.py` exercises restore and `scripts/missing.py`.",
        )
        self.assert_invalid("references missing path")

    def test_journal_change_without_migration_position_fails(self) -> None:
        self._mutate(
            "docs/releases/v0.1.0.md",
            "- `scripts/restore_drill.py` exercises restore.",
            "- `scripts/restore_drill.py` exercises restore of the journal archive.",
        )
        self._mutate(
            "docs/releases/v0.1.0.md",
            "No migration; the journal format is unchanged.",
            "Dump and restore the keyspace instead of replaying on-disk bytes.",
        )
        self._mutate(
            "docs/releases/v0.1.0.md",
            "Rollback to the same journal format is supported.",
            "Rollback is supported within one minor version.",
        )
        self.assert_invalid("does not state its position")

    def test_sectionless_notes_fail(self) -> None:
        self._write("docs/releases/v0.1.0.md", "# notes without sections\n")
        self.assert_invalid("is missing or empty")

    def test_checklist_without_open_p0_fails(self) -> None:
        self._write(
            "docs/production-readiness-gaps.md",
            "# Production readiness gaps\n\n- [x] **S1. Security.** [P0]\n",
        )
        self.assert_invalid("no open P0")

    def test_changelog_without_versions_fails(self) -> None:
        self._write("CHANGELOG.md", "# Changelog\n\n## Unreleased\n\nNothing.\n")
        self.assert_invalid("no versioned entry")


if __name__ == "__main__":
    unittest.main()

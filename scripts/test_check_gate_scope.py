#!/usr/bin/env python3
"""Regression tests for the gate-scope checker (readiness V7)."""

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

import check_gate_scope as check

REPO_ROOT = Path(__file__).resolve().parent.parent

GATES = """{
  "workflows": {
    "ci.yml": {"jobs": {"verify": ["Tests and offline CI"]}},
    "pr-quality.yml": {"jobs": {"benchmark": ["Valkey performance gate"]}}
  }
}
"""

CHECKLIST = """# Production readiness gaps

- [ ] **S1. Security review.** [P0]
- [ ] **P2. Controlled hardware.** [P1]
- [x] **V6. Runbook executed.** [P1]
"""

DOC = """# What each merge gate proves

No row here is a production acceptance. These gates do not prove production readiness.

| Required context | Proves | Does not prove |
| --- | --- | --- |
| `Tests and offline CI` | Builds and tests the workspace. | Compatibility, latency or durability. |
| `Valkey performance gate` | Beats Valkey on the shared runner. | Controlled hardware or an SLO. |

## What no merge gate proves

- Nothing about production readiness.

## Production gates still missing

- `S1` — independent security review.
- `P2` — controlled hardware evidence.

## Where the production evidence lives

- Checklist.
"""


class GateScopeCase(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self._write(".github/pr-workflow-gates.json", GATES)
        self._write("docs/production-readiness-gaps.md", CHECKLIST)
        self._write("docs/gate-scope.md", DOC)

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


class ValidCase(GateScopeCase):
    def test_fixture_is_valid(self) -> None:
        self.assertEqual(check.validate(self.root), 2)

    def test_repository_gate_scope_is_valid(self) -> None:
        self.assertEqual(check.validate(REPO_ROOT), 16)


class DriftCase(GateScopeCase):
    def test_unmapped_context_fails(self) -> None:
        self._mutate(
            ".github/pr-workflow-gates.json",
            '{"benchmark": ["Valkey performance gate"]}',
            '{"benchmark": ["Valkey performance gate", "New gate"]}',
        )
        self.assert_invalid("is not mapped")

    def test_extra_mapped_context_fails(self) -> None:
        self._mutate(
            "docs/gate-scope.md",
            "| `Valkey performance gate` |",
            "| `Retired gate` | old | nothing |\n| `Valkey performance gate` |",
        )
        self.assert_invalid("not required")

    def test_empty_does_not_prove_cell_fails(self) -> None:
        self._mutate(
            "docs/gate-scope.md",
            "| Compatibility, latency or durability. |",
            "| |",
        )
        self.assert_invalid("must state both")

    def test_missing_section_fails(self) -> None:
        self._mutate("docs/gate-scope.md", "## What no merge gate proves", "## Notes")
        self.assert_invalid("is missing the section")

    def test_missing_gate_section_without_ids_fails(self) -> None:
        self._mutate(
            "docs/gate-scope.md",
            "- `S1` — independent security review.\n- `P2` — controlled hardware evidence.",
            "- None tracked.",
        )
        self.assert_invalid("names no checklist identifier")

    def test_closed_gap_advertised_as_missing_fails(self) -> None:
        self._mutate(
            "docs/gate-scope.md",
            "- `P2` — controlled hardware evidence.",
            "- `P2` — controlled hardware evidence.\n- `V6` — runbook execution.",
        )
        self.assert_invalid("advertises gaps that are closed")

    def test_production_claim_fails(self) -> None:
        self._mutate(
            "docs/gate-scope.md",
            "| Beats Valkey on the shared runner. |",
            "| This gate proves production readiness for the profile. |",
        )
        self.assert_invalid("claims a production pass")

    def test_gates_file_without_contexts_fails(self) -> None:
        self._write(
            ".github/pr-workflow-gates.json",
            '{"workflows": {"ci.yml": {"jobs": {}}}}',
        )
        self.assert_invalid("defines no required contexts")


if __name__ == "__main__":
    unittest.main()

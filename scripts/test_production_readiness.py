import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from production_readiness import AREAS, CHECKLIST, ROOT, inspect, inventory, main


def checklist() -> str:
    lines = ["**Status: NOT PRODUCTION READY.**"]
    for index, area in enumerate(AREAS, 1):
        lines.append(f"## {index}. Area {area}")
        lines.append(f"- [{' ' if index < 4 else 'x'}] **{area}1. Example.** [P{(index - 1) % 3}]")
    return "\n".join(lines)


class ProductionReadinessTests(unittest.TestCase):
    def fixture(self, source: str) -> Path:
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name)
        (root / CHECKLIST.parent).mkdir()
        (root / CHECKLIST).write_text(source, encoding="utf-8")
        (root / "README.md").write_text(f"[Checklist]({CHECKLIST})\n", encoding="utf-8")
        (root / "docs/README.md").write_text(f"[Checklist]({CHECKLIST.name})\n", encoding="utf-8")
        return root

    def test_checkout_integrity(self):
        _, gaps = inspect(ROOT)
        self.assertTrue(gaps)

    def test_report_distinguishes_blockers_and_followups(self):
        root = self.fixture(checklist())
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(main(["--json"], root), 1)
        report = json.loads(out.getvalue())
        self.assertEqual([g["id"] for g in report["blockers"]], ["C1", "D1"])
        self.assertEqual([g["id"] for g in report["open_followups"]], ["A1"])
        self.assertFalse(report["release_evidence_verified"])
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(main(["--check"], root), 0)

    def test_reports_invalid_document_and_missing_entry_link(self):
        root = self.fixture(checklist())
        (root / "docs/README.md").write_text("[Wrong path](missing.md)\n", encoding="utf-8")
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(main([], root), 2)
        self.assertIn("docs/README.md: missing direct link", err.getvalue())

    def test_rejects_malformed_duplicate_and_incomplete_inventory(self):
        valid = checklist()
        row = "- [ ] **C1. Example.** [P0]"
        for broken in (
            valid.replace(row, ""),
            valid.replace(row, f"{row}\n{row}"),
            valid.replace("[P0]", "[P3]"),
            valid.replace("**C1.", "**D7."),
        ):
            with self.subTest(broken=broken), self.assertRaises(ValueError):
                inventory(broken)

    def test_rejects_ready_assertion_with_blockers(self):
        with self.assertRaisesRegex(ValueError, "contradicts open P0/P1"):
            inventory(checklist().replace("NOT PRODUCTION READY", "PRODUCTION READY"))

    def test_empty_blocker_list_never_verifies_release_evidence(self):
        root = self.fixture(checklist().replace("- [ ]", "- [x]"))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(main(["--json"], root), 1)
        report = json.loads(out.getvalue())
        self.assertEqual(report["blockers"], [])
        self.assertFalse(report["release_evidence_verified"])


if __name__ == "__main__":
    unittest.main()

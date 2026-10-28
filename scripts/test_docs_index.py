#!/usr/bin/env python3
"""Unit tests for scripts/check_docs_index.py (standard library only)."""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import check_docs_index  # noqa: E402


INDEX = """# Requirements

- [Area A](requirements/a.md)
- [Area B](requirements/b.md)
"""

PRODUCTION_INDEX = """# Readiness

- [Capability](production-readiness/capability.md)
"""

AREA_A = """# Area A

- **R1 (event-driven):** A requirement.
- **R-HC1 (ubiquitous):** Another requirement.
"""

AREA_B = """# Area B

- **R2 (event-driven):** A requirement.

| ID | Story and value | Size | Acceptance evidence |
| --- | --- | --- | --- |
| S01 | A story. | XS | R1 |
"""

CAPABILITY = """# Capability

| Capability | Boundary |
| --- | --- |
| Capability | Evidence. |
"""


def build_tree(root, index=INDEX, production_index=PRODUCTION_INDEX,
               areas=None, capabilities=None):
    os.makedirs(os.path.join(root, "docs", "requirements"))
    os.makedirs(os.path.join(root, "docs", "production-readiness"))
    with open(os.path.join(root, "docs", "requirements.md"), "w", encoding="utf-8") as handle:
        handle.write(index)
    with open(os.path.join(root, "docs", "production-readiness.md"), "w", encoding="utf-8") as handle:
        handle.write(production_index)
    for name, text in (areas if areas is not None else {"a.md": AREA_A, "b.md": AREA_B}).items():
        with open(os.path.join(root, "docs", "requirements", name), "w", encoding="utf-8") as handle:
            handle.write(text)
    for name, text in (capabilities if capabilities is not None
                       else {"capability.md": CAPABILITY}).items():
        with open(os.path.join(root, "docs", "production-readiness", name), "w", encoding="utf-8") as handle:
            handle.write(text)


class CheckDocsIndexTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def test_valid_split_passes(self):
        build_tree(self.root)
        self.assertEqual(check_docs_index.check(self.root), [])

    def test_unlinked_area_file_fails(self):
        build_tree(self.root, areas={"a.md": AREA_A, "b.md": AREA_B, "c.md": "# Area C\n"})
        errors = check_docs_index.check(self.root)
        self.assertTrue(any("c.md is not linked" in error for error in errors), errors)

    def test_missing_linked_file_fails(self):
        index = INDEX + "- [Missing](requirements/missing.md)\n"
        build_tree(self.root, index=index)
        errors = check_docs_index.check(self.root)
        self.assertTrue(any("missing.md" in error and "missing area file" in error for error in errors), errors)

    def test_duplicate_requirement_id_fails(self):
        duplicate = AREA_B + "\n- **R1 (state-driven):** Duplicate.\n"
        build_tree(self.root, areas={"a.md": AREA_A, "b.md": duplicate})
        errors = check_docs_index.check(self.root)
        self.assertTrue(any("duplicate requirement ID R1" in error for error in errors), errors)

    def test_duplicate_story_id_fails(self):
        duplicate = AREA_B.replace("| S01 |", "| S02 |") + "\n| S02 | Again. | XS | R1 |\n"
        build_tree(self.root, areas={"a.md": AREA_A, "b.md": duplicate})
        errors = check_docs_index.check(self.root)
        self.assertTrue(any("duplicate story ID S02" in error for error in errors), errors)

    def test_conflict_marker_fails(self):
        build_tree(self.root, areas={"a.md": AREA_A + "<<<<<<< HEAD\n", "b.md": AREA_B})
        errors = check_docs_index.check(self.root)
        self.assertTrue(any("conflict marker" in error for error in errors), errors)

    def test_empty_split_fails(self):
        build_tree(self.root)
        os.remove(os.path.join(self.root, "docs", "requirements", "a.md"))
        os.remove(os.path.join(self.root, "docs", "requirements", "b.md"))
        errors = check_docs_index.check(self.root)
        self.assertTrue(any("empty split" in error for error in errors), errors)

    def test_anchor_links_are_accepted(self):
        index = INDEX.replace("a.md", "a.md#requirements")
        build_tree(self.root, index=index)
        self.assertEqual(check_docs_index.check(self.root), [])

    def test_main_reports_success_and_failure(self):
        build_tree(self.root)
        self.assertEqual(check_docs_index.main(["--root", self.root]), 0)
        with open(os.path.join(self.root, "docs", "requirements", "a.md"), "a", encoding="utf-8") as handle:
            handle.write(">>>>>>> branch\n")
        self.assertEqual(check_docs_index.main(["--root", self.root]), 1)


if __name__ == "__main__":
    unittest.main()

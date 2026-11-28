#!/usr/bin/env python3
"""Negative controls for scripts/check_corpus_coverage.py (V2).

The accounting must fail closed: a drifted coverage file, a missing file, a
wrong schema or a corpus command that is not advertised must all be reported
rather than silently accepted as coverage.
"""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_corpus_coverage as coverage  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


class CoverageTests(unittest.TestCase):
    def fixture(self) -> Path:
        directory = Path(tempfile.mkdtemp())
        for relative in (coverage.MATRIX, coverage.CORPUS, coverage.COVERAGE):
            target = directory / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        return directory

    def test_repository_accounting_is_valid(self):
        self.assertEqual([], coverage.problems(ROOT, coverage.build(ROOT)))

    def test_drifted_coverage_file_is_rejected(self):
        directory = self.fixture()
        path = directory / coverage.COVERAGE
        data = json.loads(path.read_text())
        data["covered"] += 1
        path.write_text(json.dumps(data))
        self.assertTrue(coverage.problems(directory, coverage.build(directory)))

    def test_missing_file_schema_and_unadvertised_commands_fail_closed(self):
        directory = self.fixture()
        path = directory / coverage.COVERAGE
        path.unlink()
        self.assertIn("missing", coverage.problems(directory, coverage.build(directory))[0])
        path.write_text(json.dumps({"schema": 99}))
        self.assertTrue(coverage.problems(directory, coverage.build(directory)))

        corpus = directory / coverage.CORPUS
        corpus.write_text(corpus.read_text() + '\n&[b"notacommand", b"x"]\n')
        expected = coverage.build(directory)
        self.assertIn("notacommand", expected["corpus_commands_outside_matrix"])

    def test_every_uncovered_command_is_reported_explicitly(self):
        expected = coverage.build(ROOT)
        self.assertEqual(
            expected["advertised"],
            expected["covered"] + expected["uncovered"],
        )
        self.assertEqual(expected["uncovered"], len(expected["uncovered_commands"]))


if __name__ == "__main__":
    unittest.main()

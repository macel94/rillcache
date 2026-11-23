import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from release_evidence import MAX_BYTES, REVIEW_STAGES, STAGES, main

EXPECTED = {"candidate": "1" * 40, "artifact": "sha256:" + "2" * 64, "profile": "3" * 40}


def record():
    return {"schema": 1, "results": {
        stage: {**EXPECTED, "result": "pass", "method": "test command", "observed": "fixture ok",
                "environment": "isolated fixture", "evidence_url": f"https://example.test/{stage}",
                **({"reviewer": "fixture reviewer"} if stage in REVIEW_STAGES else {})}
        for stage in STAGES
    }}


class ReleaseEvidenceTests(unittest.TestCase):
    def report(self, document, exit_code):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "record.json"
            path.write_text(json.dumps(document) if isinstance(document, dict) else document, encoding="utf-8")
            argv = [str(path)] + [part for name, value in EXPECTED.items() for part in (f"--{name}", value)]
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(main(argv), exit_code)
        report = json.loads(output.getvalue())
        self.assertFalse(report["promotion_authorized"])
        self.assertFalse(report["release_evidence_verified"])
        return report

    def test_complete_structure_is_not_approval(self):
        self.assertTrue(self.report(record(), 0)["record_complete"])

    def test_green_merge_alone_leaves_every_release_stage_missing(self):
        document = record()
        document["results"] = {"merge": document["results"]["merge"]}
        problems = self.report(document, 1)["stage_problems"]
        self.assertEqual(problems["merge"], [])
        self.assertTrue(all(problems[stage] == ["missing stage"] for stage in STAGES[1:]))

    def test_stale_failed_missing_context_and_not_run_are_independent(self):
        document = record()
        document["results"]["recovery"].update(candidate="0" * 40, result="fail")
        document["results"]["compatibility"]["result"] = "not-run"
        document["results"]["security"].update(reviewer=" ", environment=" ", evidence_url="http://example.test")
        document["results"]["load-soak"].update(artifact="sha256:" + "0" * 64, profile="0" * 40)
        problems = self.report(document, 1)["stage_problems"]
        self.assertEqual(problems["recovery"], ["candidate missing or does not match", "result is fail"])
        self.assertEqual(problems["compatibility"], ["result is not-run"])
        self.assertEqual(problems["security"], ["missing environment", "missing or invalid HTTPS evidence URL", "missing named reviewer"])
        self.assertEqual(problems["load-soak"], ["artifact missing or does not match", "profile missing or does not match"])

    def test_malformed_or_ambiguous_documents_are_rejected(self):
        for document in ('{"schema": 0, "schema": 1, "results": {}}', ' ' * (MAX_BYTES + 1),
                         {"schema": True, "results": {}}, {"schema": 1, "results": {"typo": {}}}):
            with self.subTest(case=type(document).__name__):
                self.assertFalse(self.report(document, 2)["record_complete"])


if __name__ == "__main__":
    unittest.main()

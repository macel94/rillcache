import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from release_evidence import MAX_BYTES, REVIEW_STAGES, STAGES, main

EXPECTED = {"candidate": "1" * 40, "artifact": "sha256:" + "2" * 64, "profile": "3" * 40}
RUNBOOK = Path(__file__).resolve().parents[1] / "docs/production-readiness/acceptance-runbook.md"


def record():
    return {"schema": 2, "results": {
        stage: {**EXPECTED, "result": "pass", "method": "test command", "observed": "fixture ok",
                "environment": "isolated fixture", "evidence_url": f"https://example.test/{stage}",
                **({"reviewer": "fixture reviewer"} if stage in REVIEW_STAGES else {}),
                **({"supported_topology": "single node, standalone", "exclusions": []} if stage == "scope" else {})}
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

    def test_runbook_documents_the_schema_stages_and_scope_fields(self):
        runbook = RUNBOOK.read_text(encoding="utf-8")
        self.assertIn("schema: 2", runbook)
        for stage in STAGES:
            with self.subTest(stage=stage):
                self.assertIn(f"`{stage}`", runbook)
        self.assertIn("`supported_topology`", runbook)
        self.assertIn("an `exclusions` array", runbook)
        self.assertIn("`release_evidence_verified: false`", runbook)
        self.assertIn("`promotion_authorized: false`", runbook)

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

    def test_scope_requires_topology_review_and_an_explicit_exclusion_list(self):
        document = record()
        scope = document["results"]["scope"]
        del scope["supported_topology"]
        del scope["exclusions"]
        scope["reviewer"] = " "
        problems = self.report(document, 1)["stage_problems"]["scope"]
        self.assertEqual(problems, [
            "missing named reviewer", "missing supported topology", "missing or invalid exclusions list",
        ])

        document = record()
        document["results"]["scope"]["exclusions"] = ["   "]
        problems = self.report(document, 1)["stage_problems"]["scope"]
        self.assertEqual(problems, ["missing or invalid exclusions list"])

    def test_empty_exclusions_is_an_explicit_valid_scope_value(self):
        document = record()
        document["results"]["scope"]["exclusions"] = []
        report = self.report(document, 0)
        self.assertEqual(report["stage_problems"]["scope"], [])
        self.assertFalse(report["release_evidence_verified"])
        self.assertFalse(report["promotion_authorized"])

    def test_malformed_or_ambiguous_documents_are_rejected(self):
        for document in ('{"schema": 0, "schema": 2, "results": {}}', ' ' * (MAX_BYTES + 1),
                         {"schema": True, "results": {}}, {"schema": 1, "results": {}},
                         {"schema": 2, "results": {"typo": {}}}):
            with self.subTest(case=type(document).__name__):
                self.assertFalse(self.report(document, 2)["record_complete"])


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Regression tests for the security review registry checker."""

from __future__ import annotations

import copy
import json
import tempfile
import unittest
from pathlib import Path

import security_review

REPO_ROOT = Path(__file__).resolve().parent.parent
REGISTRY = json.loads(
    (REPO_ROOT / security_review.REGISTRY_PATH).read_text(encoding="utf-8")
)


class RegistryCase(unittest.TestCase):
    """Builds a throwaway tree that mirrors the real registry layout."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.data = copy.deepcopy(REGISTRY)
        for relative in (
            self.data["review"]["threat_model"],
            self.data["review"]["scope_document"],
        ):
            self._write(relative, self._document_text())
        for threat in self.data["threats"]:
            for evidence in threat["evidence"]:
                self._touch(evidence)
        for finding in self.data["findings"]:
            for key in ("closure_evidence", "retest_evidence"):
                for evidence in finding.get(key, []):
                    self._touch(evidence)
        for signoff in self.data["signoffs"]:
            for evidence in signoff.get("evidence", []):
                self._touch(evidence)
        self._write_registry()

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def _touch(self, relative: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            path.write_text("evidence\n", encoding="utf-8")

    def _write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def _write_registry(self) -> None:
        self._write(
            str(security_review.REGISTRY_PATH),
            json.dumps(self.data, indent=2) + "\n",
        )

    def _document_text(self) -> str:
        ids = [threat["id"] for threat in self.data["threats"]]
        ids += [finding["id"] for finding in self.data["findings"]]
        return "\n".join(ids) + "\n"

    def assert_invalid(self, fragment: str) -> None:
        with self.assertRaises(security_review.Invalid) as raised:
            security_review.validate(self.root)
        self.assertIn(fragment, str(raised.exception))

    def assert_valid(self) -> dict:
        return security_review.validate(self.root)


class ValidRegistryTest(RegistryCase):
    def test_repository_registry_is_valid(self) -> None:
        data = security_review.validate(REPO_ROOT)
        self.assertEqual(len(data["threats"]), 10)
        independent = [
            s for s in data["signoffs"] if s["kind"] == "independent"
        ]
        self.assertEqual(len(independent), 1)
        self.assertEqual(independent[0]["status"], "outstanding")

    def test_check_reports_outstanding_signature(self) -> None:
        exit_code = security_review.main(
            ["--check", "--root", str(REPO_ROOT)]
        )
        self.assertEqual(exit_code, 0)

    def test_fixture_tree_is_valid(self) -> None:
        self.assert_valid()


class ThreatValidationTest(RegistryCase):
    def test_missing_threat_evidence_path_fails(self) -> None:
        self.data["threats"][0]["evidence"].append("tests/does_not_exist.rs")
        self._write_registry()
        self.assert_invalid("missing evidence path")

    def test_threat_without_controls_fails(self) -> None:
        self.data["threats"][0]["controls"] = []
        self._write_registry()
        self.assert_invalid("controls must be a non-empty list")

    def test_unknown_threat_status_fails(self) -> None:
        self.data["threats"][0]["status"] = "probably-fine"
        self._write_registry()
        self.assert_invalid("bad status")

    def test_threat_without_residual_risk_fails(self) -> None:
        self.data["threats"][0].pop("residual_risk")
        self._write_registry()
        self.assert_invalid("residual_risk is required")

    def test_duplicate_threat_id_fails(self) -> None:
        self.data["threats"][1]["id"] = self.data["threats"][0]["id"]
        self._write_registry()
        self.assert_invalid("duplicate id")

    def test_threat_missing_from_documents_fails(self) -> None:
        self.data["threats"][0]["id"] = "T99"
        self.data["findings"] = [
            finding
            for finding in self.data["findings"]
            if finding["threat"] != "T1"
        ]
        self._write(self.data["review"]["threat_model"], "T2\n")
        self._write(self.data["review"]["scope_document"], "T2\n")
        self._write_registry()
        self.assert_invalid("not in the threat model")


class FindingValidationTest(RegistryCase):
    def test_closed_finding_without_retest_evidence_fails(self) -> None:
        closed = next(f for f in self.data["findings"] if f["status"] == "closed")
        closed.pop("retest_evidence")
        self._write_registry()
        self.assert_invalid("retest_evidence")

    def test_closed_finding_with_missing_closure_evidence_fails(self) -> None:
        closed = next(f for f in self.data["findings"] if f["status"] == "closed")
        closed["closure_evidence"] = ["docs/missing-closure.md"]
        self._write_registry()
        self.assert_invalid("missing evidence path")

    def test_open_finding_without_tracking_gap_fails(self) -> None:
        opened = next(f for f in self.data["findings"] if f["status"] == "open")
        opened["tracking"] = "somewhere later"
        self._write_registry()
        self.assert_invalid("checklist gap id")

    def test_risk_accepted_finding_needs_note(self) -> None:
        accepted = next(
            f for f in self.data["findings"] if f["status"] == "risk-accepted"
        )
        accepted.pop("risk_note")
        self._write_registry()
        self.assert_invalid("risk_note is required")

    def test_finding_with_unknown_threat_fails(self) -> None:
        self.data["findings"][0]["threat"] = "T404"
        self._write_registry()
        self.assert_invalid("unknown threat")

    def test_finding_missing_from_scope_document_fails(self) -> None:
        excluded = self.data["findings"][0]["id"]
        text = "\n".join(
            [t["id"] for t in self.data["threats"]]
            + [f["id"] for f in self.data["findings"] if f["id"] != excluded]
        )
        self._write(self.data["review"]["scope_document"], text + "\n")
        self._write_registry()
        self.assert_invalid("is not in the review scope")


class SignoffValidationTest(RegistryCase):
    def test_signed_independent_without_reviewer_fails(self) -> None:
        independent = [
            s for s in self.data["signoffs"] if s["kind"] == "independent"
        ][0]
        independent["status"] = "signed"
        independent["blocking_note"] = ""
        self._write_registry()
        self.assert_invalid("reviewer is required")

    def test_signed_independent_without_signature_fails(self) -> None:
        independent = [
            s for s in self.data["signoffs"] if s["kind"] == "independent"
        ][0]
        independent.update(
            {
                "status": "signed",
                "reviewer": "External Reviewer",
                "date": "2026-12-01",
                "signature": "docs/security/signature.txt",
            }
        )
        independent.pop("blocking_note", None)
        self._write_registry()
        self.assert_invalid("missing evidence path")

    def test_signed_independent_with_signature_is_accepted(self) -> None:
        independent = [
            s for s in self.data["signoffs"] if s["kind"] == "independent"
        ][0]
        signature = "docs/security/signature.txt"
        independent.update(
            {
                "status": "signed",
                "reviewer": "External Reviewer",
                "date": "2026-12-01",
                "signature": signature,
            }
        )
        independent.pop("blocking_note", None)
        self._touch(signature)
        self._write_registry()
        self.assert_valid()

    def test_outstanding_signoff_without_note_fails(self) -> None:
        independent = [
            s for s in self.data["signoffs"] if s["kind"] == "independent"
        ][0]
        independent.pop("blocking_note")
        self._write_registry()
        self.assert_invalid("blocking_note is required")

    def test_outstanding_signoff_with_reviewer_fails(self) -> None:
        independent = [
            s for s in self.data["signoffs"] if s["kind"] == "independent"
        ][0]
        independent["reviewer"] = "Someone"
        self._write_registry()
        self.assert_invalid("must not name a reviewer")

    def test_two_independent_records_fail(self) -> None:
        signoffs = self.data["signoffs"]
        signoffs.append(
            {
                "id": "SO-3",
                "kind": "independent",
                "status": "outstanding",
                "reviewer": None,
                "blocking_note": "duplicate",
            }
        )
        self._write_registry()
        self.assert_invalid("exactly one independent sign-off")

    def test_overclaiming_scope_document_fails(self) -> None:
        text = self._document_text() + "The independent review is complete.\n"
        self._write(self.data["review"]["scope_document"], text)
        self._write_registry()
        self.assert_invalid("claims a completed independent review")


class ReviewBlockTest(RegistryCase):
    def test_bad_scope_date_fails(self) -> None:
        self.data["review"]["scope_defined"] = "27-11-2026"
        self._write_registry()
        self.assert_invalid("ISO date")

    def test_missing_scope_document_fails(self) -> None:
        (self.root / self.data["review"]["scope_document"]).unlink()
        self.assert_invalid("missing evidence path")

    def test_missing_external_signature_statement_fails(self) -> None:
        self.data["review"].pop("external_signature")
        self._write_registry()
        self.assert_invalid("external_signature is required")

    def test_invalid_json_fails(self) -> None:
        self._write(str(security_review.REGISTRY_PATH), "{not json")
        self.assert_invalid("invalid JSON")


if __name__ == "__main__":
    unittest.main()

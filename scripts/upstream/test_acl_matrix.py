"""Offline tests for the versioned ACL allow/deny matrix (S2); no servers run."""

import io
import json
import unittest

import acl_matrix as am


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.manifest = json.loads(am.MANIFEST.read_text())

    def test_declared_matrix_matches_document_and_scenarios(self):
        manifest = am.check_manifest(self.manifest)
        self.assertEqual("rillcache-acl-v1", manifest["matrix"]["id"])
        self.assertEqual("S2", manifest["matrix"]["gap"])
        self.assertGreaterEqual(len(manifest["scenarios"]), 12)
        document = (am.common.ROOT / manifest["matrix"]["document"]).read_text()
        self.assertIn("rillcache-acl-v1", document)
        self.assertIn("ci/upstream/acl-v1.json", document)
        ids = {scenario["id"] for scenario in manifest["scenarios"]}
        for divergence in manifest["declared_divergences"]:
            self.assertTrue(set(divergence["scenarios"]) <= ids)
            self.assertTrue(divergence["reason"])

    def test_manifest_rejects_bad_decisions_and_length_mismatch(self):
        broken = json.loads(json.dumps(self.manifest))
        broken["scenarios"][0]["expect"] = ["maybe"] * len(broken["scenarios"][0]["commands"])
        with self.assertRaisesRegex(ValueError, "unknown expectation"):
            am.check_manifest(broken)

        mismatch = json.loads(json.dumps(self.manifest))
        mismatch["scenarios"][0]["commands"].append(["PING"])
        with self.assertRaisesRegex(ValueError, "length differ"):
            am.check_manifest(mismatch)

        duplicate = json.loads(json.dumps(self.manifest))
        duplicate["scenarios"].append(json.loads(json.dumps(duplicate["scenarios"][0])))
        with self.assertRaisesRegex(ValueError, "duplicate scenario"):
            am.check_manifest(duplicate)

        unknown = json.loads(json.dumps(self.manifest))
        unknown["declared_divergences"][0]["scenarios"].append("no-such-scenario")
        with self.assertRaisesRegex(ValueError, "unknown scenario"):
            am.check_manifest(unknown)

        missing_doc = json.loads(json.dumps(self.manifest))
        missing_doc["matrix"]["document"] = "docs/no-such-security.md"
        with self.assertRaisesRegex(ValueError, "document is missing"):
            am.check_manifest(missing_doc)


class DecisionTests(unittest.TestCase):
    def test_decisions_distinguish_policy_denial_from_other_errors(self):
        self.assertEqual("deny", am.decision({"error": "NOPERM this user has no permissions"}))
        self.assertEqual("error", am.decision({"error": "ERR wrong number of arguments"}))
        self.assertEqual("allow", am.decision(b"OK"))
        self.assertEqual("allow", am.decision(1))
        self.assertEqual("allow", am.decision([b"a"]))

    def test_normalize_tags_resp_classes_for_post_state_checks(self):
        self.assertEqual({"int": 1}, am.normalize(1))
        self.assertEqual({"bulk": "v", "status": "v"}, am.normalize(b"v"))
        self.assertEqual({"array": [{"int": 1}]}, am.normalize([1]))
        self.assertEqual({"null": True}, am.normalize(None))
        self.assertEqual({"error": "NOPERM x"}, am.normalize({"error": "NOPERM x"}))

    def test_acl_file_enables_the_default_user_and_is_bounded(self):
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as temp:
            path = am.acl_file(Path(temp) / "users.acl", {}, ["on", "~data:*", "+@read"],
                               {"user": "matrix", "password": "matrix-secret"},
                               {"user": "admin", "password": "admin-secret"})
            text = path.read_text()
        self.assertTrue(text.startswith("user default on >admin-secret"))
        self.assertIn("user admin on >admin-secret", text)
        self.assertIn("user matrix on ~data:* +@read >matrix-secret", text)


class VerificationTests(unittest.TestCase):
    def record(self, decisions):
        return {"decisions": decisions, "post": [], "auth": {"outcome": "allow"}}

    def test_candidate_mismatch_and_reference_difference_are_reported(self):
        manifest = {"declared_divergences": [],
                    "scenarios": [{"id": "s1", "expect": ["allow"],
                                   "differential": True, "post_expect": []}]}
        records = {"rillcache": {"s1": self.record(["deny"])},
                   "redis": {"s1": self.record(["deny"])},
                   "valkey": {"s1": self.record(["allow"])}}
        problems = am.verify(manifest, records)
        self.assertEqual({"candidate-decision", "reference-difference"},
                         {problem["kind"] for problem in problems})

    def test_declared_divergence_records_candidate_decision(self):
        scenario = {"id": "s1", "expect": ["deny"], "differential": True, "post_expect": []}
        manifest = {"declared_divergences": [{"id": "d", "scenarios": ["s1"], "reason": "reviewed"}],
                    "scenarios": [scenario]}
        agreeing = {"rillcache": {"s1": self.record(["deny"])},
                    "redis": {"s1": self.record(["allow"])},
                    "valkey": {"s1": self.record(["allow"])}}
        self.assertEqual([], am.verify(manifest, agreeing))
        drifted = {"rillcache": {"s1": self.record(["allow"])},
                   "redis": {"s1": self.record(["allow"])},
                   "valkey": {"s1": self.record(["allow"])}}
        kinds = {problem["kind"] for problem in am.verify(manifest, drifted)}
        self.assertIn("divergence-candidate", kinds)
        self.assertIn("candidate-decision", kinds)

    def test_summary_keeps_failures_visible(self):
        result = {"status": "failed", "matrix": {"id": "m", "version": 1, "gap": "S2"},
                  "candidate": {"head": "h"}, "scenarios": [
                      {"id": "s1", "candidate": {"decisions": ["allow"]}, "expect": ["allow"],
                       "references": {"redis": ["allow"], "valkey": ["allow"]}}],
                  "failures": [{"kind": "candidate-decision"}]}
        text = am.summary_text(result)
        self.assertIn("## ACL allow/deny matrix: failed", text)
        self.assertIn("- failures: 1", text)
        self.assertIn("not a security sign-off", text)


if __name__ == "__main__":
    unittest.main()

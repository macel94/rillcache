"""Offline tests for the versioned language-client matrix; no clients or servers run."""

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import client_matrix as cm
import common


def fake_harness_payload(cases, version="1.0.0"):
    return {"client": "fake", "version": version, "protocols": [2, 3],
            "cases": [{"id": case_id, "status": status, "detail": detail}
                      for case_id, status, detail in cases]}


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.manifest = json.loads(cm.MANIFEST.read_text())

    def test_declared_matrix_matches_pins_and_checklist(self):
        manifest = cm.check_manifest(self.manifest)
        self.assertEqual("rillcache-language-clients", manifest["matrix"]["id"])
        self.assertEqual("C7", manifest["matrix"]["gap"])
        self.assertEqual({"redis-py", "node-redis", "go-redis"},
                         {client["id"] for client in manifest["clients"]})
        checklist = Path(cm.common.ROOT, "docs/production-readiness-gaps.md").read_text()
        self.assertIn("**C7.", checklist)
        document = Path(cm.common.ROOT, manifest["matrix"]["document"]).read_text()
        self.assertIn("ci/upstream/clients-v1.json", document)
        self.assertIn(manifest["matrix"]["id"], document)
        for client in manifest["clients"]:
            self.assertEqual(client["version"], cm.read_pin(client)["version"])

    def test_pinned_lock_files_are_enforced(self):
        drifted = json.loads(json.dumps(self.manifest))
        drifted["clients"][0]["version"] = "0.0.1"
        with self.assertRaisesRegex(ValueError, "pinned lock file"):
            cm.check_manifest(drifted)

        missing_pin = json.loads(json.dumps(self.manifest))
        missing_pin["clients"][1]["pin"] = "ci/upstream/no-such-lock.json"
        with self.assertRaisesRegex(ValueError, "pin file is missing"):
            cm.check_manifest(missing_pin)

        missing_harness = json.loads(json.dumps(self.manifest))
        missing_harness["clients"][0]["harness"] = "scripts/no-such-harness.py"
        with self.assertRaisesRegex(ValueError, "harness is missing"):
            cm.check_manifest(missing_harness)

    def test_manifest_rejects_untracked_cases_and_bad_expectations(self):
        extra = json.loads(json.dumps(self.manifest))
        extra["cases"].append({"id": "handshake-auth", "description": "duplicate"})
        with self.assertRaisesRegex(ValueError, "duplicate case"):
            cm.check_manifest(extra)

        unknown_expectation = json.loads(json.dumps(self.manifest))
        unknown_expectation["clients"][0]["expectations"] = {
            "no-such-case": {"detail_regex": "^x$", "reason": "reviewed"}}
        with self.assertRaisesRegex(ValueError, "unknown case"):
            cm.check_manifest(unknown_expectation)

        bad_regex = json.loads(json.dumps(self.manifest))
        bad_regex["clients"][0]["expectations"] = {
            "resp3-roundtrip": {"detail_regex": "(", "reason": "reviewed"}}
        with self.assertRaisesRegex(ValueError, "regex is invalid"):
            cm.check_manifest(bad_regex)

        unreviewed = json.loads(json.dumps(self.manifest))
        unreviewed["clients"][0]["expectations"]["resp3-roundtrip"] = {"detail_regex": "^x$"}
        with self.assertRaisesRegex(ValueError, "reviewable reason"):
            cm.check_manifest(unreviewed)


class HarnessResultTests(unittest.TestCase):
    """Run a scripted harness and check how its case report is judged."""

    def setUp(self):
        self.directory = Path(tempfile.mkdtemp())
        self.harness = self.directory / "fake_harness.py"
        self.harness.write_text(
            "import json, os, sys\n"
            "args = sys.argv[1:]\n"
            "out = args[args.index('--json-out') + 1]\n"
            "payload = json.load(open(os.environ['FAKE_PAYLOAD']))\n"
            "open(out, 'w').write(json.dumps(payload))\n")
        self.manifest = {"credentials": {"named_user": "u", "named_password": "p",
                                        "limited_user": "l", "limited_password": "q",
                                        "default_password": "d"}}
        self.cases = [{"id": case_id} for case_id in ("handshake-auth", "acl-allowed")]
        self.client = {"id": "fake", "version": "1.0.0", "language": "python", "pin": "unused",
                       "harness": str(self.harness),
                       "command": ["python3", str(self.harness)], "exclusions": []}

    def judge(self, payload, client=None):
        fixture = self.directory / "payload.json"
        fixture.write_text(json.dumps(payload))
        with patch.dict(os.environ, {"FAKE_PAYLOAD": str(fixture)}):
            return cm.run_harness(client or self.client, self.manifest,
                                  {"plain": 64000, "tls": 64001, "alt": 64002}, self.directory)

    def test_all_declared_cases_passed(self):
        payload = fake_harness_payload([("handshake-auth", "passed", ""), ("acl-allowed", "passed", "")])
        manifest_cases = self.cases
        self.manifest["cases"] = manifest_cases
        record = self.judge(payload)
        self.assertEqual("passed", record["status"], record.get("problems"))
        self.assertEqual(2, len(record["cases"]))

    def test_missing_failed_extra_and_wrong_version_cases_fail(self):
        self.manifest["cases"] = self.cases
        cases = fake_harness_payload([("handshake-auth", "passed", "")])
        self.assertIn("was not reported", json.dumps(self.judge(cases)["problems"]))

        failed = fake_harness_payload([("handshake-auth", "passed", ""), ("acl-allowed", "failed", "boom")])
        self.assertIn("failed", json.dumps(self.judge(failed)["problems"]))

        extra = fake_harness_payload([("handshake-auth", "passed", ""), ("acl-allowed", "passed", ""),
                                      ("undeclared", "passed", "")])
        self.assertIn("undeclared case results", json.dumps(self.judge(extra)["problems"]))

        wrong_version = fake_harness_payload([("handshake-auth", "passed", ""),
                                              ("acl-allowed", "passed", "")], version="9.9.9")
        self.assertIn("differs from the pin", json.dumps(self.judge(wrong_version)["problems"]))

    def test_declared_exclusion_and_frozen_detail_are_enforced(self):
        self.manifest["cases"] = self.cases
        excluded_client = dict(self.client, exclusions=[{"case": "acl-allowed", "reason": "not applicable"}])
        excluded = fake_harness_payload([("handshake-auth", "passed", ""), ("acl-allowed", "excluded", "")])
        self.assertEqual("passed", self.judge(excluded, excluded_client)["status"])
        ran_instead = fake_harness_payload([("handshake-auth", "passed", ""), ("acl-allowed", "passed", "")])
        self.assertIn("declared exclusion ran", json.dumps(self.judge(ran_instead, excluded_client)["problems"]))

        expectation = {"resp3-roundtrip": {"detail_regex": "^frozen$", "reason": "reviewed"}}
        self.manifest["cases"] = [{"id": "resp3-roundtrip"}]
        frozen_client = dict(self.client, expectations=expectation)
        self.assertEqual("passed", self.judge(fake_harness_payload(
            [("resp3-roundtrip", "passed", "frozen")]), frozen_client)["status"])
        self.assertIn("declared detail drifted", json.dumps(self.judge(fake_harness_payload(
            [("resp3-roundtrip", "passed", "")]), frozen_client)["problems"]))

    def test_missing_results_fail_closed(self):
        self.manifest["cases"] = self.cases
        missing = {"id": "fake", "version": "1.0.0", "language": "python", "pin": "unused",
                   "harness": str(self.directory / "absent.py"),
                   "command": ["python3", str(self.directory / "absent.py")], "exclusions": []}
        record = cm.run_harness(missing, self.manifest,
                                {"plain": 64000, "tls": 64001, "alt": 64002}, self.directory)
        self.assertEqual("failed", record["status"])
        self.assertIn("no case results", record["failure"])


class SupportTests(unittest.TestCase):
    def test_acl_and_ports(self):
        directory = Path(tempfile.mkdtemp())
        path = directory / "users.acl"
        cm.write_acl(path, {"default_password": "d", "named_user": "matrix", "named_password": "m",
                            "limited_user": "limited", "limited_password": "l"})
        text = path.read_text()
        for fragment in ("user default on >d", "user matrix on >m", "user limited on >l ~allowed:*"):
            self.assertIn(fragment, text)
        ports = cm.distinct_ports(3)
        self.assertEqual(3, len(set(ports.values())))

    def test_summary_counts_cases(self):
        result = {"status": "passed", "matrix": {"id": "m", "version": 1},
                  "candidate": {"head": "h", "sha256": "s"},
                  "clients": [{"id": "c", "declared_version": "1", "status": "passed",
                               "cases": [{"id": "a", "status": "passed"}]}], "failures": []}
        text = cm.summary_text(result)
        self.assertIn("| c | 1 | 1/1 passed | passed |", text)
        self.assertIn("not a support window", text)


if __name__ == "__main__":
    unittest.main()

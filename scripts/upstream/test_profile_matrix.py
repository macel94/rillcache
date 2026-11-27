"""Offline tests for the versioned compatibility profile matrix; no servers run."""

import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import common
import profile_matrix as pm


class ProfileManifestTests(unittest.TestCase):
    def setUp(self):
        self.manifest = json.loads(pm.MANIFEST.read_text())

    def test_declared_profile_matches_pins_document_and_checklist(self):
        manifest = pm.check_manifest(self.manifest)
        self.assertEqual("rillcache-standalone-0.1", manifest["profile"]["id"])
        self.assertEqual("draft", manifest["profile"]["status"])
        self.assertEqual({"redis", "valkey"}, set(manifest["references"]))
        for name, pin in common.PINS["upstreams"].items():
            self.assertEqual(pin["version"], manifest["references"][name]["version"])
            self.assertEqual(pin["sha256"], manifest["references"][name]["sha256"])
        checklist = pm.CHECKLIST.read_text()
        for exclusion in manifest["exclusions"]:
            self.assertIn(f"**{exclusion['gap']}.", checklist)
        document = (common.ROOT / manifest["profile"]["document"]).read_text()
        self.assertIn("ci/upstream/profile-v1.json", document)
        self.assertIn(manifest["profile"]["id"], document)

    def test_manifest_rejects_pin_drift_and_untracked_exclusions(self):
        drifted = json.loads(json.dumps(self.manifest))
        drifted["references"]["redis"]["version"] = "8.0.0"
        with self.assertRaisesRegex(ValueError, "pins.json"):
            pm.check_manifest(drifted)

        unknown_gap = json.loads(json.dumps(self.manifest))
        unknown_gap["exclusions"][0]["gap"] = "Z9"
        with self.assertRaisesRegex(ValueError, "canonical checklist"):
            pm.check_manifest(unknown_gap)

        duplicate = json.loads(json.dumps(self.manifest))
        duplicate["rows"].append(json.loads(json.dumps(duplicate["rows"][0])))
        with self.assertRaisesRegex(ValueError, "duplicate row"):
            pm.check_manifest(duplicate)

        bad_index = json.loads(json.dumps(self.manifest))
        bad_index["rows"][0]["ttl_indices"] = [99]
        with self.assertRaisesRegex(ValueError, "out of range"):
            pm.check_manifest(bad_index)

        bad_digest = json.loads(json.dumps(self.manifest))
        bad_digest["declared_row_divergences"][0]["candidate_sha256"] = "not-a-digest"
        with self.assertRaisesRegex(ValueError, "sha256"):
            pm.check_manifest(bad_digest)

        no_clients = json.loads(json.dumps(self.manifest))
        no_clients["client_rows"] = []
        with self.assertRaisesRegex(ValueError, "client row"):
            pm.check_manifest(no_clients)

    def test_document_must_reference_the_manifest(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "ci/upstream").mkdir(parents=True)
            (root / "ci/upstream/profile-v1.json").write_text("{}")
            (root / "docs").mkdir()
            (root / "docs/production-profile.md").write_text("no identity here")
            (root / "benchmarks").mkdir()
            (root / "benchmarks/targets.mk").write_text(
                (common.ROOT / "benchmarks/targets.mk").read_text())
            (root / "docs/production-readiness-gaps.md").write_text(
                (common.ROOT / "docs/production-readiness-gaps.md").read_text())
            manifest = json.loads(json.dumps(self.manifest))
            with self.assertRaisesRegex(ValueError, "must reference"):
                pm.check_manifest(manifest, root=root)


class NormalizationTests(unittest.TestCase):
    def test_declared_normalizations_are_bounded_and_explicit(self):
        row = {"ttl_indices": [1], "sort_replies": [2], "length_replies": [3],
               "sort_pair_replies": [4], "float_replies": [5]}
        replies = [b"OK", 100, [b"b", b"a"], [b"x", b"y", b"z"], [b"b", b"2", b"a", b"1"],
                   [b"13.36138933897018433", b"member", b"12"]]
        normalized = pm.normalize(row, replies)
        self.assertEqual("valid-positive-ttl", normalized[1])
        self.assertEqual([b"a", b"b"], normalized[2])
        self.assertEqual({"reply_length": 3}, normalized[3])
        self.assertEqual([b"a", b"1", b"b", b"2"], normalized[4])
        self.assertEqual([{"float": 13.361389}, b"member", b"12"], normalized[5])

        implausible = pm.normalize({"ttl_indices": [0]}, [10 ** 12])
        self.assertEqual("implausible-ttl", implausible[0])
        self.assertEqual(-1, pm.normalize({"ttl_indices": [0]}, [-1])[0])

    def test_select_fields_keeps_only_named_portable_fields(self):
        row = {"select_fields": [{"index": 0, "fields": ["length", "groups"]},
                                 {"index": 1, "fields": ["name", "lag"], "each": True}]}
        replies = [[b"length", 1, b"radix-tree-keys", 2, b"groups", 1],
                   [[b"name", b"g", b"consumers", 1, b"lag", 3]]]
        self.assertEqual([[b"length", 1, b"groups", 1],
                          [[b"name", b"g", b"lag", 3]]], pm.normalize(row, replies))

    def test_placeholder_prefixes_isolate_rows(self):
        commands = pm.commands_for([["SET", "{p}key", "v"], ["KEYS", "{p}k*"]], "rows-")
        self.assertEqual([["SET", "rows-key", "v"], ["KEYS", "rows-k*"]], commands)
        self.assertEqual([b"\x00\xff"], pm.commands_for([[{"bytes_hex": "00ff"}]], "x-")[0])


class RowClassificationTests(unittest.TestCase):
    def manifest_with(self, candidate_sha, reference_sha):
        return {"declared_row_divergences": [{
            "id": "frozen", "row": "row", "references": ["redis"],
            "candidate_sha256": candidate_sha, "reference_sha256": reference_sha, "reason": "reviewed"}]}

    def test_identical_rows_pass_without_a_declaration(self):
        self.assertEqual({"status": "passed"},
                         pm.classify_row(self.manifest_with("a" * 64, "b" * 64), "row", "redis",
                                         [b"OK"], [b"OK"]))

    def test_declared_divergence_must_match_frozen_replies_exactly(self):
        candidate, reference = [1, b"\x00", [b"x"]], [b"1", b"\x00", [b"x"]]
        manifest = self.manifest_with(pm.digest(candidate), pm.digest(reference))
        accepted = pm.classify_row(manifest, "row", "redis", candidate, reference)
        self.assertEqual("declared-divergence", accepted["status"])
        self.assertEqual("frozen", accepted["declared_divergence"])
        self.assertEqual(pm.evidence(candidate), accepted["candidate"])

        drifted = pm.classify_row(manifest, "row", "redis", candidate, [b"1", b"\x00", [b"y"]])
        self.assertEqual("failed", drifted["status"])
        self.assertIn("drifted", drifted["failure"])

        undeclared = pm.classify_row({"declared_row_divergences": []}, "row", "redis", candidate, reference)
        self.assertEqual("failed", undeclared["status"])
        self.assertNotIn("failure", undeclared)

    def test_declaration_only_applies_to_its_reference(self):
        candidate, reference = [1], [b"1"]
        manifest = self.manifest_with(pm.digest(candidate), pm.digest(reference))
        self.assertEqual("failed", pm.classify_row(manifest, "row", "valkey", candidate, reference)["status"])
        self.assertEqual("failed", pm.classify_row(manifest, "other", "redis", candidate, reference)["status"])


class SurfaceTests(unittest.TestCase):
    def test_only_candidate_commands_are_compared_and_exemptions_are_reviewed(self):
        candidate = {"get": {"present": True, "arity": 2}, "pfdebug": {"present": True, "arity": -3},
                     "extension": {"present": True, "arity": 2}}
        reference = {"get": {"present": True, "arity": 2}, "pfdebug": {"present": True, "arity": 3},
                     "cluster": {"present": True, "arity": 2}}
        declared = [{"id": "pfdebug-arity", "command": "PFDEBUG", "field": "arity", "candidate": -3,
                     "reason": "bounded"},
                    {"id": "extension-valkey", "command": "extension", "field": "presence",
                     "references": ["valkey"], "reason": "redis-only extension"}]
        self.assertEqual([], pm.surface_comparison(candidate, reference, declared, "valkey"))
        self.assertEqual(["extension"], [item["command"] for item in
                                         pm.surface_comparison(candidate, reference, declared, "redis")])
        changed = pm.surface_comparison(candidate, {"get": {"present": True, "arity": 3},
                                                    "pfdebug": {"present": True, "arity": 3}},
                                        declared, "valkey")
        self.assertEqual([{"command": "get", "field": "arity", "candidate": 2, "reference": 3,
                           "declared_divergence": None}], changed)


class RespFramingTests(unittest.TestCase):
    def test_null_array_and_nested_error_are_values_not_desynchronizing_failures(self):
        self.assertIsNone(common.read_reply(io.BytesIO(b"*-1\r\n")))
        nested = common.read_reply(io.BytesIO(b"*2\r\n-ERR runtime\r\n$1\r\na\r\n"))
        self.assertIsInstance(nested[0], common.ErrorReply)
        self.assertEqual("ERR runtime", nested[0])
        self.assertEqual(b"a", nested[1])
        with self.assertRaisesRegex(RuntimeError, "ERR boom"):
            common.read_reply(io.BytesIO(b"-ERR boom\r\n"))

    def test_error_and_bulk_replies_stay_distinguishable(self):
        self.assertEqual({"error": "ERR x"}, pm.evidence(common.ErrorReply("ERR x")))
        self.assertEqual({"bytes_hex": "4552522078"}, pm.evidence(b"ERR x"))


class ExecutionTests(unittest.TestCase):
    def test_run_records_pass_fail_and_keeps_failed_evidence(self):
        manifest = {"schema": 1,
                    "profile": {"id": "test", "version": 1, "status": "draft", "document": "docs/production-profile.md"},
                    "references": ["redis", "valkey"],
                    "rows": [{"id": "ok", "family": "connection", "commands": [["PING"]]},
                             {"id": "bad", "family": "connection", "commands": [["ECHO", "payload"]]}],
                    "exclusions": [{"id": "exclusion", "gap": "C2", "reason": "excluded",
                                    "commands": [{"argv": ["EVAL", "return 1", "0"],
                                                  "candidate": "error", "reference": "success"}]}],
                    "client_rows": [{"id": "ping", "client": "redis-cli", "args": ["PING"]}],
                    "commands": {"declared_divergences": []},
                    "declared_row_divergences": []}
        candidate = Path(tempfile.mkdtemp()) / "candidate"
        candidate.write_bytes(b"fake candidate identity")
        cli = Path(tempfile.mkdtemp()) / "redis-cli"
        cli.write_text("#!/bin/sh\nprintf 'PONG\\n'\n")
        cli.chmod(0o755)
        active = {"name": None}

        class FakeClient:
            def __init__(self, _port):
                pass

            def call(self, *args):
                if args[:2] == ("COMMAND", "LIST"):
                    return [b"ping"]
                if args[0] == "COMMAND":
                    return [[b"ping", 1, [], 0, 0, 0]]
                if args[0] == "PING":
                    return b"PONG"
                if args[0] == "ECHO":
                    return b"payload" if active["name"] == "rillcache" else b"other"
                if args[0] == "EVAL":
                    if active["name"] == "rillcache":
                        raise RuntimeError("ERR scripting is not supported")
                    return 1
                raise AssertionError(f"unexpected command {args}")

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

        from contextlib import contextmanager

        @contextmanager
        def fake_server(name, directory, **_kwargs):
            active["name"] = name
            yield None, 6379

        output = Path(tempfile.mkdtemp())
        with patch.object(pm.common, "server", fake_server), \
                patch.object(pm.common, "Client", FakeClient), \
                patch.object(pm, "CLIENT_BINARIES", {"redis-cli": lambda: cli}), \
                patch.object(pm, "binary_identity", lambda name: {"path": name, "sha256": "0" * 64}), \
                patch.object(pm, "head", lambda: "test-head"):
            result = pm.run(output, candidate, manifest)
        self.assertEqual("failed", result["status"])
        self.assertEqual(["row", "row"], [failure["kind"] for failure in result["failures"]])
        self.assertEqual(["bad", "bad"], [failure["id"] for failure in result["failures"]])
        failed = [row for row in result["rows"] if row["id"] == "bad"]
        self.assertEqual(2, len(failed))
        self.assertEqual([{"bytes_hex": b"payload".hex()}], failed[0]["candidate"])
        self.assertEqual([{"bytes_hex": b"other".hex()}], failed[0]["reference_replies"])
        self.assertEqual(["passed", "passed"],
                         [row["status"] for row in result["rows"] if row["id"] == "ok"])
        stored = json.loads((output / "result.json").read_text())
        self.assertEqual(result["status"], stored["status"])
        self.assertTrue(pm.summary_text(result).startswith("## Compatibility profile matrix: failed"))

    def test_missing_reference_binary_identity_fails_the_run(self):
        candidate = Path(tempfile.mkdtemp()) / "candidate"
        candidate.write_bytes(b"fake candidate identity")
        manifest = {"schema": 1,
                    "profile": {"id": "test", "version": 1, "status": "draft",
                                "document": "docs/production-profile.md"},
                    "references": ["redis", "valkey"], "rows": [], "exclusions": [], "client_rows": [],
                    "commands": {"declared_divergences": [], "minimum_rows": 0},
                    "declared_row_divergences": []}
        cli = Path(tempfile.mkdtemp()) / "redis-cli"
        cli.write_text("#!/bin/sh\nexit 0\n")
        cli.chmod(0o755)

        class EmptyClient:
            def __init__(self, _port):
                pass

            def call(self, *args):
                if args == ("COMMAND", "LIST"):
                    return []
                raise AssertionError(f"unexpected command {args}")

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

        from contextlib import contextmanager

        @contextmanager
        def fake_server(name, directory, **_kwargs):
            yield None, 6379

        with tempfile.TemporaryDirectory() as temp, \
                patch.object(pm, "CLIENT_BINARIES", {"redis-cli": lambda: cli}), \
                patch.object(pm.common, "server", fake_server), \
                patch.object(pm.common, "Client", EmptyClient), \
                patch.object(pm, "binary_identity", lambda name: {"path": name, "sha256": None}), \
                patch.object(pm, "head", lambda: "test-head"):
            result = pm.run(Path(temp), candidate, manifest)
        self.assertEqual("failed", result["status"])
        self.assertEqual({"reference-binary"}, {failure["kind"] for failure in result["failures"]})


if __name__ == "__main__":
    unittest.main()

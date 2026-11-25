"""Offline policy tests for the hosted-only differential corpus."""
import json
import tempfile
import unittest
from contextlib import contextmanager
from pathlib import Path
from unittest.mock import patch

import differential as diff


class DifferentialCorpusTests(unittest.TestCase):
    def test_manifest_is_versioned_and_covers_required_behavior(self):
        manifest = json.loads(diff.MANIFEST.read_text())
        self.assertEqual(1, manifest["schema"])
        self.assertEqual(["redis", "valkey"], manifest["references"])
        self.assertEqual({"binary-set-get", "expiry", "expiry-persist", "wrong-type", "transaction", "integer-counter", "integer-counter-invalid", "list-order", "delete-key", "conditional-set"},
                         {case["id"] for case in manifest["cases"]})

    def test_seeded_commands_repeat_and_binary_is_exact(self):
        manifest = json.loads(diff.MANIFEST.read_text())
        case = manifest["cases"][0]
        first = diff.case_commands(case, manifest["seed"])
        self.assertEqual(first, diff.case_commands(case, manifest["seed"]))
        self.assertEqual(b"a\0b\xff", first[0][2])

    def test_runner_retains_exact_dialect_divergences_and_versions(self):
        manifest = json.loads(diff.MANIFEST.read_text())
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            candidate = root / "candidate"
            candidate.write_bytes(b"fake binary identity")
            active = {"name": None}

            @contextmanager
            def fake_server(name, _directory, **_kwargs):
                active["name"] = name
                yield None, 6379

            class FakeClient:
                def __init__(self, _port):
                    pass

                def __enter__(self):
                    return self

                def __exit__(self, *_args):
                    pass

                def call(self, *args):
                    if args[0] == "LPUSH" and args[1].startswith("wrong:"):
                        raise RuntimeError("ERR wrong kind of value" if active["name"] == "redis"
                                           else "WRONGTYPE Operation against a key holding the wrong kind of value")
                    return b"OK"

            with patch.object(diff.common, "server", fake_server), patch.object(diff.common, "Client", FakeClient):
                self.assertTrue(diff.run(root / "out", candidate))
            result = json.loads((root / "out/result.json").read_text())
            self.assertTrue(result["candidate"]["version"])
            self.assertEqual({"8.10.2", "9.1.2"}, {v["version"] for v in result["references"].values()})
            self.assertEqual(["wrong-type"], [d["case"] for d in result["divergences"]])
            wrong_type_seed = manifest["seed"] + next(
                i for i, case in enumerate(manifest["cases"]) if case["id"] == "wrong-type")
            self.assertEqual([wrong_type_seed], [d["seed"] for d in result["divergences"]])
            self.assertEqual("ERR wrong kind of value",
                             result["divergences"][0]["reference_result"][1]["error"])
            self.assertEqual("WRONGTYPE Operation against a key holding the wrong kind of value",
                             result["divergences"][0]["candidate"][1]["error"])
            self.assertNotEqual(result["divergences"][0]["candidate"],
                                result["divergences"][0]["reference_result"])

    def test_expiry_comparison_discards_only_clock_value(self):
        case = {"semantic": "expiry"}
        replies = [{"reply": b"OK"}, {"reply": 273}, {"reply": 0}]
        self.assertEqual("valid-positive-ttl", diff.normalize_replies(case, replies)[1]["reply"])
        self.assertEqual(0, replies[2]["reply"])
        persisted = {"semantic": "expiry-persist"}
        self.assertEqual("persistent", diff.normalize_replies(
            persisted, [{"reply": b"OK"}, {"reply": 1}, {"reply": -1}, {"reply": b"v"}])[2]["reply"])
        self.assertEqual("invalid-persistent-ttl", diff.normalize_replies(
            persisted, [{"reply": b"OK"}, {"reply": 1}, {"reply": 10}, {"reply": b"v"}])[2]["reply"])


if __name__ == "__main__":
    unittest.main()

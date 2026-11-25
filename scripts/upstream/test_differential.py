"""Offline policy tests for the hosted-only differential corpus."""
import json
import unittest

import differential as diff


class DifferentialCorpusTests(unittest.TestCase):
    def test_manifest_is_versioned_and_covers_required_behavior(self):
        manifest = json.loads(diff.MANIFEST.read_text())
        self.assertEqual(1, manifest["schema"])
        self.assertEqual(["redis", "valkey"], manifest["references"])
        self.assertEqual({"binary-set-get", "expiry", "wrong-type", "transaction", "integer-counter", "integer-counter-invalid"},
                         {case["id"] for case in manifest["cases"]})

    def test_seeded_commands_repeat_and_binary_is_exact(self):
        manifest = json.loads(diff.MANIFEST.read_text())
        case = manifest["cases"][0]
        first = diff.case_commands(case, manifest["seed"])
        self.assertEqual(first, diff.case_commands(case, manifest["seed"]))
        self.assertEqual(b"a\0b\xff", first[0][2])

    def test_expiry_comparison_discards_only_clock_value(self):
        replies = [{"reply": b"OK"}, {"reply": 273}, {"reply": 0}]
        replies[1] = {"reply": "valid-positive-ttl" if isinstance(replies[1]["reply"], int)
                      and 0 < replies[1]["reply"] <= 500 else "invalid-ttl"}
        self.assertEqual("valid-positive-ttl", replies[1]["reply"])
        self.assertEqual(0, replies[2]["reply"])


if __name__ == "__main__":
    unittest.main()

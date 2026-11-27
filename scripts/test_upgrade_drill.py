"""Offline tests for the exact-binary upgrade/rollback drill (A4); no servers run."""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("upgrade_drill", ROOT / "scripts/upgrade_drill.py")
ud = importlib.util.module_from_spec(spec)
sys.modules.setdefault("upgrade_drill", ud)
spec.loader.exec_module(ud)


class PinTests(unittest.TestCase):
    def pins(self, **overrides):
        base = {"schema": 1, "pair": {
            "id": "pair", "old": {"label": "old", "commit": "a" * 40},
            "new": {"label": "new", "ref": "HEAD"},
            "expectations": {"downgrade": "serves", "failed_upgrade": "preserved"}}}
        for key, value in overrides.items():
            base["pair"][key] = value
        return base

    def test_declared_pins_load_and_require_an_immutable_baseline(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "pins.json"
            path.write_text(json.dumps(self.pins()))
            self.assertEqual("pair", ud.load_pins(path)["pair"]["id"])
            for bad in (self.pins(old={"label": "x", "commit": "HEAD"}),
                        self.pins(old={"label": "x"}),
                        self.pins(expectations={"downgrade": "maybe", "failed_upgrade": "x"}),
                        self.pins(expectations={"downgrade": "serves"})):
                path.write_text(json.dumps(bad))
                with self.assertRaises(ud.rd.DrillError):
                    ud.load_pins(path)

    def test_repository_pins_are_versioned_and_pin_a_real_commit(self):
        pins = json.loads((ROOT / "ci/upgrade-pins.json").read_text())
        self.assertEqual(1, pins["schema"])
        self.assertRegex(pins["pair"]["old"]["commit"], r"^[0-9a-f]{40}$")
        self.assertEqual("HEAD", pins["pair"]["new"]["ref"])
        self.assertIn(pins["pair"]["expectations"]["downgrade"], ("serves", "refused"))


class ElementTests(unittest.TestCase):
    def test_element_records_exact_binary_digests(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "rillcache").write_bytes(b"server")
            (directory / "rillcache-backup").write_bytes(b"backup")
            record = ud.element(directory, "old")
        self.assertEqual("old", record["label"])
        self.assertEqual(len(record["server_sha256"]), 64)
        self.assertEqual(len(record["backup_sha256"]), 64)
        self.assertEqual(record["server_sha256"], ud.digest_bytes(b"server"))

    def test_missing_binaries_are_recorded_as_absent_digests(self):
        with tempfile.TemporaryDirectory() as temp:
            record = ud.element(Path(temp), "absent")
        self.assertIsNone(record["server_sha256"])
        self.assertIsNone(record["backup_sha256"])


class SummaryTests(unittest.TestCase):
    def test_summary_reports_the_pair_outcomes_and_non_claim(self):
        result = {
            "status": "passed",
            "pair": {"id": "pair", "old": {"label": "old", "commit": "a" * 40},
                     "new": {"label": "new", "ref": "HEAD"}},
            "upgrade": {"journal": {"bytes": 10}},
            "downgrade": {"outcome": "served", "expected": "serves"},
            "failed_upgrade": {"preserved_state_matches": True},
            "rollback": {"cutover": "stop, verify, install, validate"},
            "timings_ms": {"upgrade_startup": 1, "rollback_restore": 2, "total": 3},
            "failures": [],
        }
        text = ud.summary_text(result)
        self.assertIn("## Exact-binary upgrade/rollback drill: passed", text)
        self.assertIn("Downgrade outcome | served (declared serves)", text)
        self.assertIn("not a claim that arbitrary version skew is supported", text)

    def test_summary_keeps_a_failure_visible(self):
        text = ud.summary_text({"status": "failed", "pair": {"id": "p", "old": {}, "new": {}},
                                "upgrade": {"journal": {"bytes": 0}},
                                "downgrade": {}, "failed_upgrade": {}, "rollback": {},
                                "timings_ms": {}, "failures": [{"kind": "drill-error"}]})
        self.assertIn("## Exact-binary upgrade/rollback drill: failed", text)
        self.assertIn("- Failures: 1", text)


class FailureTests(unittest.TestCase):
    def test_missing_binaries_fail_closed_and_write_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "out"
            code = ud.main(["--old", str(Path(temp) / "old"), "--new", str(Path(temp) / "new"),
                            "--output", str(output), "--keys", "50"])
            self.assertEqual(1, code)
            result = json.loads((output / "result.json").read_text())
            self.assertEqual("failed", result["status"])
            self.assertEqual("drill-error", result["failures"][0]["kind"])
            self.assertIn("## Exact-binary upgrade/rollback drill: failed",
                          (output / "summary.md").read_text())

    def test_key_count_is_bounded(self):
        with self.assertRaises(SystemExit):
            ud.main(["--old", "/x", "--new", "/y", "--keys", "10"])


if __name__ == "__main__":
    unittest.main()

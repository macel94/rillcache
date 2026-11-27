"""Offline tests for the isolated backup/restore drill (gap D5); no server runs."""

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("restore_drill", ROOT / "scripts/restore_drill.py")
rd = importlib.util.module_from_spec(spec)
sys.modules.setdefault("restore_drill", rd)
spec.loader.exec_module(rd)


class DatasetTests(unittest.TestCase):
    def test_dataset_is_deterministic_and_covers_every_type(self):
        first = rd.dataset(459, 70, now=1_700_000_000_000)
        second = rd.dataset(459, 70, now=1_700_000_000_000)
        self.assertEqual(first[0], second[0])
        self.assertEqual(first[1], second[1])
        kinds = {expectation["type"] for expectation in first[1].values()}
        self.assertEqual({"string", "list", "hash", "set", "zset", "stream"}, kinds)
        expiring = [key for key, expectation in first[1].items() if expectation.get("expire")]
        self.assertTrue(expiring)
        # A different seed changes the sampled absolute expirations, not the shape.
        def expirations(seed):
            commands, _, _ = rd.dataset(seed, 70, now=1_700_000_000_000)
            return [step for step in commands if step[0] == "PEXPIREAT"]
        self.assertNotEqual(expirations(459), expirations(460))

    def test_dataset_uses_binary_and_empty_safe_payloads(self):
        commands, expectations, _ = rd.dataset(7, 7, now=1_700_000_000_000)
        strings = [step for step in commands if step[0] == "SET"]
        self.assertTrue(any(isinstance(step[2], bytes) and b"\x00" in step[2] for step in strings))
        self.assertTrue(any(b"other" in expectation.get("pairs", {}) for expectation in expectations.values()))


class ComparisonTests(unittest.TestCase):
    def state(self, **overrides):
        base = {"keys": {b"a": {"type": "string", "value": b"1", "pexpiretime": -1}},
                "db0": "# Keyspace\r\ndb0:keys=1,expires=0,avg_ttl=0,subexpiry=0\r\n",
                "payload": {"keys.count": 1, "dataset.bytes": 32},
                "observed_at_ms": 1_700_000_000_000}
        base.update(overrides)
        return base

    def test_identical_state_has_no_problems(self):
        self.assertEqual([], rd.compare(self.state(), self.state()))

    def test_value_type_expiry_and_accounting_differences_are_reported(self):
        changed_value = self.state(keys={b"a": {"type": "string", "value": b"2", "pexpiretime": -1}})
        self.assertEqual("value", rd.compare(self.state(), changed_value)[0]["kind"])

        changed_type = self.state(keys={b"a": {"type": "list", "value": [b"1"], "pexpiretime": -1}})
        self.assertEqual("value", rd.compare(self.state(), changed_type)[0]["kind"])

        changed_expiry = self.state(keys={b"a": {"type": "string", "value": b"1", "pexpiretime": 42}})
        self.assertIn("value", [problem["kind"] for problem in rd.compare(self.state(), changed_expiry)])

        missing = self.state(keys={b"b": {"type": "string", "value": b"1", "pexpiretime": -1}})
        self.assertEqual("keyset", rd.compare(self.state(), missing)[0]["kind"])

        extra = self.state(keys={b"a": {"type": "string", "value": b"1", "pexpiretime": -1},
                                 b"b": {"type": "string", "value": b"2", "pexpiretime": -1}})
        self.assertEqual("keyset", rd.compare(self.state(), extra)[0]["kind"])

        accounting = self.state(db0="# Keyspace\r\ndb0:keys=2,expires=0,avg_ttl=0,subexpiry=0\r\n")
        self.assertEqual("keyspace-accounting", rd.compare(self.state(), accounting)[0]["kind"])

        # avg_ttl is clock-derived: equal values pass, a drift beyond the
        # declared tolerance fails, and a value inside it is accepted.
        moved = self.state(db0="# Keyspace\r\ndb0:keys=1,expires=0,avg_ttl=900,subexpiry=0\r\n")
        self.assertEqual([], rd.compare(self.state(), moved, 5000))
        self.assertEqual("keyspace-avg-ttl", rd.compare(self.state(), moved)[0]["kind"])

        payload = self.state(payload={"keys.count": 1, "dataset.bytes": 33})
        self.assertEqual("payload-accounting", rd.compare(self.state(), payload)[0]["kind"])

    def test_db0_line_requires_the_keyspace_report(self):
        self.assertEqual("db0:keys=1,expires=0,avg_ttl=0,subexpiry=0",
                         rd.db0_line(self.state()["db0"]))
        with self.assertRaises(rd.DrillError):
            rd.db0_line("# Keyspace\r\n")


class FramingTests(unittest.TestCase):
    def test_reply_reader_handles_bulk_null_array_integer_and_error(self):
        self.assertEqual(b"value", rd.read_reply(io.BytesIO(b"$5\r\nvalue\r\n")))
        self.assertIsNone(rd.read_reply(io.BytesIO(b"$-1\r\n")))
        self.assertIsNone(rd.read_reply(io.BytesIO(b"*-1\r\n")))
        self.assertEqual(7, rd.read_reply(io.BytesIO(b":7\r\n")))
        self.assertEqual([b"a", b"b", None], rd.read_reply(io.BytesIO(b"*3\r\n$1\r\na\r\n$1\r\nb\r\n$-1\r\n")))
        with self.assertRaisesRegex(rd.DrillError, "ERR failure"):
            rd.read_reply(io.BytesIO(b"-ERR failure\r\n"))
        with self.assertRaises(rd.DrillError):
            rd.read_reply(io.BytesIO(b"$5\r\nvalue"))  # truncated payload

    def test_frame_encodes_binary_and_text_arguments(self):
        self.assertEqual(b"*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$2\r\n\x00\xff\r\n",
                         rd.frame(["SET", b"k", b"\x00\xff"]))


class FailureTests(unittest.TestCase):
    def test_missing_binaries_fail_closed_and_write_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "out"
            code = rd.main(["--server", "/nonexistent/rillcache",
                            "--backup", "/nonexistent/rillcache-backup",
                            "--output", str(output), "--keys", "50"])
            self.assertEqual(1, code)
            result = json.loads((output / "result.json").read_text())
            self.assertEqual("failed", result["status"])
            self.assertEqual("drill-error", result["failures"][0]["kind"])
            self.assertIn("## Isolated backup/restore drill: failed",
                          (output / "summary.md").read_text())

    def test_arguments_are_bounded(self):
        with self.assertRaises(SystemExit):
            rd.main(["--keys", "10"])
        with self.assertRaises(SystemExit):
            rd.main(["--retain", "0"])


if __name__ == "__main__":
    unittest.main()

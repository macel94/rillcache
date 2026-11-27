"""Offline tests for the pinned-native replication exclusion harness (A1)."""

import io
import unittest
from unittest.mock import patch

import replication_exclusion as rex


class ReplyTests(unittest.TestCase):
    def test_reply_reader_covers_the_replication_probes(self):
        self.assertEqual([b"master", 0, []],
                         rex.read_reply(io.BytesIO(b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n")))
        self.assertEqual({"error": "ERR replication is not supported by this standalone server"},
                         rex.read_reply(io.BytesIO(
                             b"-ERR replication is not supported by this standalone server\r\n")))
        self.assertEqual(0, rex.read_reply(io.BytesIO(b":0\r\n")))
        self.assertEqual(b"PONG", rex.read_reply(io.BytesIO(b"+PONG\r\n")))

    def test_info_replication_parses_fields_and_ignores_comments(self):
        text = (b"# Replication\r\nrole:slave\r\nmaster_link_status:down\r\n"
                b"connected_slaves:0\r\nmaster_repl_offset:0\r\n")
        with patch.object(rex, "call", return_value=text):
            fields = rex.info_replication(1)
        self.assertEqual("slave", fields["role"])
        self.assertEqual("down", fields["master_link_status"])
        self.assertNotIn("# Replication", fields)

    def test_info_replication_fails_closed_on_an_error_reply(self):
        with patch.object(rex, "call", return_value={"error": "ERR"}):
            with self.assertRaises(rex.HarnessError):
                rex.info_replication(1)


class EvidenceTests(unittest.TestCase):
    def result(self, **overrides):
        base = {"schema": 1, "status": "passed", "pins": {"redis": "8.10.2", "valkey": "9.1.2"},
                "references": {"redis": {"version": "8.10.2", "replica_role": "slave",
                                         "master_link_status": "down", "replica_dbsize": 0,
                                         "full_resync_sent": False,
                                         "candidate_connected_slaves": "0",
                                         "candidate_master_repl_offset": "0"}},
                "failures": []}
        base.update(overrides)
        return base

    def test_summary_reports_every_pin_and_the_exclusion_scope(self):
        text = rex.summary_text(self.result())
        self.assertIn("## Replication exclusion vs pinned replicas: passed", text)
        self.assertIn("| redis 8.10.2 | slave | down | 0 | False | 0 / 0 |", text)
        self.assertIn("exclusion evidence, not a replication implementation", text)

    def test_summary_keeps_failures_visible(self):
        text = rex.summary_text(self.result(status="failed",
                                           failures=[{"kind": "link-up", "reference": "valkey"}]))
        self.assertIn("## Replication exclusion vs pinned replicas: failed", text)
        self.assertIn("- failures: 1", text)

    def test_evidence_renders_bytes_without_losing_them(self):
        self.assertEqual(["master", 0, []], rex._evidence([b"master", 0, []]))
        self.assertEqual("down", rex._evidence(b"down"))


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
import json
import tempfile
import unittest
from pathlib import Path

import rillcache_pi_wake as wake


class WakeDecisionTests(unittest.TestCase):
    def test_pane_metadata_supports_explicit_shell_command(self):
        self.assertEqual(wake.expected_pane_meta("agent", 123, "sh"), "agent|123|sh|0|0")
        self.assertEqual(wake.expected_pane_meta("agent", 123, "pi"), "agent|123|pi|0|0")

    def test_pinned_pi_must_belong_to_expected_pane(self):
        self.assertTrue(wake.pane_owns_pi(10, 11, "11 12"))
        self.assertFalse(wake.pane_owns_pi(10, 11, "12 13"))
        self.assertTrue(wake.pane_owns_pi(10, 10, ""))

    def test_continue_mode_always_wakes(self):
        self.assertTrue(wake.should_wake("continue", None, "", "p", 100)[0])
        self.assertTrue(wake.should_wake("continue", {"fingerprint": "same", "prompt_hash": "p", "last_wake": 99}, "same", "p", 100)[0])

    def test_wait_without_state_wakes_once(self):
        should, reason = wake.should_wake("waiting", None, "fp", "prompt", 100)
        self.assertTrue(should)
        self.assertEqual(reason, "no prior wait state")

    def test_unchanged_wait_is_suppressed_until_heartbeat(self):
        state = {"fingerprint": "fp", "prompt_hash": "prompt", "last_wake": 100}
        should, reason = wake.should_wake("waiting", state, "fp", "prompt", 100 + 3600)
        self.assertFalse(should)
        self.assertIn("suppressing", reason)
        self.assertTrue(wake.should_wake("waiting", state, "fp", "prompt", 100 + 86400)[0])

    def test_external_change_is_coalesced_but_prompt_change_is_immediate(self):
        state = {"fingerprint": "old", "prompt_hash": "prompt", "last_wake": 100}
        should, reason = wake.should_wake("waiting", state, "new", "prompt", 101)
        self.assertFalse(should)
        self.assertIn("coalescing", reason)
        self.assertTrue(wake.should_wake("waiting", state, "new", "prompt", 100 + 3600)[0])
        self.assertTrue(wake.should_wake("waiting", state, "old", "new-prompt", 101)[0])

    def test_waiting_marker_is_required_to_suppress(self):
        self.assertEqual(wake.waiting_mode("blocked\n" + wake.WAITING_MARKER), "waiting")
        self.assertEqual(wake.waiting_mode("progress\n" + wake.CONTINUE_MARKER), "continue")
        self.assertEqual(wake.waiting_mode("unmarked final response"), "continue")

    def test_versioned_prompt_requires_wait_and_continue_markers(self):
        prompt = Path(__file__).with_name("production-readiness-loop-prompt.md").read_text()
        self.assertIn(wake.WAITING_MARKER, prompt)
        self.assertIn(wake.CONTINUE_MARKER, prompt)
        self.assertIn("NOT PRODUCTION READY", prompt)

    def test_state_round_trips_atomically(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "state.json"
            wake.write_state(path, "fingerprint", "prompt", 123.5)
            self.assertEqual(wake.read_state(path), {
                "version": 1,
                "fingerprint": "fingerprint",
                "prompt_hash": "prompt",
                "last_wake": 123.5,
            })
            self.assertFalse(path.with_suffix(".json.tmp").exists())

    def test_fingerprint_covers_main_pr_and_issue_state(self):
        replies = iter([
            "mainsha",
            json.dumps([{"number": 2, "headRefOid": "h", "updatedAt": "t"}]),
            json.dumps([{"number": 5, "updatedAt": "t", "assignees": [], "labels": [], "state": "OPEN"}]),
        ])
        calls = []

        def fake_runner(*args):
            calls.append(args)
            return next(replies)

        digest = wake.external_fingerprint(fake_runner)
        self.assertEqual(len(digest), 64)
        self.assertEqual(len(calls), 3)
        self.assertTrue(any("commits/main" in arg for arg in calls[0]))

    def test_normalization_ignores_unimportant_api_array_order(self):
        first = {"labels": [{"name": "b"}, {"name": "a"}], "number": 1}
        second = {"number": 1, "labels": [{"name": "a"}, {"name": "b"}]}
        self.assertEqual(wake._normalize(first), wake._normalize(second))


if __name__ == "__main__":
    unittest.main()

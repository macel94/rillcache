"""Offline mocked state-machine and safety tests for pr-delivery.py."""
import contextlib
import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from types import SimpleNamespace
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("pr_delivery", Path(__file__).with_name("pr-delivery.py"))
mod = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = mod
SPEC.loader.exec_module(mod)
A, B, C = "a" * 40, "b" * 40, "c" * 40


def pull(head=B, base=A, *, draft=False, fork=False):
    repo = "elsewhere/fork" if fork else mod.OWNER
    return {"number": 42, "state": "open", "draft": draft,
            "base": {"ref": "main", "sha": base, "repo": {"full_name": mod.OWNER}},
            "head": {"ref": "feat/example", "sha": head, "repo": {"full_name": repo}}}


class FakeApi:
    def __init__(self):
        self.pr = pull()
        self.main = A
        self.posts = []
        self.verbs = []
        self.compare = "behind"
        self.review_data = []
        self.run_data = {"ci.yml": [], "pr-quality.yml": []}
        self.check_data = {"check_runs": []}

    def get(self, path):
        if path.endswith("pulls?state=open&base=main&per_page=100"):
            return [self.pr]
        if path.endswith("pulls/42"):
            return dict(self.pr, auto_merge={"enabled_by": {"login": "owner"}})
        if path.endswith("pulls/42/requested_reviewers"):
            return {"users": [], "teams": []}
        if path.endswith("pulls/42/reviews?per_page=100"):
            return self.review_data
        if path.endswith("git/ref/heads/main"):
            return {"object": {"sha": self.main}}
        if "/compare/" in path:
            return {"status": self.compare}
        if "/actions/runs/" in path:
            run_id = path.rsplit("/", 1)[1]
            return next((r for rs in self.run_data.values() for r in rs if str(r.get("id")) == run_id), None)
        if "/actions/workflows/ci.yml/runs" in path:
            return {"workflow_runs": self.run_data["ci.yml"]}
        if "/actions/workflows/pr-quality.yml/runs" in path:
            return {"workflow_runs": self.run_data["pr-quality.yml"]}
        if "/check-runs" in path:
            return self.check_data
        if "/actions/workflows/" in path and "branch=" in path:
            return {"workflow_runs": []}
        raise AssertionError(path)

    def post(self, path, data):
        self.verbs.append("POST")
        self.posts.append((path, data))
        return None

    def put(self, path, data):
        self.verbs.append("PUT")
        self.posts.append((path, data))
        return None


class ControllerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.local = mod.Local(Path(self.tmp.name))
        self.api = FakeApi()
        self.controller = mod.Controller(self.api, self.local)
        self.controller.verify_repo = lambda: None

    def tearDown(self):
        self.tmp.cleanup()

    def test_default_cli_is_read_only_and_creates_no_state(self):
        api = FakeApi()
        outside = Path(self.tmp.name) / "state-home"
        def command(args, **kwargs):
            if args[:2] == ["git", "rev-parse"]:
                return SimpleNamespace(stdout=str(Path(__file__).resolve().parents[1]), stderr="", returncode=0)
            if args[:3] == ["gh", "repo", "view"]:
                return SimpleNamespace(stdout=json.dumps({"nameWithOwner": mod.OWNER}), stderr="", returncode=0)
            raise AssertionError(args)
        output = io.StringIO()
        with patch.object(mod, "Gh", return_value=api), patch.object(mod.subprocess, "run", side_effect=command), contextlib.redirect_stdout(output):
            self.assertEqual(mod.main(["--state-dir", str(outside)]), 0)
        self.assertFalse(outside.exists())
        self.assertFalse(api.posts)
        self.assertIn("Read-only mode", output.getvalue())

    def test_cli_rejects_unsafe_watch_enqueue_and_invalid_pr_numbers(self):
        for argv in (["--apply", "--enqueue", "--pr", "42", "--watch"], ["--pr", "0"]):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as err:
                mod.main(list(argv))
            self.assertEqual(err.exception.code, 2)

    def test_identity_excludes_draft_fork_closed_and_wrong_base(self):
        self.assertTrue(mod.Controller.valid_identity(pull()))
        for p in (pull(draft=True), pull(fork=True), dict(pull(), state="closed"),
                  {**pull(), "base": {**pull()["base"], "ref": "other"}}):
            self.assertFalse(mod.Controller.valid_identity(p))

    def test_stale_is_ancestry_not_sha_equality(self):
        self.assertTrue(self.controller.stale(pull(head=B, base=A)))
        self.api.compare = "ahead"
        self.assertFalse(self.controller.stale(pull(head=B, base=A)))
        self.api.compare = "diverged"
        self.assertTrue(self.controller.stale(pull(head=B, base=A)))

    def test_async_update_once_then_race_pauses_without_clobber(self):
        first = self.controller.apply()
        self.assertIn("accepted asynchronously", first)
        updates = [p for p in self.api.posts if p[0].endswith("update-branch")]
        self.assertEqual(len(updates), 1)
        self.assertEqual(updates[0][1], {"expected_head_sha": B})
        self.assertEqual(self.api.verbs, ["PUT"])
        state = self.local.load()
        update = next(iter(state["attempts"].values()))
        self.assertEqual(update["status"], "updated_pending")
        self.assertEqual(self.controller.apply(), f"PR #42 update remains pending for expected head {B}; no repeat update issued")
        self.assertEqual(len([p for p in self.api.posts if p[0].endswith("update-branch")]), 1)
        self.api.pr = pull(head=C, base=A)
        self.api.main = A
        self.controller.apply()
        self.assertEqual(self.api.posts[-1][0].split("/")[-1], "dispatches")
        self.assertEqual(self.api.posts[-1][1]["ref"], "feat/example")
        attempts = self.local.load()["attempts"]
        self.assertTrue(any(a.get("head") == C and a.get("status") == "checks" for a in attempts.values()))

    def test_review_blocked_pr_does_not_block_later_independent_candidate(self):
        blocked, eligible = pull(), pull(head=C)
        eligible["number"] = 43
        eligible["head"]["sha"] = "d" * 40
        self.controller.list_prs = lambda: [blocked, eligible]
        self.controller.pull = lambda number: blocked if number == 42 else eligible
        self.controller.opted_in = lambda number: True
        self.controller.review_blocker = lambda number: "requested reviews pending: reviewer" if number == 42 else None
        self.controller.stale = lambda pr: True
        result = self.controller.apply()
        self.assertIn("PR #43 update accepted", result)
        self.assertEqual(self.api.posts[0][0], f"repos/{mod.OWNER}/pulls/43/update-branch")

    def test_head_race_before_update_does_not_mutate(self):
        original = self.controller.pull
        count = 0
        def raced(number):
            nonlocal count
            count += 1
            if count >= 2:
                return pull(head=C)
            return original(number)
        self.controller.pull = raced
        result = self.controller.apply()
        self.assertIn("raced before update", result)
        self.assertFalse(self.api.posts)

    def test_uncertain_update_restart_pauses_without_duplicate_mutation(self):
        pr = pull()
        ident = mod.key(pr, A, B)
        self.local.save({"version": 1, "attempts": {ident: {"number": 42, "base": A, "head": B,
                           "status": "updating"}}, "active": None, "events": []})
        result = self.controller.apply()
        self.assertIn("outcome is uncertain", result)
        self.assertEqual(self.local.load()["attempts"][ident]["status"], "paused")
        self.assertFalse(self.api.posts)

    def test_active_oauth_pr_workflow_is_adopted_not_redispatched(self):
        pr = pull(head=C)
        candidate = {"number": 42, "base": A, "head": C, "status": "checks"}
        self.api.run_data["ci.yml"] = [{"id": 77, "head_sha": "d" * 40, "event": "pull_request",
            "path": ".github/workflows/ci.yml@refs/pull/42/merge", "status": "in_progress",
            "pull_requests": [{"head": {"sha": C}, "base": {"sha": A}}]}]
        self.controller._state = {"version": 1, "attempts": {}, "active": None, "events": []}
        self.controller.dispatch(pr, candidate)
        self.assertEqual(candidate["ci_run_id"], "77")
        dispatches = [p for p in self.api.posts if p[0].endswith("/dispatches")]
        self.assertEqual(len(dispatches), 1)
        self.assertIn("pr-quality.yml/dispatches", dispatches[0][0])

    def test_main_advancement_pauses_pending_update(self):
        self.controller.apply()
        self.api.main = C
        result = self.controller.apply()
        self.assertIn("main/base advanced", result)
        attempt = next(iter(self.local.load()["attempts"].values()))
        self.assertEqual(attempt["status"], "paused")

    def test_revoked_opt_in_and_change_request_exclude_pr(self):
        self.api.get = lambda path: [] if path.endswith("reviews?per_page=100") else (
            {"number": 42, "auto_merge": None} if path.endswith("pulls/42") else
            [self.api.pr] if path.endswith("per_page=100") else self.api.get(path))
        self.assertIn("no eligible", self.controller.apply())
        self.assertFalse(self.api.posts)
        self.api = FakeApi()
        self.api.review_data = [{"user": {"login": "reviewer"}, "state": "CHANGES_REQUESTED"}]
        self.controller = mod.Controller(self.api, self.local)
        self.controller.verify_repo = lambda: None
        self.assertIn("review", self.controller.apply())
        self.assertFalse(self.api.posts)

    def test_failed_gate_is_not_retried(self):
        pr = pull(head=C)
        self.api.pr = pr
        ident = mod.key(pr, A, C)
        self.local.save({"version": 1, "attempts": {ident: {"number": 42, "base": A, "head": C,
                            "status": "failed", "reason": "failed check"}}, "active": None, "events": []})
        self.assertIn("no eligible", self.controller.apply())
        self.assertFalse(self.api.posts)

    def test_check_validation_rejects_wrong_origin_sha_and_pending(self):
        pr = pull(head=C)
        attempt = {"ci_run_id": "1", "gate_run_id": "2"}
        self.api.run_data = {
            "ci.yml": [{"id": 1, "head_sha": C, "event": "workflow_dispatch", "path": ".github/workflows/ci.yml@refs/heads/feat/example", "check_suite_id": 11, "status": "in_progress"}],
            "pr-quality.yml": [{"id": 2, "head_sha": C, "event": "workflow_dispatch", "path": ".github/workflows/pr-quality.yml@refs/heads/feat/example", "check_suite_id": 22, "status": "completed", "conclusion": "success"}]}
        self.assertFalse(self.controller.checks(pr, attempt)[0])
        self.api.run_data["ci.yml"][0].update(status="completed", conclusion="success")
        self.api.check_data = {"check_runs": [{"name": "Tests and offline CI", "head_sha": C,
                          "app": {"slug": "forged"}, "conclusion": "success"}]}
        self.assertFalse(self.controller.checks(pr, attempt)[0])
        self.api.check_data["check_runs"][0]["app"]["slug"] = "github-actions"
        self.api.check_data["check_runs"].append({"name": "Valkey performance gate", "head_sha": B,
                          "app": {"slug": "github-actions"}, "conclusion": "success"})
        self.assertFalse(self.controller.checks(pr, attempt)[0])

    def test_success_requires_both_exact_actions_check_suites_and_holds_serial_lane(self):
        self.api.pr = pull(head=C)
        ident = mod.key(self.api.pr, A, C)
        attempt = {"number": 42, "base": A, "head": C, "status": "checks",
                   "ci_run_id": "11", "gate_run_id": "22"}
        self.api.run_data = {
            "ci.yml": [{"id": 11, "head_sha": C, "event": "workflow_dispatch", "path": ".github/workflows/ci.yml@refs/heads/feat/example", "check_suite_id": 101, "status": "completed", "conclusion": "success"}],
            "pr-quality.yml": [{"id": 22, "head_sha": C, "event": "workflow_dispatch", "path": ".github/workflows/pr-quality.yml@refs/heads/feat/example", "check_suite_id": 202, "status": "completed", "conclusion": "success"}]}
        self.api.check_data = {"check_runs": [
            {"name": "Tests and offline CI", "head_sha": C, "check_suite": {"id": 101}, "app": {"slug": "github-actions", "id": 15368}, "conclusion": "success"},
            {"name": "Valkey performance gate", "head_sha": C, "check_suite": {"id": 202}, "app": {"slug": "github-actions", "id": 15368}, "conclusion": "success"}]}
        state = {"version": 1, "attempts": {ident: attempt}, "active": None, "events": []}
        self.local.save(state)
        self.assertIn("both fresh required checks passed", self.controller.apply())
        self.assertEqual(self.local.load()["attempts"][ident]["status"], "checks_passed")
        self.assertIn("awaiting its protected squash auto-merge", self.controller.apply())
        self.assertEqual(self.local.load()["attempts"][ident]["status"], "checks_passed")
        self.api.pr["merged"] = True
        self.api.pr["state"] = "closed"
        self.controller.apply()
        self.assertEqual(self.local.load()["attempts"][ident]["status"], "merged")

    def test_failed_valkey_gate_is_not_a_pass_or_retried(self):
        self.api.pr = pull(head=C)
        attempt = {"ci_run_id": "11", "gate_run_id": "22", "base": A}
        self.api.run_data = {
            "ci.yml": [{"id": 11, "head_sha": C, "event": "workflow_dispatch", "path": ".github/workflows/ci.yml@refs/heads/feat/example", "check_suite_id": 101, "status": "completed", "conclusion": "success"}],
            "pr-quality.yml": [{"id": 22, "head_sha": C, "event": "workflow_dispatch", "path": ".github/workflows/pr-quality.yml@refs/heads/feat/example", "check_suite_id": 202, "status": "completed", "conclusion": "failure"}]}
        self.api.check_data = {"check_runs": [{"name": "Tests and offline CI", "head_sha": C, "check_suite": {"id": 101}, "app": {"slug": "github-actions", "id": 15368}, "conclusion": "success"}]}
        self.assertFalse(self.controller.checks(self.api.pr, attempt)[0])
        self.assertIn("owner fix required", self.controller.checks(self.api.pr, attempt)[1])

    def test_state_is_private_and_corruption_fails_closed(self):
        self.local.save({"version": 1, "attempts": {}, "active": None, "events": []})
        self.assertEqual(os.stat(self.local.state_dir).st_mode & 0o777, 0o700)
        self.assertEqual(os.stat(self.local.state_path).st_mode & 0o777, 0o600)
        self.local.state_path.write_text("bad")
        with self.assertRaises(mod.DeliveryError):
            self.local.load()

    def test_symlinked_state_paths_fail_closed(self):
        external = Path(self.tmp.name) / "external"
        external.mkdir()
        external_state = external / "controller.json"
        external_state.write_text(json.dumps({"version": 1, "attempts": {}}))
        self.local.state_dir.mkdir()
        self.local.state_path.symlink_to(external_state)
        with self.assertRaises(mod.DeliveryError):
            self.local.load()

        link_root = Path(self.tmp.name) / "linked-root"
        link_root.mkdir()
        (link_root / "state").symlink_to(external, target_is_directory=True)
        with self.assertRaises(mod.DeliveryError):
            mod.Local(link_root).load()

    def test_read_only_local_creation_has_no_filesystem_side_effect(self):
        other = mod.Local(Path(self.tmp.name) / "unused")
        self.assertFalse(other.state_dir.exists())
        self.assertEqual(other.load()["attempts"], {})
        self.assertFalse(other.state_dir.exists())

    def test_exclusive_controller_lease(self):
        fd = self.local.lock()
        try:
            with self.assertRaises(mod.DeliveryError):
                self.local.lock()
        finally:
            os.close(fd)
        fd2 = self.local.lock()
        os.close(fd2)

    @patch.object(mod.subprocess, "run")
    def test_gh_adapter_uses_json_input_and_redacts_stderr(self, run):
        run.return_value.returncode = 1
        run.return_value.stderr = "secret-token"
        api = mod.Gh()
        with self.assertRaises(mod.DeliveryError) as err:
            api.post("repos/x/y/example", {"expected_head_sha": A})
        self.assertNotIn("secret-token", str(err.exception))
        args = run.call_args.args[0]
        self.assertIn("--input", args)
        self.assertEqual(run.call_args.kwargs["input"], json.dumps({"expected_head_sha": A}))
        with self.assertRaises(mod.DeliveryError):
            api.put("repos/x/y/pulls/1/update-branch", {"expected_head_sha": A})
        args = run.call_args.args[0]
        self.assertEqual(args[args.index("--method") + 1], "PUT")


if __name__ == "__main__":
    unittest.main()

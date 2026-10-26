"""Offline tests for the hosted updater's API state machine and trust boundary."""
from __future__ import annotations

import importlib.util
import io
import json
import os
import urllib.error
import urllib.parse
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("pr_branch_updater", ROOT / "scripts/update-pr-branches.py")
mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mod)
A, B, C, D = (c * 40 for c in "abcd")


def pull(number=1, *, base=A, head=B, draft=False, fork=False, opted=True):
    repo = "someone/fork" if fork else mod.REPO
    return {
        "number": number, "state": "open", "draft": draft,
        "html_url": f"https://github.com/{mod.REPO}/pull/{number}",
        "auto_merge": {"enabled_by": {"login": "owner"}} if opted else None,
        "base": {"ref": "main", "sha": base, "repo": {"full_name": mod.REPO}},
        "head": {"ref": f"feature/{number}", "sha": head, "repo": {"full_name": repo}},
    }


class FakeAPI:
    def __init__(self, prs=None):
        self.prs = {p["number"]: p for p in (prs or [pull()])}
        self.main = A
        self.current = set()
        self.workflows = {name: [] for name in mod.REQUIRED}
        self.checks = {}
        self.calls = []
        self.update_result = "update"
        self.update_error = None
        self.dispatch_error = None
        # (workflow, ref) -> blob sha override; default means branch matches main.
        self.workflow_blobs = {}

    def pages(self, path):
        output = []
        for page in range(1, mod.MAX_PAGES + 1):
            separator = "&" if "?" in path else "?"
            values = self.call(f"{path}{separator}per_page={mod.PAGE_SIZE}&page={page}")
            output.extend(values)
            if len(values) < mod.PAGE_SIZE:
                return output
        raise mod.ApiError(None, "page bound")

    def call(self, path, *, method="GET", body=None):
        self.calls.append((method, path, body))
        if path == f"repos/{mod.REPO}":
            return {"full_name": mod.REPO, "default_branch": "main"}
        if path.startswith(f"repos/{mod.REPO}/pulls?"):
            return [self.prs[n] for n in sorted(self.prs)]
        if path.startswith(f"repos/{mod.REPO}/pulls/") and path.endswith("/update-branch"):
            if self.update_error:
                raise self.update_error
            number = int(path.split("/")[-2])
            if body != {"expected_head_sha": self.prs[number]["head"]["sha"]}:
                raise AssertionError("branch update lacked its frozen expected head")
            if self.update_result == "update":
                self.prs[number]["head"]["sha"] = C
                self.prs[number]["base"]["sha"] = self.main
                self.current.add((self.main, C))
            return {"status": "pending", "details": {"uuid": "test-request"}}
        if path.startswith(f"repos/{mod.REPO}/pulls/"):
            number = int(path.rsplit("/", 1)[1])
            return self.prs[number]
        if "/contents/.github/workflows/" in path:
            tail = path.split("/contents/", 1)[1]
            file, _, query = tail.partition("?")
            ref = urllib.parse.unquote(query.split("ref=", 1)[1]) if "ref=" in query else "main"
            workflow = file.rsplit("/", 1)[1]
            return {"sha": self.workflow_blobs.get((workflow, ref), "blob-" + workflow)}
        if path == f"repos/{mod.REPO}/git/ref/heads/main":
            return {"object": {"sha": self.main}}
        if "/compare/" in path:
            pair = path.split("/compare/", 1)[1]
            base, head = pair.split("...", 1)
            return {"status": "ahead" if (base, head) in self.current else "behind"}
        if "/actions/workflows/" in path and "/runs?branch=" in path:
            workflow = path.split("/workflows/", 1)[1].split("/runs", 1)[0]
            return {"workflow_runs": self.workflows[workflow]}
        if "/actions/workflows/" in path and path.endswith("/dispatches"):
            if self.dispatch_error:
                raise self.dispatch_error
            return None
        if "/check-runs?per_page=100" in path:
            sha = path.split("/commits/", 1)[1].split("/check-runs", 1)[0]
            return {"check_runs": self.checks.get(sha, [])}
        raise AssertionError(f"unhandled API call: {method} {path} {body}")


class UpdaterTests(unittest.TestCase):
    def setUp(self):
        self.api = FakeAPI()
        self.updater = mod.Updater(self.api, sleep=lambda _: None, update_wait=0)

    def test_eligibility_covers_every_open_same_repo_main_pr(self):
        # The user asked for every open PR branch to stay current: draft state
        # and auto-merge are not eligibility gates.
        cases = [
            (pull(), True),
            (pull(draft=True), True),
            (pull(opted=False), True),
            (pull(fork=True), False),
            ({**pull(), "state": "closed"}, False),
            ({**pull(), "base": {**pull()["base"], "ref": "release"}}, False),
            ({**pull(), "head": {**pull()["head"], "sha": "not-a-sha"}}, False),
        ]
        for candidate, expected in cases:
            with self.subTest(candidate=candidate):
                self.assertEqual(self.updater.eligible(candidate)[0], expected)

    def test_repository_mismatch_fails_closed(self):
        self.api.call = lambda *a, **k: {"full_name": "fork/repo", "default_branch": "main"}
        with self.assertRaises(mod.ApiError):
            self.updater.repo()

    def test_http_adapter_pins_host_redacts_error_body_and_rejects_unsafe_paths(self):
        observed = {}
        class Response:
            status = 204
            def __enter__(self): return self
            def __exit__(self, *args): return False
            def read(self, _limit): return b""
        def opener(request, timeout):
            observed.update(url=request.full_url, method=request.method,
                           authorization=request.get_header("Authorization"), timeout=timeout)
            return Response()
        api = mod.GitHub("top-secret", opener=opener, timeout=7)
        self.assertIsNone(api.call("repos/macel94/rillcache/actions/workflows/ci.yml/dispatches",
                                   method="POST", body={"ref": "main"}))
        self.assertEqual(observed["url"], "https://api.github.com/repos/macel94/rillcache/actions/workflows/ci.yml/dispatches")
        self.assertEqual(observed["method"], "POST")
        self.assertEqual(observed["authorization"], "Bearer top-secret")
        self.assertEqual(observed["timeout"], 7)
        with self.assertRaises(mod.ApiError):
            api.call("repos/../secret")
        def rejected(_request, **_kwargs):
            raise urllib.error.HTTPError("https://api.github.com", 403, "denied", {}, io.BytesIO(b"secret-body"))
        with self.assertRaises(mod.ApiError) as failure:
            mod.GitHub("top-secret", opener=rejected).call("repos/macel94/rillcache")
        self.assertNotIn("secret-body", str(failure.exception))
        self.assertNotIn("top-secret", str(failure.exception))

    def test_guarded_update_uses_expected_sha_then_explicitly_dispatches_both_workflows(self):
        outcome = self.updater.process(pull())
        self.assertEqual(outcome["status"], "checks_dispatched")
        updates = [c for c in self.api.calls if c[0] == "PUT"]
        self.assertEqual(len(updates), 1)
        self.assertTrue(updates[0][1].endswith("/update-branch"))
        self.assertEqual(updates[0][2], {"expected_head_sha": B})
        dispatched = [c for c in self.api.calls if c[0] == "POST"]
        self.assertEqual([c[1].rsplit("/", 2)[-2] for c in dispatched], ["ci.yml", "pr-quality.yml"])
        self.assertEqual(dispatched[0][2], {"ref": "feature/1"})
        self.assertEqual(dispatched[1][2], {"ref": "feature/1", "inputs": {"pull_request": "1"}})

    def test_pr_that_stops_being_eligible_during_update_stops_required_dispatch(self):
        original = self.api.call
        def close(path, **kwargs):
            result = original(path, **kwargs)
            if path.endswith("/update-branch"):
                self.api.prs[1]["state"] = "closed"
            return result
        self.api.call = close
        outcome = self.updater.process(pull())
        self.assertEqual(outcome["status"], "consent_revoked")
        self.assertFalse(any(call[0] == "POST" for call in self.api.calls))

    def test_staleness_uses_current_main_not_the_pr_base_snapshot(self):
        # Regression: GitHub reports base.sha as the PR's last-synced base
        # snapshot (A) while main has advanced to D. The head does not contain
        # main, so it must be refreshed instead of reported as base_race.
        self.api.main = D
        p = pull(base=A, head=B)
        self.api.prs[1] = p
        outcome = self.updater.process(p)
        self.assertEqual(outcome["status"], "checks_dispatched")
        self.assertEqual(outcome["base"], D)
        puts = [c for c in self.api.calls if c[0] == "PUT"]
        self.assertEqual(len(puts), 1)
        self.assertEqual(puts[0][2], {"expected_head_sha": B})

    def test_draft_branches_are_refreshed_without_running_required_checks(self):
        p = pull(draft=True)
        self.api.prs[1] = p
        outcome = self.updater.process(p)
        self.assertEqual(outcome["status"], "draft_updated")
        self.assertTrue(any(c[0] == "PUT" for c in self.api.calls))
        self.assertFalse(any(c[0] == "POST" for c in self.api.calls))

    def test_changed_required_workflow_on_branch_blocks_privileged_dispatch(self):
        p = pull(base=A, head=C)
        self.api.prs[1] = p
        self.api.current.add((A, C))
        self.api.workflow_blobs[("ci.yml", "feature/1")] = "attacker-controlled"
        outcome = self.updater.process(p)
        self.assertEqual(outcome["status"], "untrusted_workflow")
        self.assertFalse(any(c[0] == "POST" for c in self.api.calls))

    def test_failed_evidence_survives_a_main_advance_for_the_same_head(self):
        p = pull(base=A, head=B)
        self.api.prs[1] = p
        self.api.workflows["ci.yml"] = [{
            "id": 9, "path": ".github/workflows/ci.yml@refs/pull/1/merge",
            "event": "pull_request", "status": "completed", "conclusion": "failure",
            "created_at": "2026-10-26T00:00:00Z",
            "pull_requests": [{"number": 1, "head": {"sha": B}, "base": {"sha": A}}],
        }]
        self.api.main = D
        result = self.updater.process(pull(base=A, head=B))
        self.assertEqual(result["status"], "failed_gate")
        self.assertFalse(any(c[0] == "PUT" for c in self.api.calls))

    def test_main_head_race_conflict_and_pending_update_are_visible(self):
        self.api.update_error = mod.ApiError(422, "conflict")
        self.assertEqual(self.updater.process(pull())["status"], "conflict_or_race")
        self.api.update_error = None
        self.api.update_result = "pending"
        self.assertEqual(self.updater.process(pull())["status"], "update_pending")
        self.assertFalse(any(c[0] == "POST" for c in self.api.calls))

    def test_base_advance_during_update_is_not_accepted(self):
        original = self.api.call
        def raced(path, **kwargs):
            result = original(path, **kwargs)
            if kwargs.get("method") == "PUT":
                self.api.main = D
            return result
        self.api.call = raced
        result = self.updater.process(pull())
        self.assertEqual(result["status"], "identity_race")
        self.assertFalse(any(c[0] == "POST" for c in self.api.calls))

    def test_failed_or_active_candidate_checks_prevent_base_update(self):
        p = pull()
        self.api.workflows["ci.yml"] = [{
            "id": 1, "path": ".github/workflows/ci.yml@refs/pull/1/merge", "event": "pull_request",
            "status": "completed", "conclusion": "failure", "created_at": "2026-10-26T00:00:00Z",
            "pull_requests": [{"number": 1, "head": {"sha": B}, "base": {"sha": A}}],
        }]
        result = self.updater.process(p)
        self.assertEqual(result["status"], "failed_gate")
        self.assertFalse(any(c[0] == "PUT" for c in self.api.calls))
        self.api.workflows["ci.yml"][0].update(status="in_progress", conclusion=None)
        self.assertEqual(self.updater.process(p)["status"], "waiting_existing_check")

    def test_exact_successful_runs_require_real_workflow_and_check_identity(self):
        p = pull(base=A, head=C)
        self.api.prs[1] = p
        self.api.current.add((A, C))
        for index, workflow in enumerate(mod.REQUIRED):
            run_id = index + 1
            check_sha = C
            self.api.workflows[workflow].append({
                "id": run_id, "path": f".github/workflows/{workflow}@refs/heads/feature/1",
                "event": "workflow_dispatch", "head_sha": C, "head_branch": "feature/1",
                "status": "completed", "conclusion": "success", "check_suite_id": 100 + index,
                "created_at": "2026-10-26T00:00:00Z",
            })
            self.api.checks[check_sha] = self.api.checks.get(check_sha, []) + [{
                "name": mod.REQUIRED[workflow], "head_sha": check_sha,
                "check_suite": {"id": 100 + index},
                "app": {"slug": "github-actions", "id": mod.ACTIONS_APP_ID},
                "conclusion": "success",
            }]
        result = self.updater.process(p)
        self.assertEqual(result["status"], "checks_passed")
        self.assertFalse(any(c[0] in {"PUT", "POST"} for c in self.api.calls))

    def test_github_token_approval_held_pr_runs_do_not_block_explicit_dispatch(self):
        p = pull(base=A, head=C)
        self.api.prs[1] = p
        self.api.current.add((A, C))
        for workflow in mod.REQUIRED:
            self.api.workflows[workflow].append({
                "id": 70, "path": f".github/workflows/{workflow}@refs/pull/1/merge",
                "event": "pull_request", "head_sha": D, "status": "completed",
                "conclusion": "action_required", "actor": {"login": "github-actions[bot]"},
                "pull_requests": [{"number": 1, "head": {"sha": C}, "base": {"sha": A}}],
            })
        result = self.updater.process(p)
        self.assertEqual(result["status"], "checks_dispatched")
        dispatches = [call for call in self.api.calls if call[0] == "POST"]
        self.assertEqual(len(dispatches), 2)

    def test_wrong_sha_path_app_or_suite_never_counts_as_success(self):
        p = pull(base=A, head=C)
        self.api.prs[1] = p
        self.api.current.add((A, C))
        self.api.workflows["ci.yml"].append({
            "id": 1, "path": ".github/workflows/other.yml@refs/heads/feature/1",
            "event": "workflow_dispatch", "head_sha": C, "head_branch": "feature/1",
            "status": "completed", "conclusion": "success", "check_suite_id": 5,
        })
        self.assertEqual(self.updater.process(p)["status"], "checks_dispatched")

    def test_every_open_same_repo_main_pr_is_reconciled(self):
        self.api = FakeAPI([pull(2, head=B), pull(1, head=B, opted=False), pull(3, head=B, draft=True)])
        self.updater = mod.Updater(self.api, sleep=lambda _: None, update_wait=0)
        outcomes = self.updater.reconcile()
        self.assertEqual([x["number"] for x in outcomes], [1, 2, 3])
        self.assertEqual(
            [x["status"] for x in outcomes],
            ["checks_dispatched", "checks_dispatched", "draft_updated"],
        )
        self.assertEqual(sum(c[0] == "PUT" for c in self.api.calls), 3)

    def test_workflow_failure_is_not_retried_and_new_source_head_is_a_new_identity(self):
        p = pull(base=A, head=C)
        self.api.prs[1] = p
        self.api.current.add((A, C))
        self.api.workflows["ci.yml"].append({
            "id": 1, "path": ".github/workflows/ci.yml@refs/heads/feature/1", "event": "workflow_dispatch",
            "head_sha": C, "head_branch": "feature/1", "status": "completed", "conclusion": "failure",
            "check_suite_id": 5, "created_at": "2026-10-26T00:00:00Z",
        })
        self.assertEqual(self.updater.process(p)["status"], "failed")
        self.assertFalse(any(c[0] == "POST" for c in self.api.calls))
        p["head"]["sha"] = D
        self.api.prs[1] = p
        self.api.current.add((A, D))
        self.assertEqual(self.updater.process(p)["status"], "checks_dispatched")

    def test_duplicate_branch_heads_are_reported_instead_of_ambiguous_dispatch(self):
        first, second = pull(1, head=B), pull(2, head=B)
        second["head"]["ref"] = first["head"]["ref"]
        self.api = FakeAPI([first, second])
        self.updater = mod.Updater(self.api, sleep=lambda _: None, update_wait=0)
        result = self.updater.reconcile()
        self.assertEqual([item["status"] for item in result], ["identity_ambiguous", "identity_ambiguous"])
        self.assertFalse(any(call[0] in {"PUT", "POST"} for call in self.api.calls))

    def test_pr_and_workflow_history_pagination_have_hard_bounds(self):
        class LargeAPI(FakeAPI):
            def call(self, path, **kwargs):
                if path.startswith(f"repos/{mod.REPO}/pulls?"):
                    return [{}] * mod.PAGE_SIZE
                if "/actions/workflows/" in path and "/runs?branch=" in path:
                    return {"workflow_runs": [{}] * mod.PAGE_SIZE}
                return super().call(path, **kwargs)
        api = LargeAPI()
        updater = mod.Updater(api, sleep=lambda _: None)
        with self.assertRaises(mod.ApiError):
            updater.pulls()
        with self.assertRaises(mod.ApiError):
            updater.workflow_runs("ci.yml", "feature/1")

    def test_report_serializes_machine_outcomes_and_summary_without_shell_injection(self):
        with tempfile.TemporaryDirectory() as tmp:
            artifact = Path(tmp) / "outcomes.json"
            summary = Path(tmp) / "summary.md"
            with patch.dict(os.environ, {"PR_UPDATE_OUTCOMES": str(artifact), "GITHUB_STEP_SUMMARY": str(summary), "GITHUB_RUN_ID": "5"}):
                mod.write_reports([{"number": 3, "url": "https://github.com/macel94/rillcache/pull/3", "status": "checks_passed", "detail": "ok | done"}])
            data = json.loads(artifact.read_text())
            self.assertEqual(data["pull_requests"][0]["number"], 3)
            self.assertIn("\\|", summary.read_text())

    def test_cli_rejects_pr_event_and_wrong_repo_before_api_access(self):
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": mod.REPO, "GITHUB_EVENT_NAME": "pull_request", "GITHUB_TOKEN": "secret"}):
            self.assertEqual(mod.main(), 2)
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": "attacker/repo", "GITHUB_EVENT_NAME": "push", "GITHUB_TOKEN": "secret"}):
            self.assertEqual(mod.main(), 2)


class WorkflowPolicyTests(unittest.TestCase):
    def test_only_read_only_self_tests_run_on_pr_events_and_writer_is_main_trusted(self):
        workflow = (ROOT / ".github/workflows/pr-branch-updater.yml").read_text()
        self.assertIn("pull_request:", workflow)
        self.assertIn("if: github.event_name == 'pull_request'", workflow)
        self.assertIn("if: github.event_name != 'pull_request' && github.ref == 'refs/heads/main'", workflow)
        self.assertIn("ref: main", workflow)
        self.assertIn("persist-credentials: false", workflow)
        self.assertIn("queue: max", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("workflow_run:", workflow)
        self.assertIn("workflow_dispatch:", workflow)
        self.assertIn("schedule:", workflow)
        self.assertIn("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1", workflow)
        self.assertIn("actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9", workflow)
        self.assertNotIn("pull_request_target:", workflow)
        self.assertNotIn("secrets.", workflow)
        self.assertNotIn("git push", workflow)
        self.assertNotIn("--force", workflow)
        self.assertIn("permissions:\n      contents: read\n    steps:", workflow)
        self.assertIn("contents: write", workflow)
        self.assertIn("pull-requests: write", workflow)
        self.assertIn("actions: write", workflow)

    def test_dispatch_allowlist_matches_real_required_workflows(self):
        source = (ROOT / "scripts/update-pr-branches.py").read_text()
        for workflow in ("ci.yml", "pr-quality.yml"):
            self.assertIn(f'"{workflow}"', source)
        self.assertEqual(set(mod.REQUIRED.values()), {"Tests and offline CI", "Valkey performance gate"})
        self.assertIn('path = f"repos/{REPO}/actions/workflows/{workflow}/dispatches"', source)
        self.assertIn('if workflow == "pr-quality.yml":', source)


if __name__ == "__main__":
    unittest.main()

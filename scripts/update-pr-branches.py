#!/usr/bin/env python3
"""Reconcile opted-in same-repository main PR branches from trusted GitHub Actions."""
from __future__ import annotations

import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO = "macel94/rillcache"
OWNER, NAME = REPO.split("/")
MAIN = "main"
REQUIRED = {
    "ci.yml": "Tests and offline CI",
    "pr-quality.yml": "Valkey performance gate",
}
ACTIONS_APP_ID = 15368
SHA = re.compile(r"^[0-9a-f]{40}$")
MAX_PAGES = 10
PAGE_SIZE = 100
API_VERSION = "2022-11-28"


class ApiError(RuntimeError):
    def __init__(self, status: int | None, message: str):
        super().__init__(message)
        self.status = status


class GitHub:
    """Small standard-library REST client; never prints token or API error bodies."""
    def __init__(self, token: str, opener=None, timeout: int = 20):
        if not token:
            raise ApiError(None, "GITHUB_TOKEN is required")
        self.token = token
        self.opener = opener or urllib.request.urlopen
        self.timeout = timeout

    def call(self, path: str, *, method: str = "GET", body: dict | None = None) -> Any:
        if path.startswith("/") or ".." in path.split("/"):
            raise ApiError(None, "unsafe GitHub API path")
        data = None if body is None else json.dumps(body, separators=(",", ":")).encode()
        req = urllib.request.Request(
            f"https://api.github.com/{path}", data=data, method=method,
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {self.token}",
                "X-GitHub-Api-Version": API_VERSION,
                "Content-Type": "application/json",
                "User-Agent": "rillcache-pr-branch-updater",
            },
        )
        try:
            with self.opener(req, timeout=self.timeout) as response:
                raw = response.read(2_000_001)
                if len(raw) > 2_000_000:
                    raise ApiError(response.status, "GitHub API response exceeded 2 MB")
                if not raw:
                    return None
                return json.loads(raw)
        except urllib.error.HTTPError as exc:
            # Do not echo GitHub's body: errors can contain untrusted metadata.
            raise ApiError(exc.code, f"GitHub API returned HTTP {exc.code}") from exc
        except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as exc:
            raise ApiError(None, f"GitHub API request failed ({type(exc).__name__})") from exc

    def pages(self, path: str) -> list[dict]:
        result: list[dict] = []
        for page in range(1, MAX_PAGES + 1):
            sep = "&" if "?" in path else "?"
            data = self.call(f"{path}{sep}per_page={PAGE_SIZE}&page={page}")
            if not isinstance(data, list):
                raise ApiError(None, "expected a paginated GitHub API list")
            result.extend(item for item in data if isinstance(item, dict))
            if len(data) < PAGE_SIZE:
                return result
        raise ApiError(None, f"GitHub API list exceeded {MAX_PAGES * PAGE_SIZE} records")


def valid_sha(value: Any) -> bool:
    return isinstance(value, str) and SHA.fullmatch(value) is not None


def iso_time(value: Any) -> float:
    if not isinstance(value, str):
        return 0.0
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return 0.0


def api_path_branch(branch: str) -> str:
    return urllib.parse.quote(branch, safe="")


class Updater:
    def __init__(self, api: GitHub, *, sleep=time.sleep, clock=time.time, update_wait=90):
        self.api = api
        self.sleep = sleep
        self.clock = clock
        self.update_wait = update_wait

    def repo(self) -> dict:
        info = self.api.call(f"repos/{REPO}")
        if info.get("full_name") != REPO or info.get("default_branch") != MAIN:
            raise ApiError(None, "repository/default-branch identity is not the expected main repository")
        return info

    def main_sha(self) -> str:
        ref = self.api.call(f"repos/{REPO}/git/ref/heads/{MAIN}")
        sha = ref.get("object", {}).get("sha")
        if not valid_sha(sha):
            raise ApiError(None, "main ref did not return a full commit SHA")
        return sha

    def pull(self, number: int) -> dict:
        pr = self.api.call(f"repos/{REPO}/pulls/{number}")
        if pr.get("number") != number:
            raise ApiError(None, f"PR #{number} response identity mismatch")
        return pr

    def pulls(self) -> list[dict]:
        return self.api.pages(f"repos/{REPO}/pulls?state=open&base={MAIN}")

    def eligible(self, pr: dict) -> tuple[bool, str]:
        """Every open same-repository PR targeting main is kept current.

        Draft state and auto-merge are deliberately not eligibility gates: the
        user asked for every open PR branch to stay up to date with main. This
        workflow only refreshes branches; it never promotes a draft, enables or
        revokes auto-merge, approves, or merges. Required-check reconciliation
        still applies only to non-draft PRs, which are the only ones that can
        merge.
        """
        if pr.get("state") != "open":
            return False, "closed"
        if pr.get("base", {}).get("ref") != MAIN or pr.get("base", {}).get("repo", {}).get("full_name") != REPO:
            return False, "not targeting this repository's main"
        if pr.get("head", {}).get("repo", {}).get("full_name") != REPO:
            return False, "fork head is not eligible"
        if not valid_sha(pr.get("base", {}).get("sha")) or not valid_sha(pr.get("head", {}).get("sha")):
            return False, "invalid base/head identity"
        if not isinstance(pr.get("head", {}).get("ref"), str) or not pr["head"]["ref"]:
            return False, "invalid head branch"
        return True, "eligible"

    def compare(self, base: str, head: str) -> str:
        result = self.api.call(f"repos/{REPO}/compare/{base}...{head}")
        status = result.get("status")
        if status not in {"ahead", "identical", "behind", "diverged"}:
            raise ApiError(None, "GitHub compare returned an unknown status")
        return status

    def workflow_runs(self, workflow: str, branch: str) -> list[dict]:
        path = (f"repos/{REPO}/actions/workflows/{workflow}/runs"
                f"?branch={api_path_branch(branch)}")
        result: list[dict] = []
        for page in range(1, MAX_PAGES + 1):
            data = self.api.call(f"{path}&per_page={PAGE_SIZE}&page={page}")
            runs = data.get("workflow_runs", [])
            if not isinstance(runs, list):
                raise ApiError(None, f"invalid workflow-run list for {workflow}")
            result.extend(r for r in runs if isinstance(r, dict))
            if len(runs) < PAGE_SIZE:
                return result
        raise ApiError(None, f"workflow-run history exceeded {MAX_PAGES * PAGE_SIZE} for {workflow}")

    @staticmethod
    def run_matches(run: dict, pr: dict, head: str, base: str, workflow: str) -> bool:
        # pull_requests[].head/base are mutable associations, not snapshots of
        # what this run tested. Current-main ancestry is guarded by process();
        # only the run's own source SHA/branch can establish candidate identity.
        return Updater.run_mentions_head(run, pr, workflow, head)

    def matching_runs(self, pr: dict, workflow: str, head: str, base: str) -> list[dict]:
        runs = self.workflow_runs(workflow, pr["head"]["ref"])
        matched = [r for r in runs if self.run_matches(r, pr, head, base, workflow)]
        return sorted(matched, key=lambda r: (iso_time(r.get("created_at")), int(r.get("id", 0) or 0)))

    @staticmethod
    def run_mentions_head(run: dict, pr: dict, workflow: str, head: str) -> bool:
        """Match immutable run source identity, never mutable PR head/base fields.

        Exact-head failures survive a main advance. Conversely, changing the
        associated PR cannot relabel an older run as evidence for a newer head.
        """
        if (run.get("path", "").split("@", 1)[0] != f".github/workflows/{workflow}"
                or run.get("head_sha") != head
                or run.get("head_branch") != pr["head"]["ref"]):
            return False
        if run.get("event") == "workflow_dispatch":
            return True
        if run.get("event") != "pull_request":
            return False
        bot_actor = run.get("actor", {}).get("login") == "github-actions[bot]"
        if run.get("conclusion") == "action_required" or (
                bot_actor and run.get("status") in {"queued", "waiting", "requested", "pending"}):
            # Approval-held token events have executed no tests.
            return False
        associated = run.get("pull_requests") or []
        return not associated or any(
            item.get("number") == pr["number"]
            for item in associated if isinstance(item, dict)
        )

    def runs_for_head(self, pr: dict, workflow: str, head: str) -> list[dict]:
        runs = self.workflow_runs(workflow, pr["head"]["ref"])
        matched = [r for r in runs if self.run_mentions_head(r, pr, workflow, head)]
        return sorted(matched, key=lambda r: (iso_time(r.get("created_at")), int(r.get("id", 0) or 0)))

    def workflow_trusted(self, workflow: str, ref: str) -> tuple[bool, str]:
        """Fail closed unless the branch's required workflow matches protected main.

        ``workflow_dispatch`` runs the workflow definition from the dispatched
        ref. A branch that rewrote ``ci.yml``/``pr-quality.yml`` could otherwise
        run attacker-controlled workflow code with the updater's write token.
        """
        path = f".github/workflows/{workflow}"
        try:
            main_file = self.api.call(f"repos/{REPO}/contents/{path}?ref={MAIN}")
            head_file = self.api.call(f"repos/{REPO}/contents/{path}?ref={api_path_branch(ref)}")
        except ApiError as exc:
            return False, f"cannot verify {workflow} definition: {exc}"
        if not isinstance(main_file, dict) or not isinstance(head_file, dict):
            return False, f"{workflow} content metadata unavailable"
        if not valid_sha(main_file.get("sha")) or main_file.get("sha") != head_file.get("sha"):
            return False, f"{workflow} differs from protected main or lacks a valid blob identity; refusing privileged dispatch"
        return True, "workflow definition matches protected main"

    def check_identity(self, run: dict, workflow: str) -> tuple[bool, str]:
        run_id = run.get("id")
        check_sha = run.get("head_sha")
        if not isinstance(run_id, int) or not valid_sha(check_sha):
            return False, "workflow run lacks valid id/head SHA"
        if run.get("path", "").split("@", 1)[0] != f".github/workflows/{workflow}":
            return False, "workflow path identity mismatch"
        if run.get("status") != "completed":
            return False, f"workflow run {run_id} is {run.get('status', 'unknown')}"
        if run.get("conclusion") != "success":
            return False, f"workflow run {run_id} concluded {run.get('conclusion', 'unknown')}; no automatic retry"
        checks = self.api.call(f"repos/{REPO}/commits/{check_sha}/check-runs?per_page=100")
        rows = [item for item in checks.get("check_runs", [])
                if item.get("name") == REQUIRED[workflow]
                and item.get("head_sha") == check_sha
                and item.get("check_suite", {}).get("id") == run.get("check_suite_id")]
        if len(rows) != 1:
            return False, f"required check identity is missing or ambiguous for {workflow}"
        check = rows[0]
        if (check.get("conclusion") != "success"
                or check.get("app", {}).get("slug") != "github-actions"
                or check.get("app", {}).get("id") != ACTIONS_APP_ID):
            return False, f"required check is not a successful GitHub Actions check for {workflow}"
        return True, "success"

    def dispatch(self, pr: dict, workflow: str) -> None:
        path = f"repos/{REPO}/actions/workflows/{workflow}/dispatches"
        body: dict[str, Any] = {"ref": pr["head"]["ref"]}
        if workflow == "pr-quality.yml":
            body["inputs"] = {"pull_request": str(pr["number"])}
        self.api.call(path, method="POST", body=body)

    def ensure_workflow(self, pr: dict, workflow: str, head: str, base: str) -> tuple[str, str]:
        runs = self.matching_runs(pr, workflow, head, base)
        if runs:
            failed = next((run for run in runs if run.get("status") == "completed"
                           and run.get("conclusion") != "success"), None)
            if failed is not None:
                _, detail = self.check_identity(failed, workflow)
                return "failed", detail
            active = next((run for run in runs if run.get("status") != "completed"), None)
            if active is not None:
                _, detail = self.check_identity(active, workflow)
                return "waiting", detail
            ok, detail = self.check_identity(runs[-1], workflow)
            return ("passed" if ok else "failed"), detail
        trusted, trust_detail = self.workflow_trusted(workflow, pr["head"]["ref"])
        if not trusted:
            return "untrusted_workflow", trust_detail
        try:
            self.dispatch(pr, workflow)
        except ApiError as exc:
            # An uncertain mutation is not repeated in this pass. A later pass
            # first searches Actions for the exact run before considering dispatch.
            return "dispatch_uncertain", str(exc)
        return "dispatched", "explicit workflow_dispatch accepted; awaiting exact run"

    def update_branch(self, number: int, frozen_main: str, frozen_head: str) -> tuple[str, dict]:
        """Merge frozen main into the PR branch; ``frozen_main`` is current main.

        ``pr.base.sha`` is only the PR's last-synced base snapshot, so it must
        never be compared with current main here.
        """
        try:
            self.api.call(f"repos/{REPO}/pulls/{number}/update-branch", method="PUT",
                          body={"expected_head_sha": frozen_head})
        except ApiError as exc:
            if exc.status in {409, 422}:
                return "conflict_or_race", {"error": str(exc)}
            return "update_uncertain", {"error": str(exc)}
        deadline = self.clock() + self.update_wait
        while True:
            latest = self.pull(number)
            main = self.main_sha()
            head = latest.get("head", {}).get("sha")
            if main != frozen_main:
                return "identity_race", {"main": main, "observed_head": head}
            if valid_sha(head) and head != frozen_head:
                if self.compare(frozen_main, head) in {"ahead", "identical"}:
                    return "updated", {"head": head}
            remaining = deadline - self.clock()
            if remaining <= 0:
                break
            self.sleep(min(5, remaining))
        return "update_pending", {"expected_head": frozen_head}

    def process(self, listed: dict) -> dict:
        number = listed.get("number")
        outcome: dict[str, Any] = {"number": number, "url": listed.get("html_url"), "status": "error"}
        if not isinstance(number, int) or number <= 0:
            outcome["detail"] = "invalid PR number"
            return outcome
        try:
            pr = self.pull(number)
            eligible, why = self.eligible(pr)
            if not eligible:
                outcome.update(status="skipped", detail=why)
                return outcome
            head = pr["head"]["sha"]
            # ``pr.base.sha`` is the PR's last-synced base snapshot, not current
            # main. Staleness is whether the head already contains current main.
            main = self.main_sha()
            stale = self.compare(main, head) not in {"ahead", "identical"}
            if stale:
                # Failed evidence for this exact source head is a stop condition:
                # do not update it merely to get a new merge SHA/check identity.
                for workflow in REQUIRED:
                    prior = self.runs_for_head(pr, workflow, head)
                    if any(run.get("status") == "completed" and run.get("conclusion") != "success" for run in prior):
                        outcome.update(status="failed_gate", base=main, head=head,
                                       detail=f"{workflow} failed for this exact source head; no automatic retry")
                        return outcome
                    if any(run.get("status") != "completed" for run in prior):
                        outcome.update(status="waiting_existing_check", base=main, head=head,
                                       detail=f"{workflow} is already running for this exact source head")
                        return outcome
                latest = self.pull(number)
                still_eligible, eligibility = self.eligible(latest)
                current_main = self.main_sha()
                if not still_eligible:
                    outcome.update(status="consent_revoked", base=main, head=head,
                                   detail=f"PR stopped being eligible before guarded update: {eligibility}")
                    return outcome
                if latest.get("head", {}).get("sha") != head or current_main != main:
                    outcome.update(status="identity_race", base=main, head=head,
                                   detail="head/main changed before guarded update")
                    return outcome
                state, detail = self.update_branch(number, main, head)
                outcome.update(status=state, base=main, previous_head=head, **detail)
                if state != "updated":
                    return outcome
                # The branch now has a new head (and will need fresh checks).
                # reconcile() spends the single per-pass refresh slot on it.
                outcome["refreshed"] = True
                head = detail["head"]
                pr = self.pull(number)
                still_eligible, eligibility = self.eligible(pr)
                if not still_eligible:
                    outcome.update(status="consent_revoked", head=head,
                                   detail=f"PR stopped being eligible after branch update: {eligibility}")
                    return outcome
                if pr.get("head", {}).get("sha") != head or self.main_sha() != main:
                    outcome.update(status="identity_race", head=head,
                                   detail="candidate identity changed immediately after update")
                    return outcome
                if self.compare(main, head) not in {"ahead", "identical"}:
                    outcome.update(status="not_current", head=head,
                                   detail="updated candidate does not contain frozen main")
                    return outcome

            if pr.get("draft"):
                # Draft branches are kept current, but the expensive required
                # checks are reconciled once the PR is ready for review.
                outcome.update(status="draft_updated" if stale else "draft_current",
                               base=main, head=head,
                               detail="draft branch reconciled with main; required checks run when it is ready")
                return outcome

            outcome.update(base=main, head=head)
            states = {}
            for workflow in REQUIRED:
                pr = self.pull(number)
                still_eligible, eligibility = self.eligible(pr)
                if not still_eligible:
                    outcome.update(status="consent_revoked", workflows=states,
                                   detail=f"PR stopped being eligible before {workflow}: {eligibility}")
                    return outcome
                if pr.get("head", {}).get("sha") != head or self.main_sha() != main:
                    outcome.update(status="identity_race", workflows=states,
                                   detail=f"candidate identity changed before {workflow}")
                    return outcome
                state, detail = self.ensure_workflow(pr, workflow, head, main)
                states[workflow] = {"status": state, "detail": detail}
                if state not in {"passed", "dispatched"}:
                    outcome.update(status="waiting" if state == "waiting" else state,
                                   workflows=states, detail=f"{workflow}: {detail}")
                    return outcome
            latest = self.pull(number)
            still_eligible, eligibility = self.eligible(latest)
            if not still_eligible:
                outcome.update(status="consent_revoked", workflows=states,
                               detail=f"PR stopped being eligible while checks were running: {eligibility}")
                return outcome
            if latest.get("head", {}).get("sha") != head or self.main_sha() != main:
                outcome.update(status="identity_race", workflows=states,
                               detail="head/main changed while required checks were being reconciled")
                return outcome
            if all(value["status"] == "passed" for value in states.values()):
                outcome.update(status="checks_passed", workflows=states,
                               detail="both exact required workflow/check identities passed; protected auto-merge remains authoritative")
            else:
                outcome.update(status="checks_dispatched", workflows=states,
                               detail="missing required workflows explicitly dispatched; awaiting exact runs")
            return outcome
        except ApiError as exc:
            outcome.update(status="api_error", detail=str(exc))
            return outcome

    def reconcile(self) -> list[dict]:
        """Reconcile the serialized PR queue, refreshing at most one branch.

        A pass walks eligible PRs in ascending PR-number order and stops after
        the first successful branch refresh: that refresh creates a new head and
        a new required-check identity, so refreshing every stale PR at once would
        stampede the gate. Candidates that conflict, race, time out or error are
        recorded and the pass continues to the next candidate. Every PR behind the
        successful refresh is reported as ``deferred`` so the queue is visible;
        later passes (main push, schedule, workflow completion) advance it.
        """
        self.repo()
        prs = self.pulls()
        if len(prs) >= MAX_PAGES * PAGE_SIZE:
            raise ApiError(None, "PR reconciliation reached configured page bound")
        identities: dict[tuple[str, str], int] = {}
        for pr in prs:
            head = pr.get("head", {})
            identity = (head.get("ref", ""), head.get("sha", ""))
            identities[identity] = identities.get(identity, 0) + 1
        outcomes = []
        refreshed_pr: int | None = None
        for pr in sorted(prs, key=lambda item: item.get("number", 0)):
            number = pr.get("number")
            head = pr.get("head", {})
            if refreshed_pr is not None:
                outcomes.append(self.deferred(pr, refreshed_pr))
                continue
            identity = (head.get("ref", ""), head.get("sha", ""))
            if identities.get(identity, 0) > 1:
                outcomes.append({"number": number, "url": pr.get("html_url"),
                                 "status": "identity_ambiguous",
                                 "detail": "multiple open PRs share this branch/head; workflow-dispatch PR identity cannot be proven"})
                continue
            outcome = self.process(pr)
            outcomes.append(outcome)
            if outcome.get("refreshed"):
                refreshed_pr = number if isinstance(number, int) else 0
        if sum(1 for item in outcomes if item.get("refreshed")) > 1:
            raise ApiError(None, "invariant violated: more than one branch refresh in a pass")
        return outcomes

    @staticmethod
    def deferred(listed: dict, refreshed_pr: int) -> dict:
        """Report a PR that this pass did not attempt because the slot was used."""
        head = listed.get("head", {})
        return {"number": listed.get("number"), "url": listed.get("html_url"),
                "status": "deferred", "head": head.get("sha"),
                "detail": (f"not attempted: this pass already refreshed PR #{refreshed_pr}; "
                           "one successful branch refresh per pass, a later pass advances the queue")}


def write_reports(outcomes: list[dict]) -> None:
    path = os.environ.get("PR_UPDATE_OUTCOMES")
    report = {"schema": 2, "repository": REPO, "run_id": os.environ.get("GITHUB_RUN_ID"),
              "generated_at": datetime.now(timezone.utc).isoformat(), "pull_requests": outcomes}
    if path:
        target = Path(path)
        target.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        lines = ["## PR branch reconciliation", "", "| PR | Status | Base | Head | Detail |", "|---:|---|---|---|---|"]
        for item in outcomes:
            number = item.get("number", "?")
            url = item.get("url")
            label = f"[#{number}]({url})" if isinstance(url, str) and url.startswith("https://github.com/") else f"#{number}"
            detail = str(item.get("detail") or item.get("error") or "").replace("|", "\\|").replace("\n", " ")
            lines.append(f"| {label} | {item.get('status', 'unknown')} | `{item.get('base', '')}` | `{item.get('head', item.get('previous_head', ''))}` | {detail} |")
        refreshed = [item.get("number") for item in outcomes if item.get("refreshed")]
        deferred = [item.get("number") for item in outcomes if item.get("status") == "deferred"]
        lines.append("")
        lines.append(f"Refreshed this pass: {', '.join(f'#{n}' for n in refreshed) if refreshed else 'none'}")
        lines.append(f"Deferred to a later pass: {len(deferred)}"
                     + (f" ({', '.join(f'#{n}' for n in deferred)})" if deferred else ""))
        lines.append("")
        lines.append("At most one successful branch refresh is performed per pass, so a rebased head "
                     "does not stampede the required checks. Deferred PRs stay stale only until a "
                     "later pass reaches them; failed candidates remain visible in the table above.")
        Path(summary).write_text("\n".join(lines) + "\n", encoding="utf-8", errors="replace")


def main() -> int:
    if os.environ.get("GITHUB_REPOSITORY") != REPO:
        print("refusing unexpected repository context", file=sys.stderr)
        return 2
    if os.environ.get("GITHUB_EVENT_NAME") == "pull_request":
        print("privileged updater must not run on pull_request", file=sys.stderr)
        return 2
    try:
        updater = Updater(GitHub(os.environ.get("GITHUB_TOKEN", "")))
        outcomes = updater.reconcile()
        write_reports(outcomes)
        print(json.dumps(outcomes, sort_keys=True))
        blocking = {"error", "api_error", "failed", "failed_gate", "conflict_or_race", "identity_race",
                    "identity_ambiguous", "consent_revoked", "dispatch_uncertain",
                    "untrusted_workflow", "update_uncertain", "not_current"}
        # "deferred" is the expected serialized-queue state, not a failure: the
        # PR stays queued for a later pass and its branch is untouched.
        return 1 if any(item.get("status") in blocking for item in outcomes) else 0
    except ApiError as exc:
        print(f"PR updater failed closed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

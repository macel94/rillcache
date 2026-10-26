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
        if pr.get("state") != "open":
            return False, "closed"
        if pr.get("base", {}).get("ref") != MAIN or pr.get("base", {}).get("repo", {}).get("full_name") != REPO:
            return False, "not targeting this repository's main"
        if pr.get("head", {}).get("repo", {}).get("full_name") != REPO:
            return False, "fork head is not eligible"
        if pr.get("draft"):
            return False, "draft PR is not eligible"
        if not pr.get("auto_merge"):
            return False, "explicit protected auto-merge request is absent"
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
        if run.get("path", "").split("@", 1)[0] != f".github/workflows/{workflow}":
            return False
        if run.get("event") == "workflow_dispatch":
            return run.get("head_sha") == head and run.get("head_branch") == pr["head"]["ref"]
        if run.get("event") != "pull_request":
            return False
        bot_actor = run.get("actor", {}).get("login") == "github-actions[bot]"
        if run.get("conclusion") == "action_required" or (
                bot_actor and run.get("status") in {"queued", "waiting", "requested", "pending"}):
            # GITHUB_TOKEN-generated PR runs can be approval-held; they are not
            # test evidence and must not suppress the explicit dispatch fallback.
            return False
        return any(
            item.get("number") == pr["number"]
            and item.get("head", {}).get("sha") == head
            and item.get("base", {}).get("sha") == base
            for item in run.get("pull_requests", []) if isinstance(item, dict)
        )

    def matching_runs(self, pr: dict, workflow: str, head: str, base: str) -> list[dict]:
        runs = self.workflow_runs(workflow, pr["head"]["ref"])
        matched = [r for r in runs if self.run_matches(r, pr, head, base, workflow)]
        return sorted(matched, key=lambda r: (iso_time(r.get("created_at")), int(r.get("id", 0) or 0)))

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
            latest = runs[-1]
            ok, detail = self.check_identity(latest, workflow)
            if latest.get("status") != "completed":
                return "waiting", detail
            if not ok:
                return "failed", detail
            return "passed", detail
        try:
            self.dispatch(pr, workflow)
        except ApiError as exc:
            # An uncertain mutation is not repeated in this pass. A later pass
            # first searches Actions for the exact run before considering dispatch.
            return "dispatch_uncertain", str(exc)
        return "dispatched", "explicit workflow_dispatch accepted; awaiting exact run"

    def update_branch(self, number: int, frozen_base: str, frozen_head: str) -> tuple[str, dict]:
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
            base = latest.get("base", {}).get("sha")
            head = latest.get("head", {}).get("sha")
            if main != frozen_base or base != frozen_base:
                return "identity_race", {"observed_base": base, "main": main, "observed_head": head}
            if valid_sha(head) and head != frozen_head:
                status = self.compare(frozen_base, head)
                if status in {"ahead", "identical"}:
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
            base = pr["base"]["sha"]
            head = pr["head"]["sha"]
            main = self.main_sha()
            if base != main:
                outcome.update(status="base_race", base=base, main=main,
                               detail="PR base is not the frozen current main; rescan later")
                return outcome

            stale = self.compare(base, head) not in {"ahead", "identical"}
            if stale:
                # Failed evidence for this exact source head is a stop condition:
                # do not update it merely to get a new merge SHA/check identity.
                for workflow in REQUIRED:
                    prior = self.matching_runs(pr, workflow, head, base)
                    if prior and prior[-1].get("status") == "completed" and prior[-1].get("conclusion") != "success":
                        outcome.update(status="failed_gate", base=base, head=head,
                                       detail=f"{workflow} failed for this exact candidate; no automatic retry")
                        return outcome
                    if prior and prior[-1].get("status") != "completed":
                        outcome.update(status="waiting_existing_check", base=base, head=head,
                                       detail=f"{workflow} is already running for this exact candidate")
                        return outcome
                latest = self.pull(number)
                still_eligible, eligibility = self.eligible(latest)
                current_main = self.main_sha()
                if not still_eligible:
                    outcome.update(status="consent_revoked", base=base, head=head,
                                   detail=f"eligibility changed before guarded update: {eligibility}")
                    return outcome
                if (latest.get("head", {}).get("sha") != head
                        or latest.get("base", {}).get("sha") != base or current_main != base):
                    outcome.update(status="identity_race", base=base, head=head,
                                   detail="base/head changed before guarded update")
                    return outcome
                state, detail = self.update_branch(number, base, head)
                outcome.update(status=state, base=base, previous_head=head, **detail)
                if state != "updated":
                    return outcome
                head = detail["head"]
                pr = self.pull(number)
                still_eligible, eligibility = self.eligible(pr)
                if not still_eligible:
                    outcome.update(status="consent_revoked", head=head,
                                   detail=f"eligibility changed after branch update: {eligibility}")
                    return outcome
                if (pr.get("head", {}).get("sha") != head or pr.get("base", {}).get("sha") != base
                        or self.main_sha() != base):
                    outcome.update(status="identity_race", head=head,
                                   detail="candidate identity changed immediately after update")
                    return outcome
                if self.compare(base, head) not in {"ahead", "identical"}:
                    outcome.update(status="not_current", head=head,
                                   detail="updated candidate does not contain frozen main")
                    return outcome

            outcome.update(base=base, head=head)
            states = {}
            for workflow in REQUIRED:
                pr = self.pull(number)
                still_eligible, eligibility = self.eligible(pr)
                if not still_eligible:
                    outcome.update(status="consent_revoked", workflows=states,
                                   detail=f"eligibility changed before {workflow}: {eligibility}")
                    return outcome
                if (pr.get("head", {}).get("sha") != head or pr.get("base", {}).get("sha") != base
                        or self.main_sha() != base):
                    outcome.update(status="identity_race", workflows=states,
                                   detail=f"candidate identity changed before {workflow}")
                    return outcome
                state, detail = self.ensure_workflow(pr, workflow, head, base)
                states[workflow] = {"status": state, "detail": detail}
                if state not in {"passed", "dispatched"}:
                    outcome.update(status="waiting" if state == "waiting" else state,
                                   workflows=states, detail=f"{workflow}: {detail}")
                    return outcome
            latest = self.pull(number)
            still_eligible, eligibility = self.eligible(latest)
            if not still_eligible:
                outcome.update(status="consent_revoked", workflows=states,
                               detail=f"eligibility changed while checks were running: {eligibility}")
                return outcome
            if (latest.get("head", {}).get("sha") != head or latest.get("base", {}).get("sha") != base
                    or self.main_sha() != base):
                outcome.update(status="identity_race", workflows=states,
                               detail="base/head changed while required checks were being reconciled")
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
        for pr in sorted(prs, key=lambda item: item.get("number", 0)):
            head = pr.get("head", {})
            identity = (head.get("ref", ""), head.get("sha", ""))
            if identities.get(identity, 0) > 1:
                outcomes.append({"number": pr.get("number"), "url": pr.get("html_url"),
                                 "status": "identity_ambiguous",
                                 "detail": "multiple open PRs share this branch/head; workflow-dispatch PR identity cannot be proven"})
            else:
                outcomes.append(self.process(pr))
        return outcomes


def write_reports(outcomes: list[dict]) -> None:
    path = os.environ.get("PR_UPDATE_OUTCOMES")
    report = {"schema": 1, "repository": REPO, "run_id": os.environ.get("GITHUB_RUN_ID"),
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
            detail = str(item.get("detail", "")).replace("|", "\\|").replace("\n", " ")
            lines.append(f"| {label} | {item.get('status', 'unknown')} | `{item.get('base', '')}` | `{item.get('head', item.get('previous_head', ''))}` | {detail} |")
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
        blocking = {"api_error", "failed", "failed_gate", "conflict_or_race", "identity_race",
                    "identity_ambiguous", "consent_revoked", "dispatch_uncertain",
                    "update_uncertain", "not_current"}
        return 1 if any(item.get("status") in blocking for item in outcomes) else 0
    except ApiError as exc:
        print(f"PR updater failed closed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

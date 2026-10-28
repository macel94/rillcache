#!/usr/bin/env python3
"""Read-only report-PR identity/run/auto-merge verification through gh, stdlib only.

The shell publisher alone creates its own report PR and requests protected
squash auto-merge. This helper never approves, merges, updates refs or retries a
failed run. It prints dispatch-needed flags, never an assertion that tests passed.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
from urllib.parse import quote

sys.path.insert(0, str(Path(__file__).resolve().parent))
from pr_gate_policy import PolicyError, audit_live, required_workflows

WORKFLOWS = tuple(required_workflows())
SHA = re.compile(r"[0-9a-f]{40}\Z")


class DeliveryError(RuntimeError):
    pass


class GitHub:
    def __init__(self, repo):
        self.repo = repo
        self.deadline = time.monotonic() + 150

    def get(self, path):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise DeliveryError("report delivery API observation deadline exceeded")
        try:
            result = subprocess.run(["gh", "api", "--method", "GET", f"repos/{self.repo}/{path}"],
                                    capture_output=True, text=True, timeout=min(20, remaining), check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise DeliveryError("GitHub API unavailable or timed out") from error
        if result.returncode:
            # Do not echo potentially sensitive/untrusted API or gh error bodies.
            raise DeliveryError("GitHub API command failed; no mutation/retry was attempted")
        try:
            return json.loads(result.stdout)
        except (ValueError, TypeError) as error:
            raise DeliveryError("invalid GitHub API JSON") from error


def identity(api, number, branch, expected_head=None, allow_merged=False):
    pr = api.get(f"pulls/{number}")
    if not isinstance(pr, dict):
        raise DeliveryError("invalid pull request response")
    head = pr.get("head", {})
    base = pr.get("base", {})
    if (pr.get("number") != number or head.get("ref") != branch or base.get("ref") != "main"
            or head.get("repo", {}).get("full_name") != api.repo
            or base.get("repo", {}).get("full_name") != api.repo
            or pr.get("draft") is not False):
        raise DeliveryError("report PR must be ready, same-repository and target main with the expected branch")
    sha = head.get("sha", "")
    if not isinstance(sha, str) or SHA.fullmatch(sha) is None or (expected_head is not None and sha != expected_head):
        raise DeliveryError("report PR head identity changed or is malformed")
    merged = pr.get("state") == "closed" and bool(pr.get("merged_at"))
    if pr.get("state") != "open" and not (allow_merged and merged):
        raise DeliveryError("report PR is not open (or normally merged)")
    return pr


def matching(run, repo, number, branch, head, workflow):
    if not isinstance(run, dict):
        return False
    if (run.get("path", "").split("@", 1)[0] != f".github/workflows/{workflow}"
            or run.get("head_branch") != branch or run.get("head_sha") != head
            or run.get("event") not in {"pull_request", "workflow_dispatch"}
            or run.get("head_repository", {}).get("full_name") != repo):
        return False
    if run["event"] == "pull_request":
        associated = run.get("pull_requests") or []
        # GitHub can omit associations. Exact source SHA + owned unique report
        # branch + repository still identify real evidence; do not duplicate it.
        return not associated or any(p.get("number") == number for p in associated if isinstance(p, dict))
    return True


def runs(api, workflow, branch):
    result = []
    for page in range(1, 11):
        response = api.get(f"actions/workflows/{workflow}/runs?branch={quote(branch, safe='')}&per_page=100&page={page}")
        items = response.get("workflow_runs") if isinstance(response, dict) else None
        if not isinstance(items, list):
            raise DeliveryError("invalid workflow history response")
        result.extend(items)
        if len(items) < 100:
            return result
    raise DeliveryError("workflow history exceeded the 1,000-run observation bound")


def dispatch_plan(api, number, branch):
    try:
        audit_live(api)
    except (PolicyError, KeyError, TypeError, ValueError) as error:
        raise DeliveryError(f"live merge protection audit failed: {error}") from error
    pr = identity(api, number, branch)
    head = pr["head"]["sha"]
    main = api.get("git/ref/heads/main").get("object", {}).get("sha", "")
    if not isinstance(main, str) or SHA.fullmatch(main) is None:
        raise DeliveryError("invalid protected main identity")
    if api.get(f"compare/{main}...{head}").get("status") not in {"ahead", "identical"}:
        raise DeliveryError("report branch is stale; trusted branch reconciliation must update it first")
    flags = []
    for workflow in WORKFLOWS:
        observed = [r for r in runs(api, workflow, branch) if matching(r, api.repo, number, branch, head, workflow)]
        # Approval-held GITHUB_TOKEN PR events have executed no tests. Explicit
        # dispatch is allowed only when no real run already exists for the head.
        real = [r for r in observed if r.get("conclusion") != "action_required"]
        if any(r.get("status") not in {"queued", "in_progress", "waiting", "requested", "pending", "completed"} for r in real):
            raise DeliveryError(f"{workflow} has an unknown run state; refusing dispatch")
        if any(r.get("status") == "completed" and r.get("conclusion") != "success" for r in real):
            raise DeliveryError(f"{workflow} has failed/invalid exact-head evidence; refusing a retry")
        if real:
            flags.append(0)
            continue
        path = f"contents/.github/workflows/{workflow}"
        protected = api.get(f"{path}?ref={main}")
        candidate = api.get(f"{path}?ref={head}")
        if (not isinstance(protected, dict) or not isinstance(candidate, dict)
                or not isinstance(protected.get("sha"), str)
                or SHA.fullmatch(protected["sha"]) is None or protected["sha"] != candidate.get("sha")):
            raise DeliveryError(f"{workflow} does not match protected main; refusing dispatch")
        flags.append(1)
    identity(api, number, branch, head)
    if api.get("git/ref/heads/main").get("object", {}).get("sha") != main:
        raise DeliveryError("main advanced during delivery observation")
    return head, flags


def verify_auto(api, number, branch, head, require):
    pr = identity(api, number, branch, head, allow_merged=True)
    if pr.get("state") == "closed" and pr.get("merged_at"):
        return "normally merged through the protected PR path"
    request = pr.get("auto_merge")
    if request:
        if request.get("merge_method") != "squash":
            raise DeliveryError("auto-merge method is not the required squash method")
        return "protected squash auto-merge verified (checks/reviews remain authoritative)"
    if require:
        raise DeliveryError("auto-merge was not actually enabled; refusing publication success")
    return "auto-merge disabled on existing PR; preserving existing merge intent"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("inspect", "verify-auto"))
    parser.add_argument("--number", type=int, required=True)
    parser.add_argument("--branch", required=True)
    parser.add_argument("--head")
    parser.add_argument("--require-enabled", action="store_true")
    args = parser.parse_args()
    try:
        repo = os.environ.get("GITHUB_REPOSITORY", "")
        if (re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo) is None or args.number <= 0
                or re.fullmatch(r"automation/benchmark-report-[1-9][0-9]*-[1-9][0-9]*", args.branch) is None):
            raise DeliveryError("invalid report-delivery context")
        api = GitHub(repo)
        if args.operation == "inspect":
            head, flags = dispatch_plan(api, args.number, args.branch)
            print(head, *flags)
        else:
            if not args.head or SHA.fullmatch(args.head) is None:
                raise DeliveryError("a full expected head is required for auto-merge verification")
            print(verify_auto(api, args.number, args.branch, args.head, args.require_enabled))
        return 0
    except DeliveryError as error:
        print(f"Report PR delivery stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

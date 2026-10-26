#!/usr/bin/env python3
"""Restartable, fail-closed serialized PR branch updater (Python stdlib only)."""
from __future__ import annotations

import argparse
import fcntl
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import quote

OWNER = "macel94/rillcache"
MAIN = "main"
GITHUB_ACTIONS_APP_ID = 15368
UPDATE_OBSERVATION_SECONDS = 900
DISPATCH_OBSERVATION_SECONDS = 180
WORKFLOW_OBSERVATION_SECONDS = 3600
REQUIRED = {"ci.yml": "Tests and offline CI", "pr-quality.yml": "Valkey performance gate"}
HEX = re.compile(r"^[0-9a-f]{40}$")


class DeliveryError(RuntimeError):
    pass


class Gh:
    """Bounded gh CLI adapter; credentials stay in gh's own credential store."""
    def __init__(self, timeout: int = 20):
        self.timeout = timeout

    def call(self, args: list[str], *, data: dict | None = None, method: str | None = None, timeout: int | None = None) -> Any:
        cmd = ["gh", "api", "--hostname", "github.com"]
        if data is not None:
            cmd += ["--method", method or "POST", "--input", "-"]
        elif method:
            cmd += ["--method", method]
        cmd += args
        try:
            p = subprocess.run(cmd, input=json.dumps(data) if data is not None else None,
                               text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               timeout=timeout or self.timeout, check=False)
        except subprocess.TimeoutExpired as exc:
            raise DeliveryError(f"bounded gh call timed out: {args[0]}") from exc
        if p.returncode:
            # Never echo stderr: it can contain URLs or credential-bearing diagnostics.
            raise DeliveryError(f"gh API failed ({p.returncode}) for {args[0]}")
        try:
            return json.loads(p.stdout) if p.stdout else None
        except json.JSONDecodeError as exc:
            raise DeliveryError(f"invalid JSON from gh API for {args[0]}") from exc

    def get(self, path: str) -> Any:
        return self.call([path])

    def post(self, path: str, data: dict) -> Any:
        return self.call([path], data=data, method="POST")

    def put(self, path: str, data: dict) -> Any:
        return self.call([path], data=data, method="PUT")


@dataclass
class Local:
    root: Path

    def __post_init__(self) -> None:
        self.state_dir = self.root / "state"
        self.state_path = self.state_dir / "controller.json"
        self.lock_path = self.state_dir / "controller.lock"

    def secure_dir(self) -> None:
        self.state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        if self.state_dir.is_symlink() or not self.state_dir.is_dir():
            raise DeliveryError("state directory must be a real directory, not a symlink")
        os.chmod(self.state_dir, 0o700)

    def lock(self):
        self.secure_dir()
        fd = os.open(self.lock_path, os.O_CREAT | os.O_RDWR | getattr(os, "O_NOFOLLOW", 0), 0o600)
        os.chmod(self.lock_path, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            os.close(fd)
            raise DeliveryError("another controller holds the local lease") from exc
        return fd

    def load(self) -> dict:
        if self.state_dir.is_symlink() or (self.state_dir.exists() and not self.state_dir.is_dir()):
            raise DeliveryError("state directory must be a real directory, not a symlink")
        if self.state_path.is_symlink():
            raise DeliveryError("state file must not be a symlink")
        if not self.state_path.exists():
            return {"version": 1, "attempts": {}, "active": None, "events": []}
        try:
            value = json.loads(self.state_path.read_text())
            if value.get("version") != 1 or not isinstance(value.get("attempts"), dict):
                raise ValueError("unexpected state schema")
            return value
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise DeliveryError(f"cannot safely read controller state: {exc}") from exc

    def save(self, state: dict) -> None:
        self.secure_dir()
        fd, temp_name = tempfile.mkstemp(prefix=".controller-", dir=self.state_dir)
        try:
            with os.fdopen(fd, "w") as stream:
                os.fchmod(stream.fileno(), 0o600)
                json.dump(state, stream, sort_keys=True, indent=2)
                stream.write("\n")
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temp_name, self.state_path)
            os.chmod(self.state_path, 0o600)
            dir_fd = os.open(self.state_dir, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(dir_fd)
            finally:
                os.close(dir_fd)
        finally:
            if os.path.exists(temp_name):
                os.unlink(temp_name)


def key(pr: dict, base: str, head: str) -> str:
    return f"{pr['number']}:{base}:{head}"


class Controller:
    def __init__(self, api: Gh, local: Local):
        self.api, self.local = api, local

    def verify_repo(self) -> None:
        try:
            p = subprocess.run(["gh", "repo", "view", "--json", "nameWithOwner"],
                               text=True, capture_output=True, timeout=10, check=True)
            repo = json.loads(p.stdout).get("nameWithOwner")
        except (subprocess.SubprocessError, ValueError, OSError) as exc:
            raise DeliveryError("cannot validate gh repository context") from exc
        if repo != OWNER:
            raise DeliveryError(f"repository context must be {OWNER}")

    def pull(self, number: int) -> dict:
        p = self.api.get(f"repos/{OWNER}/pulls/{number}")
        if p.get("number") != number or p.get("base", {}).get("repo", {}).get("full_name") != OWNER:
            raise DeliveryError(f"PR #{number} is not based in {OWNER}")
        return p

    def list_prs(self) -> list[dict]:
        prs = self.api.get(f"repos/{OWNER}/pulls?state=open&base={MAIN}&per_page=100")
        return sorted((p for p in prs if p.get("base", {}).get("ref") == MAIN),
                      key=lambda p: p["number"])

    @staticmethod
    def valid_identity(pr: dict) -> bool:
        return (pr.get("state") == "open" and not pr.get("draft") and
                pr.get("base", {}).get("ref") == MAIN and
                pr.get("base", {}).get("repo", {}).get("full_name") == OWNER and
                pr.get("head", {}).get("repo", {}).get("full_name") == OWNER and
                bool(HEX.fullmatch(pr.get("head", {}).get("sha", ""))) and
                bool(HEX.fullmatch(pr.get("base", {}).get("sha", ""))))

    def opted_in(self, number: int) -> bool:
        result = self.api.get(f"repos/{OWNER}/pulls/{number}")
        # The REST pull representation advertises auto_merge only when requested.
        return bool(result.get("auto_merge"))

    def review_blocker(self, number: int) -> str | None:
        requested = self.api.get(f"repos/{OWNER}/pulls/{number}/requested_reviewers")
        pending = [x.get("login", "unknown reviewer") for x in requested.get("users", [])]
        pending += [x.get("slug", "unknown team") for x in requested.get("teams", [])]
        if pending:
            return "requested reviews pending: " + ", ".join(pending)
        reviews = self.api.get(f"repos/{OWNER}/pulls/{number}/reviews?per_page=100")
        latest: dict[str, str] = {}
        for review in reviews:
            user = review.get("user", {}).get("login")
            if user and review.get("state") in {"APPROVED", "CHANGES_REQUESTED", "DISMISSED"}:
                latest[user] = review["state"]
        if "CHANGES_REQUESTED" in latest.values():
            return "unresolved changes-requested review"
        return None

    def reviews_clear(self, number: int) -> bool:
        return self.review_blocker(number) is None

    def runs(self, workflow: str, head: str) -> list[dict]:
        data = self.api.get(f"repos/{OWNER}/actions/workflows/{workflow}/runs?per_page=100&head_sha={head}")
        return data.get("workflow_runs", [])

    def branch_runs(self, workflow: str, branch: str) -> list[dict]:
        data = self.api.get(f"repos/{OWNER}/actions/workflows/{workflow}/runs?per_page=100&branch={quote(branch, safe='')}")
        return data.get("workflow_runs", [])

    def stale(self, pr: dict) -> bool:
        base, head = pr["base"]["sha"], pr["head"]["sha"]
        comparison = self.api.get(f"repos/{OWNER}/compare/{base}...{head}")
        # A head is current only when it contains the frozen base. Divergence is
        # stale too; the normal update API decides whether it can merge cleanly.
        return comparison.get("status") not in {"ahead", "identical"}

    def checks(self, pr: dict, attempt: dict) -> tuple[bool, str]:
        # Require independently dispatched Actions workflows for this exact branch SHA.
        head = pr["head"]["sha"]
        expected = {"ci.yml": "ci_run_id", "pr-quality.yml": "gate_run_id"}
        for workflow, idfield in expected.items():
            run_id = attempt.get(idfield)
            if not run_id:
                return False, f"waiting for explicit {workflow} dispatch on {head}"
            run = self.api.get(f"repos/{OWNER}/actions/runs/{run_id}")
            if not run or str(run.get("id")) != str(run_id):
                return False, f"workflow {workflow} run {run_id} is not observable for {head}"
            if run.get("event") == "workflow_dispatch":
                if run.get("head_sha") != head or run.get("path", "").split("@", 1)[0] != f".github/workflows/{workflow}":
                    return False, f"workflow {workflow} identity does not match current head/path"
                check_sha = head
            elif run.get("event") == "pull_request":
                refs = run.get("pull_requests", [])
                exact = any(x.get("head", {}).get("sha") == head and x.get("base", {}).get("sha") == attempt["base"] for x in refs)
                if not exact or run.get("path", "").split("@", 1)[0] != f".github/workflows/{workflow}":
                    return False, f"workflow {workflow} PR run does not match exact head/base/path"
                check_sha = run.get("head_sha")
            else:
                return False, f"workflow {workflow} has unexpected trigger {run.get('event')}"
            if run.get("status") != "completed":
                started = run.get("run_started_at") or run.get("created_at")
                if attempt.get("candidate_created_at") and time.time() - attempt["candidate_created_at"] > WORKFLOW_OBSERVATION_SECONDS:
                    return False, f"workflow {workflow} observation timed out; owner must investigate, no rerun"
                return False, f"workflow {workflow} run {run_id} is {run.get('status')} (started {started})"
            if run.get("conclusion") != "success":
                return False, f"workflow {workflow} failed ({run.get('conclusion')}); owner fix required"
            checks = self.api.get(f"repos/{OWNER}/commits/{check_sha}/check-runs?per_page=100")
            matching = [c for c in checks.get("check_runs", []) if c.get("name") == REQUIRED[workflow] and c.get("head_sha") == check_sha and c.get("check_suite", {}).get("id") == run.get("check_suite_id")]
            if (len(matching) != 1 or matching[0].get("app", {}).get("slug") != "github-actions" or
                    matching[0].get("app", {}).get("id") != GITHUB_ACTIONS_APP_ID or matching[0].get("conclusion") != "success"):
                return False, f"required GitHub Actions check identity missing/invalid for {workflow} at {head}"
        return True, "both fresh required checks passed for exact head"

    def dispatch(self, pr: dict, attempt: dict) -> None:
        head = pr["head"]["sha"]
        # Record pre-dispatch run IDs for recovery/deduplication; workflow dispatch
        # itself is only issued once per tuple and its acceptance is persisted.
        for workflow, field in (("ci.yml", "ci_run_id"), ("pr-quality.yml", "gate_run_id")):
            if attempt.get(field):
                continue
            before = {str(r.get("id")) for r in self.runs(workflow, head)}
            # OAuth-authenticated update-branch normally emits synchronize events.
            # Adopt an exact, active PR workflow instead of dispatching duplicate CI.
            active = [r for r in self.branch_runs(workflow, pr["head"]["ref"])
                      if r.get("event") == "pull_request" and r.get("status") in {"queued", "in_progress"}
                      and r.get("path", "").split("@", 1)[0] == f".github/workflows/{workflow}"
                      and any(x.get("head", {}).get("sha") == head and x.get("base", {}).get("sha") == attempt["base"] for x in r.get("pull_requests", []))]
            if active:
                if len(active) != 1:
                    attempt.update(status="failed", reason=f"multiple active {workflow} runs match candidate identity")
                    self.local.save(self._state)
                    raise DeliveryError(f"multiple active {workflow} runs match PR #{pr['number']} exact identity")
                attempt[field] = str(active[0]["id"])
                self.local.save(self._state)
                continue
            # Persist intent before mutation: an uncertain timeout must not cause
            # an automatic duplicate dispatch after restart.
            attempt[field] = {"before": sorted(before), "dispatched": int(time.time()), "ref": pr["head"]["ref"]}
            self.local.save(self._state)
            if workflow == "ci.yml":
                self.api.post(f"repos/{OWNER}/actions/workflows/{workflow}/dispatches", {"ref": pr["head"]["ref"]})
            else:
                self.api.post(f"repos/{OWNER}/actions/workflows/{workflow}/dispatches",
                              {"ref": pr["head"]["ref"], "inputs": {"pull_request": str(pr["number"])}})

    def recover_run_ids(self, pr: dict, attempt: dict) -> None:
        for workflow, field in (("ci.yml", "ci_run_id"), ("pr-quality.yml", "gate_run_id")):
            value = attempt.get(field)
            if isinstance(value, dict):
                runs = [r for r in self.runs(workflow, pr["head"]["sha"])
                        if str(r.get("id")) not in value["before"] and
                        r.get("event") == "workflow_dispatch" and r.get("head_sha") == pr["head"]["sha"]]
                if len(runs) == 1:
                    attempt[field] = str(runs[0]["id"])
                    self.local.save(self._state)
                elif len(runs) > 1:
                    attempt.update(status="failed", reason=f"duplicate {workflow} runs for exact candidate identity")
                    self.local.save(self._state)
                    raise DeliveryError(f"duplicate {workflow} dispatches for PR #{pr['number']} exact head; manual investigation required")
                elif time.time() - value.get("dispatched", time.time()) > DISPATCH_OBSERVATION_SECONDS:
                    attempt.update(status="paused", reason=f"{workflow} dispatch not observed; API outcome uncertain, inspect before any retry")
                    self.local.save(self._state)
                    return

    def apply(self, number: int | None = None, *, enqueue: bool = False) -> str:
        self.verify_repo()
        state = self.local.load()
        self._state = state
        if enqueue:
            if number is None:
                raise DeliveryError("--enqueue requires a PR number")
            pr = self.pull(number)
            if not self.valid_identity(pr):
                raise DeliveryError("enqueue accepts only open, non-draft, same-repository main PRs")
            sha = pr["head"]["sha"]
            p = subprocess.run(["gh", "pr", "merge", str(number), "--auto", "--squash", "--match-head-commit", sha], capture_output=True, text=True, timeout=20, check=False)
            if p.returncode:
                raise DeliveryError(f"explicit auto-merge opt-in failed for PR #{number}; no delivery started")
            return f"explicitly requested protected squash auto-merge for PR #{number} at {sha}"

        # Keep successful delivery serialized through the ordinary protected merge.
        # A passed candidate does not release the lane while its PR remains open.
        for ident, completed in state["attempts"].items():
            if completed.get("status") != "checks_passed" or (number is not None and completed.get("number") != number):
                continue
            current = self.pull(completed["number"])
            if current.get("merged") or current.get("merged_at"):
                completed.update(status="merged", merged_at=current.get("merged_at"))
                self.local.save(state)
                continue
            if current.get("state") != "open":
                completed.update(status="paused", reason="PR closed without protected merge")
                self.local.save(state)
                continue
            if not self.opted_in(completed["number"]):
                completed.update(status="paused", reason="auto-merge consent revoked while awaiting merge")
                self.local.save(state)
                continue
            return f"PR #{completed['number']} passed fresh checks and is awaiting its protected squash auto-merge; later PRs remain serialized"

        notices: list[str] = []
        prs = [p for p in self.list_prs() if (number is None or p["number"] == number) and self.valid_identity(p)]
        for listed in prs:
            n = listed["number"]
            if not self.opted_in(n):
                continue
            blocker = self.review_blocker(n)
            if blocker:
                notices.append(f"PR #{n} paused: {blocker}; owner/reviewer action required")
                continue
            pr = self.pull(n)
            blocker = self.review_blocker(n)
            if not self.valid_identity(pr) or not self.opted_in(n) or blocker:
                notices.append(f"PR #{n} paused: identity/consent/review changed ({blocker or 'recheck required'})")
                continue
            base, head = pr["base"]["sha"], pr["head"]["sha"]

            # Recover a pending async branch update by immutable old tuple before
            # considering a new update. Never repeat an accepted update request.
            pending = next(((k, a) for k, a in state["attempts"].items()
                            if a.get("number") == n and a.get("status") in {"updating", "updated_pending"}), None)
            if pending:
                _, update = pending
                main = self.api.get(f"repos/{OWNER}/git/ref/heads/main")["object"]["sha"]
                if base != update["base"] or main != update["base"]:
                    update.update(status="paused", reason="base advanced during asynchronous update")
                    self.local.save(state)
                    notices.append(f"PR #{n} paused: main/base advanced during update; reconcile fresh identity")
                    continue
                if head == update["head"]:
                    if update.get("status") == "updated_pending" and time.time() - update.get("updated_at", time.time()) > UPDATE_OBSERVATION_SECONDS:
                        update.update(status="paused", reason="asynchronous branch update exceeded observation bound")
                        self.local.save(state)
                        notices.append(f"PR #{n} update timed out at head {head}; owner must inspect, no retry issued")
                        continue
                    if update.get("status") == "updating":
                        update.update(status="paused", reason="update API outcome uncertain after restart; refusing duplicate request")
                        self.local.save(state)
                        notices.append(f"PR #{n} update outcome is uncertain at expected head {head}; inspect the PR/API state, no retry issued")
                        continue
                    return f"PR #{n} update remains pending for expected head {head}; no repeat update issued"
                if not HEX.fullmatch(head):
                    update.update(status="paused", reason="invalid observed head SHA")
                    self.local.save(state)
                    notices.append(f"PR #{n} paused: invalid updated head identity")
                    continue
                update.update(status="updated", resulting_head=head)
                ident = key(pr, base, head)
                candidate = state["attempts"].setdefault(ident, {"status": "checks", "number": n, "base": base, "head": head, "candidate_created_at": time.time()})
                self.local.save(state)
                self._state = state
                if not candidate.get("ci_run_id") and not candidate.get("ci_dispatch"):
                    self.dispatch(pr, candidate)
                    return f"PR #{n} update observed at {head}; fresh workflow dispatches requested once"
            else:
                ident = key(pr, base, head)
                candidate = state["attempts"].get(ident)
                if candidate and candidate.get("status") in {"failed", "paused", "checks_passed"}:
                    continue
                if not candidate or candidate.get("status") != "checks":
                    if not self.stale(pr):
                        continue
                    # Persist before mutation, then use GitHub's expected-head guard.
                    update = {"status": "updating", "number": n, "base": base, "head": head}
                    state["attempts"][ident] = update
                    self.local.save(state)
                    if self.pull(n)["head"]["sha"] != head or self.api.get(f"repos/{OWNER}/git/ref/heads/main")["object"]["sha"] != base:
                        update.update(status="paused", reason="head/base raced before update")
                        self.local.save(state)
                        notices.append(f"PR #{n} paused: head/base raced before update")
                        continue
                    try:
                        self.api.put(f"repos/{OWNER}/pulls/{n}/update-branch", {"expected_head_sha": head})
                    except DeliveryError:
                        observed = self.pull(n)
                        update.update(status="paused", reason="update rejected: conflict or head race",
                                      observed_head=observed["head"]["sha"], observed_base=observed["base"]["sha"])
                        self.local.save(state)
                        notices.append(f"PR #{n} update rejected; owner must inspect conflict/race at {observed['head']['sha']}")
                        continue
                    update["status"] = "updated_pending"
                    update["updated_at"] = time.time()
                    self.local.save(state)
                    return f"PR #{n} update accepted asynchronously for frozen base={base}, head={head}"

            # The updated candidate gets two explicit workflow_dispatch runs.
            candidate = state["attempts"][key(pr, base, head)]
            self._state = state
            self.recover_run_ids(pr, candidate)
            if candidate.get("status") in {"paused", "failed"}:
                notices.append(f"PR #{n} {candidate.get('reason', 'candidate paused; inspect persisted state')}")
                continue
            if not candidate.get("ci_run_id") or not candidate.get("gate_run_id"):
                self.dispatch(pr, candidate)
                self.local.save(state)
                return f"PR #{n} checks dispatch pending/accepted for exact head {head}"
            ok, reason = self.checks(pr, candidate)
            if not ok:
                if "failed" in reason or "invalid" in reason or "duplicate" in reason or "timed out" in reason:
                    candidate.update(status="failed", reason=reason)
                    self.local.save(state)
                    notices.append(f"PR #{n} {reason}")
                    continue
                return f"PR #{n} {reason}"
            main = self.api.get(f"repos/{OWNER}/git/ref/heads/main")["object"]["sha"]
            final = self.pull(n)
            if main != base or final["base"]["sha"] != base or final["head"]["sha"] != head or not self.opted_in(n) or not self.reviews_clear(n):
                candidate.update(status="paused", reason="base/head/consent/review changed after checks")
                self.local.save(state)
                notices.append(f"PR #{n} paused: identity, consent, or review changed after checks")
                continue
            candidate.update(status="checks_passed", reason=reason)
            self.local.save(state)
            return f"PR #{n} both fresh required checks passed at {head}; protected auto-merge remains authoritative"
        return "; ".join(notices) if notices else "no eligible stale opted-in PR; failed/paused identities were not retried"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default=OWNER)
    parser.add_argument("--state-dir", type=Path, default=Path.home() / ".local/state/rillcache-pr-delivery")
    parser.add_argument("--pr", type=int)
    parser.add_argument("--enqueue", action="store_true", help="explicitly request protected squash auto-merge for exactly --pr")
    parser.add_argument("--apply", action="store_true", help="perform at most one state-machine transition; default is read-only")
    parser.add_argument("--once", action="store_true", help="perform one bounded controller pass (requires --apply)")
    parser.add_argument("--watch", action="store_true", help="poll bounded transitions (requires --apply)")
    parser.add_argument("--interval", type=int, default=60)
    args = parser.parse_args(argv)
    if args.repo != OWNER:
        parser.error(f"--repo must be {OWNER}")
    if args.pr is not None and args.pr < 1:
        parser.error("--pr must be a positive PR number")
    if args.interval < 10 or args.interval > 3600:
        parser.error("--interval must be 10..3600 seconds")
    if args.watch and not args.apply or args.once and not args.apply:
        parser.error("--once and --watch require --apply")
    if args.watch and args.once:
        parser.error("choose either --once or --watch")
    if args.enqueue and (not args.apply or args.pr is None):
        parser.error("--enqueue requires --apply --pr NUMBER")
    if args.enqueue and args.watch:
        parser.error("--enqueue is a one-shot consent action and cannot be combined with --watch")
    local = Local(args.state_dir.expanduser().resolve())
    controller = Controller(Gh(), local)
    fd = None
    try:
        try:
            repo_root = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], text=True, capture_output=True, timeout=5, check=True).stdout.strip()).resolve()
            local.root.relative_to(repo_root)
        except ValueError:
            pass
        except (OSError, subprocess.SubprocessError) as exc:
            raise DeliveryError("cannot validate that state is outside the repository") from exc
        else:
            raise DeliveryError("state directory must be outside tracked repository source")
        if args.apply:
            fd = local.lock()
        if not args.apply:
            controller.verify_repo()
            print("Read-only mode: no API mutations. Eligible open PRs require same-repository, non-draft main target, explicit auto-merge and no unresolved changes request.")
            for pr in controller.list_prs():
                if (args.pr is not None and pr["number"] != args.pr) or not Controller.valid_identity(pr):
                    continue
                opt = controller.opted_in(pr["number"])
                is_stale = controller.stale(pr)
                print(f"PR #{pr['number']} head={pr['head']['sha']} base={pr['base']['sha']} opted_in={opt} stale={is_stale}")
            return 0
        while True:
            print(controller.apply(args.pr, enqueue=args.enqueue), flush=True)
            if not args.watch:
                break
            time.sleep(args.interval)
        return 0
    except DeliveryError as exc:
        print(f"pr-delivery: {exc}", file=sys.stderr)
        return 2
    finally:
        if fd is not None:
            os.close(fd)


if __name__ == "__main__":
    raise SystemExit(main())

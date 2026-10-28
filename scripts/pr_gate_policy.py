#!/usr/bin/env python3
"""Audit every repository workflow and live main gates; stdlib, read-only GETs.

The inventory deliberately names every PR job/matrix check. Adding a workflow,
job, matrix member or exception requires an explicit policy update, not a silent
advisory check. YAML checks enforce this repository's existing canonical layout;
this is not a general-purpose YAML parser.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
REPO = "macel94/rillcache"
RULESET_ID = 25863639
ACTIONS_APP_ID = 15368


class PolicyError(ValueError):
    pass


def ensure(value, message):
    if not value:
        raise PolicyError(message)


def load_inventory(root=ROOT):
    data = json.loads((root / ".github/pr-workflow-gates.json").read_text())
    ensure(type(data["schema_version"]) is int and data["schema_version"] == 1, "unknown gate inventory schema")
    return data["workflows"]


def required_workflows(root=ROOT):
    return {path: tuple(check for checks in item["jobs"].values() for check in checks)
            for path, item in load_inventory(root).items() if item["jobs"]}


def required_checks(root=ROOT):
    return [{"context": check, "integration_id": ACTIONS_APP_ID}
            for checks in required_workflows(root).values() for check in checks]


def rule_parameters(policy):
    ensure(policy["target"] == "branch" and policy["enforcement"] == "active", "main ruleset is not active")
    ensure(policy["bypass_actors"] == [], "main has bypass actors")
    ensure(policy["conditions"] == {"ref_name": {"include": ["refs/heads/main"], "exclude": []}}, "main target/exclusions differ")
    rules = {rule["type"]: rule.get("parameters", {}) for rule in policy["rules"]}
    ensure({"deletion", "non_fast_forward", "required_linear_history", "pull_request", "required_status_checks"} <= rules.keys(), "main safety rules missing")
    pr = rules["pull_request"]
    ensure(pr["allowed_merge_methods"] == ["squash"] and pr["require_code_owner_review"] is True
           and pr["required_review_thread_resolution"] is True, "review/squash protections relaxed")
    status = rules["required_status_checks"]
    ensure(status["strict_required_status_checks_policy"] is True and status["do_not_enforce_on_create"] is False,
           "required checks are not strictly up-to-date/enforced")
    return status["required_status_checks"]


def workflow_blocks(text):
    events = re.search(r"^on:\n(.*?)(?=^\S|\Z)", text, re.M | re.S)
    ensure(events is not None, "workflow must use canonical on: mapping")
    triggers = re.findall(r"^  ([a-z_]+):", events[1], re.M)
    jobs = re.search(r"^jobs:\n(.*)", text, re.M | re.S)
    ensure(jobs is not None, "workflow jobs missing")
    blocks = re.findall(r"^  ([a-zA-Z0-9_-]+):\n(.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)", jobs[1], re.M | re.S)
    ensure(len({key for key, _ in blocks}) == len(blocks), "duplicate workflow job")
    return triggers, dict(blocks)


def resolved_checks(path, job, block):
    name = re.search(r"^    name: ([^\n]+)$", block, re.M)
    if path == "container.yml" and job == "build":
        ensure(name is None, "container check naming changed without policy audit")
        rows = re.findall(r"- arch: (\S+)\n\s+runner: (\S+)", block)
        ensure(rows, "container architecture matrix missing")
        return [f"build ({arch}, {runner})" for arch, runner in rows]
    if path == "upstream-compatibility.yml" and job == "suites":
        ensure(name and name[1] == "Tcl ${{ matrix.reference }} / ${{ matrix.suite.id }}",
               "Tcl check naming changed without policy audit")
        references = re.search(r"^        reference: \[([^]]+)\]", block, re.M)
        suites = re.findall(r"- \{id: ([a-zA-Z0-9_-]+), path: [^}]+\}", block)
        ensure(references and suites, "Tcl reference/suite matrix missing")
        return [f"Tcl {reference.strip()} / {suite}" for reference in references[1].split(",") for suite in suites]
    ensure("    strategy:" not in block and "      matrix:" not in block, f"uncatalogued matrix in {path}/{job}")
    return [name[1] if name else job]


def audit_local(root=ROOT):
    inventory = load_inventory(root)
    files = {p.name for p in (root / ".github/workflows").iterdir() if p.suffix in (".yml", ".yaml")}
    ensure(files == set(inventory), f"uncatalogued/missing workflow files: {sorted(files ^ set(inventory))}")
    all_checks = []
    non_required = []
    for path, item in inventory.items():
        text = (root / ".github/workflows" / path).read_text()
        triggers, jobs = workflow_blocks(text)
        ensure(triggers == item["events"], f"workflow events changed without audit: {path}")
        ensure("pull_request_target" not in triggers, f"privileged PR trigger forbidden: {path}")
        if "pull_request" not in triggers:
            ensure(not item["jobs"] and item.get("non_pr_reason"), f"non-PR workflow classification missing: {path}")
            for job, block in jobs.items():
                name = re.search(r"^    name: ([^\n]+)$", block, re.M)
                ensure(not name or "${{" not in name[1], f"uncatalogued optional check-name template: {path}/{job}")
                non_required.append((name[1] if name else job, "    strategy:" in block))
            continue
        ensure(item["jobs"], f"PR workflow has no required checks: {path}")
        ensure("paths:" not in text and "paths-ignore:" not in text, f"PR workflow has path exemptions: {path}")
        exempt = item["non_pr_jobs"]
        ensure(set(jobs) == set(item["jobs"]) | set(exempt), f"uncatalogued/missing PR jobs: {path}")
        for job, checks in item["jobs"].items():
            block = jobs[job]
            ensure(checks == resolved_checks(path, job, block), f"required job/matrix contexts differ: {path}/{job}")
            ensure("continue-on-error:" not in block, f"required job masks failures: {path}/{job}")
            condition = re.search(r"^    if:[ \t]*(.*)$", block, re.M)
            if path == "pr-branch-updater.yml" and job == "read-only-self-tests":
                ensure(condition and condition[1] == "github.event_name == 'pull_request' || (github.event_name == 'workflow_dispatch' && github.ref != 'refs/heads/main')",
                       "policy self-tests must execute for PRs and read-only branch dispatch")
                ensure("contents: write" not in block and "pull-requests: write" not in block,
                       "untrusted policy self-tests have repository write permissions")
            else:
                ensure(condition is None, f"required PR job can be skipped by a condition: {path}/{job}")
            all_checks.extend(checks)
        for job, kind in exempt.items():
            name = re.search(r"^    name: ([^\n]+)$", jobs[job], re.M)
            non_required.append((name[1] if name else job, "    strategy:" in jobs[job]))
            if kind == "not-pull-request":
                ensure("github.event_name != 'pull_request'" in jobs[job], f"publication/writer can execute on a PR: {path}/{job}")
            elif kind == "needs-publish":
                ensure("    needs: publish\n" in jobs[job] and exempt.get("publish") == "not-pull-request",
                       f"publication dependent exemption is not guarded: {path}/{job}")
            else:
                raise PolicyError(f"unknown non-PR job exemption: {path}/{job}")
    ensure(len(all_checks) == len(set(all_checks)), "required check names collide across workflows/jobs")
    ensure(all(not any(check == name or (matrix and check.startswith(name + " (")) for check in all_checks)
               for name, matrix in non_required), "optional/publication job collides with a required check context")
    policy = json.loads((root / ".github/main-ruleset.json").read_text())
    ensure(rule_parameters(policy) == required_checks(root), "tracked ruleset omits a PR workflow/job or differs from inventory")
    return inventory


class GitHub:
    def get(self, path):
        ensure(not path.startswith("/") and ".." not in path, "unsafe API path")
        result = subprocess.run(["gh", "api", "--method", "GET", f"repos/{REPO}/{path}"],
                                capture_output=True, text=True, timeout=20)
        ensure(result.returncode == 0, f"read-only GitHub query failed: {path.split('?')[0]}")
        ensure(len(result.stdout) <= 2_000_000, "GitHub response too large")
        return json.loads(result.stdout)


def audit_live(api, root=ROOT):
    inventory = audit_local(root)
    ruleset = api.get(f"rulesets/{RULESET_ID}")
    live = rule_parameters(ruleset)
    expected = required_checks(root)
    missing = [check for check in expected if check not in live]
    ensure(not missing, "LIVE RULESET DRIFT: missing/wrong-app required checks: " + ", ".join(c["context"] for c in missing))
    effective = api.get("rules/branches/main")
    enforced = [check for rule in effective if rule["type"] == "required_status_checks"
                and rule["parameters"]["strict_required_status_checks_policy"] is True
                for check in rule["parameters"]["required_status_checks"]]
    ensure(all(check in enforced for check in expected), "effective main branch rules do not enforce every required check")
    response = api.get("actions/workflows?per_page=100")
    ensure(response["total_count"] <= 100, "workflow inventory exceeded bounded observation; audit needs pagination")
    workflows = {item["path"]: item for item in response["workflows"]}
    for path in required_workflows(root):
        item = workflows.get(f".github/workflows/{path}")
        ensure(item is not None and item["state"] == "active", f"required PR workflow is absent/disabled live: {path}")
    return inventory


def render(inventory):
    lines = ["## Actions merge-gate audit", "", "| Workflow | Required PR check(s) / scope |", "| --- | --- |"]
    for path, item in inventory.items():
        names = [name for checks in item["jobs"].values() for name in checks]
        scope = "; ".join(names) if names else item["non_pr_reason"]
        lines.append(f"| `{path}` | {scope} |")
    lines += ["", "Missing checks, wrong GitHub App, disabled PR workflows, bypass actors, path exemptions, "
              "uncatalogued jobs/matrix members or live/tracked policy drift fail this audit. "
              "Manual/scheduled studies and PR-inapplicable publication jobs are not fake PR passes.", ""]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true", help="also verify effective GitHub rules and workflow activation (GET only)")
    args = parser.parse_args()
    try:
        inventory = audit_live(GitHub()) if args.live else audit_local()
        print(render(inventory))
        print("Live protection verified." if args.live else "Local workflow/policy coverage verified (not live enforcement).")
        return 0
    except (PolicyError, KeyError, TypeError, OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"PR gate audit FAILED: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

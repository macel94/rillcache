#!/usr/bin/env python3
"""Validate the production acceptance registry and findings log (V3).

Fails on malformed rows, on a signed row without a named reviewer, a review
scope, evidence or a full candidate SHA, on a maintainer self-signing the
independent security scope, on a signed row that references not-found or still
open findings, on a closed finding without retest evidence, and - with
--require-all - on any scope that is still unassigned or withheld.

`--outstanding` prints the exact third-party signatures that are still missing
and exits 0: it records the residual external dependency instead of hiding it.

Standard library only.
"""

from __future__ import annotations

import argparse
import datetime
import os
import sys

HEADER = [
    "candidate",
    "scope",
    "reviewer",
    "reviewer_kind",
    "evidence",
    "review_scope",
    "findings",
    "signed_at",
    "state",
]
SCOPES = ("profile", "security", "performance", "recovery", "release")
KINDS = ("maintainer", "independent")
STATES = ("signed", "unassigned", "withheld")
# Scope -> the reviewer kind that may accept it, and how its signature is named.
REQUIRED_REVIEWERS = {
    "profile": ("maintainer or independent reviewer", ("maintainer", "independent")),
    "security": ("independent reviewer only", ("independent",)),
    "performance": ("maintainer or independent reviewer", ("maintainer", "independent")),
    "recovery": ("maintainer or independent reviewer", ("maintainer", "independent")),
    "release": ("release owner", ("maintainer", "independent")),
}
FINDINGS_HEADER = [
    "id",
    "scope",
    "summary",
    "severity",
    "disposition",
    "retest_evidence",
    "status",
]
FINDING_SEVERITIES = ("low", "medium", "high", "critical")
FINDING_STATUSES = ("open", "closed")


def _parse_date(value: str) -> bool:
    try:
        datetime.date.fromisoformat(value)
    except ValueError:
        return False
    return True


def _evidence_exists(root: str, evidence: str) -> bool:
    if evidence.startswith(("http://", "https://")):
        return True
    return os.path.exists(os.path.join(root, evidence))


def _rows(registry: str, header: list[str]) -> tuple[list[str], list[list[str]]]:
    with open(registry, encoding="utf-8") as handle:
        lines = [line.rstrip("\n") for line in handle if line.strip()]
    if not lines:
        return [], []
    if lines[0].split("\t") != header:
        return lines[0].split("\t"), []
    return header, [line.split("\t") for line in lines[1:]]


def findings_problems(findings_path: str, root: str) -> tuple[list[str], dict[str, str]]:
    """Validate the findings log and return its problems and id -> status map."""
    problems: list[str] = []
    try:
        header, rows = _rows(findings_path, FINDINGS_HEADER)
    except FileNotFoundError:
        return [f"findings log not found: {findings_path}"], {}
    if header != FINDINGS_HEADER:
        return [f"bad findings header: {header!r} (expected {FINDINGS_HEADER!r})"], {}
    statuses: dict[str, str] = {}
    for number, fields in enumerate(rows, start=2):
        if len(fields) != len(FINDINGS_HEADER):
            problems.append(f"findings line {number}: expected {len(FINDINGS_HEADER)} fields")
            continue
        identifier, scope, summary, severity, disposition, retest, status = fields
        if not identifier or identifier in statuses:
            problems.append(f"findings line {number}: missing or duplicate id {identifier!r}")
        if scope not in SCOPES:
            problems.append(f"findings line {number}: unknown scope {scope!r}")
        if not summary.strip():
            problems.append(f"findings line {number}: empty summary")
        if severity not in FINDING_SEVERITIES:
            problems.append(f"findings line {number}: unknown severity {severity!r}")
        if status not in FINDING_STATUSES:
            problems.append(f"findings line {number}: unknown status {status!r}")
        if status == "closed":
            if not disposition.strip() or disposition.strip() == "-":
                problems.append(f"findings line {number}: closed finding needs a disposition")
            if not retest.strip() or retest.strip() == "-":
                problems.append(f"findings line {number}: closed finding needs retest evidence")
            elif not _evidence_exists(root, retest):
                problems.append(f"findings line {number}: retest evidence not found: {retest}")
        statuses[identifier] = status
    return problems, statuses


def check(registry: str, root: str, require_all: bool, findings: str | None = None) -> list[str]:
    problems: list[str] = []
    findings_path = findings or os.path.join(os.path.dirname(registry), "findings.tsv")
    finding_problems, findings_status = findings_problems(findings_path, root)
    problems.extend(finding_problems)

    try:
        header, rows = _rows(registry, HEADER)
    except FileNotFoundError:
        return [f"registry not found: {registry}"]
    if not rows and header != HEADER:
        return [f"bad header: {header!r} (expected {HEADER!r})"]

    seen: dict[str, str] = {}
    for number, fields in enumerate(rows, start=2):
        if len(fields) != len(HEADER):
            problems.append(f"line {number}: expected {len(HEADER)} fields, got {len(fields)}")
            continue
        candidate, scope, reviewer, kind, evidence, review_scope, referenced, signed_at, state = fields
        if scope not in SCOPES:
            problems.append(f"line {number}: unknown scope {scope!r}")
        if kind not in KINDS:
            problems.append(f"line {number}: unknown reviewer_kind {kind!r}")
        if state not in STATES:
            problems.append(f"line {number}: unknown state {state!r}")
        seen.setdefault(scope, state)

        if state != "signed":
            continue
        if len(candidate) != 40 or any(c not in "0123456789abcdef" for c in candidate):
            problems.append(f"line {number}: candidate must be a full lowercase 40-char SHA")
        if not reviewer or reviewer.lower() in {"tbd", "none", "unassigned"}:
            problems.append(f"line {number}: signed row needs a named reviewer")
        if not review_scope.strip() or review_scope.strip() == "-":
            problems.append(f"line {number}: signed row needs a review_scope statement")
        if not _parse_date(signed_at):
            problems.append(f"line {number}: signed_at must be YYYY-MM-DD")
        if not _evidence_exists(root, evidence):
            problems.append(f"line {number}: evidence not found: {evidence}")
        allowed = REQUIRED_REVIEWERS.get(scope, ("any", ("maintainer", "independent")))[1]
        if kind not in allowed:
            problems.append(
                f"line {number}: scope {scope!r} requires {REQUIRED_REVIEWERS[scope][0]}"
            )
        for identifier in [item.strip() for item in referenced.split(",") if item.strip() not in ("", "-")]:
            status = findings_status.get(identifier)
            if status is None:
                problems.append(f"line {number}: finding {identifier!r} is not in the findings log")
            elif status != "closed":
                problems.append(f"line {number}: finding {identifier!r} is still open")

    missing = [scope for scope in SCOPES if scope not in seen]
    if require_all and missing:
        problems.append(f"no registry row for scope(s): {', '.join(missing)}")
    if require_all:
        unaccepted = [s for s in SCOPES if seen.get(s) in {"unassigned", "withheld"}]
        if unaccepted:
            problems.append(f"scope(s) not yet accepted: {', '.join(unaccepted)}")
    return problems


def outstanding(registry: str) -> list[str]:
    """Return one line per scope whose acceptance is still outstanding."""
    try:
        header, rows = _rows(registry, HEADER)
    except FileNotFoundError:
        return [f"{scope}\tmissing registry row for {REQUIRED_REVIEWERS[scope][0]}" for scope in SCOPES]
    seen = {fields[1]: fields for fields in rows if len(fields) == len(HEADER)}
    result = []
    for scope in SCOPES:
        fields = seen.get(scope)
        if fields is not None and fields[8] == "signed":
            continue
        state = fields[8] if fields is not None else "unassigned"
        result.append(
            f"{scope}\t{state}\tneeds {REQUIRED_REVIEWERS[scope][0]} signature"
        )
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--registry",
        default="docs/acceptance/signoffs.tsv",
        help="registry path relative to --root",
    )
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument(
        "--findings",
        default=None,
        help="findings log path (default: findings.tsv beside the registry)",
    )
    parser.add_argument(
        "--require-all",
        action="store_true",
        help="also fail when a scope is unassigned or withheld (release gate)",
    )
    parser.add_argument(
        "--outstanding",
        action="store_true",
        help="list the third-party signatures that are still missing and exit 0",
    )
    args = parser.parse_args(argv)

    registry = os.path.join(args.root, args.registry)
    if args.outstanding:
        for line in outstanding(registry):
            print(line)
        return 0
    findings = os.path.join(args.root, args.findings) if args.findings else None
    problems = check(registry, args.root, args.require_all, findings)
    for problem in problems:
        print(f"FAIL: {problem}")
    if problems:
        print(f"{len(problems)} acceptance problem(s) found")
        return 1
    print("acceptance registry is valid")
    return 0


if __name__ == "__main__":
    sys.exit(main())

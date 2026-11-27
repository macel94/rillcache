#!/usr/bin/env python3
"""Validate the production acceptance registry (V3).

Fails on malformed rows, on a signed row without a named reviewer, evidence or a
full candidate SHA, on a maintainer self-signing the independent security scope,
and - with --require-all - on any scope that is still unassigned or withheld.

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
    "signed_at",
    "state",
]
SCOPES = ("profile", "security", "performance", "recovery", "release")
KINDS = ("maintainer", "independent")
STATES = ("signed", "unassigned", "withheld")


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


def check(registry: str, root: str, require_all: bool) -> list[str]:
    problems: list[str] = []
    try:
        with open(registry, encoding="utf-8") as handle:
            lines = [line.rstrip("\n") for line in handle if line.strip()]
    except FileNotFoundError:
        return [f"registry not found: {registry}"]

    if not lines:
        return ["registry is empty"]

    header = lines[0].split("\t")
    if header != HEADER:
        problems.append(f"bad header: {header!r} (expected {HEADER!r})")
        return problems

    seen: dict[str, str] = {}
    for number, line in enumerate(lines[1:], start=2):
        fields = line.split("\t")
        if len(fields) != len(HEADER):
            problems.append(f"line {number}: expected {len(HEADER)} fields, got {len(fields)}")
            continue
        candidate, scope, reviewer, kind, evidence, signed_at, state = fields
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
        if not _parse_date(signed_at):
            problems.append(f"line {number}: signed_at must be YYYY-MM-DD")
        if not _evidence_exists(root, evidence):
            problems.append(f"line {number}: evidence not found: {evidence}")
        if scope == "security" and kind == "maintainer":
            problems.append(
                f"line {number}: the security scope cannot be self-signed by a maintainer"
            )

    missing = [scope for scope in SCOPES if scope not in seen]
    if require_all and missing:
        problems.append(f"no registry row for scope(s): {', '.join(missing)}")
    if require_all:
        unaccepted = [s for s in SCOPES if seen.get(s) in {"unassigned", "withheld"}]
        if unaccepted:
            problems.append(f"scope(s) not yet accepted: {', '.join(unaccepted)}")

    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--registry",
        default="docs/acceptance/signoffs.tsv",
        help="registry path relative to --root",
    )
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument(
        "--require-all",
        action="store_true",
        help="also fail when a scope is unassigned or withheld (release gate)",
    )
    args = parser.parse_args(argv)

    problems = check(
        os.path.join(args.root, args.registry), args.root, args.require_all
    )
    for problem in problems:
        print(f"FAIL: {problem}")
    if problems:
        print(f"{len(problems)} acceptance problem(s) found")
        return 1
    print("acceptance registry is valid")
    return 0


if __name__ == "__main__":
    sys.exit(main())

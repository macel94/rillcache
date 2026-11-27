#!/usr/bin/env python3
"""Negative and positive controls for scripts/check_acceptance.py (V3).

Each case writes a temporary registry and asserts the checker's exit code, so a
regression that would silently accept a self-signed security review or a
malformed row fails this test instead.

Standard library only.
"""

from __future__ import annotations

import importlib.util
import os
import pathlib
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CHECKER = ROOT / "scripts" / "check_acceptance.py"
CANDIDATE = "d19c37cc9b0bb5398b8356d05576f78a73fdbc3c"
HEADER = "candidate\tscope\treviewer\treviewer_kind\tevidence\tsigned_at\tstate\n"


def load_checker():
    spec = importlib.util.spec_from_file_location("check_acceptance", CHECKER)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def run(rows: str, require_all: bool = False) -> list[str]:
    module = load_checker()
    with tempfile.NamedTemporaryFile("w", suffix=".tsv", delete=False) as handle:
        handle.write(HEADER + rows)
        path = handle.name
    try:
        return module.check(path, str(ROOT), require_all)
    finally:
        os.unlink(path)


def row(scope: str, reviewer: str, kind: str, evidence: str, date: str, state: str) -> str:
    return f"{CANDIDATE}\t{scope}\t{reviewer}\t{kind}\t{evidence}\t{date}\t{state}\n"


def case(name: str, rows: str, expected_problems: int, require_all: bool = False) -> None:
    problems = run(rows, require_all)
    if len(problems) != expected_problems:
        print(f"FAIL {name}: expected {expected_problems} problem(s), got {problems}")
        raise SystemExit(1)
    print(f"PASS {name} ({expected_problems} problem(s))")


def main() -> int:
    base = (
        row("profile", "Reviewer A", "maintainer", "docs/compatibility.md", "2026-11-27", "signed")
        + row("performance", "-", "maintainer", "docs/benchmark-reports.md", "-", "withheld")
        + row("recovery", "-", "maintainer", "docs/persistence.md", "-", "withheld")
        + row("release", "Reviewer A", "maintainer", "docs/production-readiness-gaps.md", "2026-11-27", "signed")
        + row("security", "-", "independent", "docs/release-acceptance.md", "-", "unassigned")
    )

    case("valid registry with open scopes passes plain mode", base, 0)
    case("release mode refuses while scopes are open", base, 1, require_all=True)

    self_signed = base.replace(
        row("security", "-", "independent", "docs/release-acceptance.md", "-", "unassigned"),
        row("security", "Reviewer A", "maintainer", "docs/release-acceptance.md", "2026-11-27", "signed"),
    )
    case("maintainer self-signing security is rejected", self_signed, 1)

    missing_evidence = base.replace(
        row("profile", "Reviewer A", "maintainer", "docs/compatibility.md", "2026-11-27", "signed"),
        row("profile", "Reviewer A", "maintainer", "docs/does-not-exist.md", "2026-11-27", "signed"),
    )
    case("signed row with missing evidence is rejected", missing_evidence, 1)

    bad_candidate = base.replace(
        row("profile", "Reviewer A", "maintainer", "docs/compatibility.md", "2026-11-27", "signed"),
        "abc123\tprofile\tReviewer A\tmaintainer\tdocs/compatibility.md\t2026-11-27\tsigned\n",
    )
    case("abbreviated candidate SHA is rejected", bad_candidate, 1)

    unnamed = base.replace(
        row("profile", "Reviewer A", "maintainer", "docs/compatibility.md", "2026-11-27", "signed"),
        row("profile", "tbd", "maintainer", "docs/compatibility.md", "2026-11-27", "signed"),
    )
    case("placeholder reviewer is rejected", unnamed, 1)

    bad_date = base.replace(
        row("profile", "Reviewer A", "maintainer", "docs/compatibility.md", "2026-11-27", "signed"),
        row("profile", "Reviewer A", "maintainer", "docs/compatibility.md", "27.11.2026", "signed"),
    )
    case("non-ISO sign-off date is rejected", bad_date, 1)

    missing_scope = base.replace(
        row("release", "Reviewer A", "maintainer", "docs/production-readiness-gaps.md", "2026-11-27", "signed"), ""
    )
    # Two independent problems: the missing scope row, plus the still-open scopes.
    case("release mode rejects a registry without a scope row", missing_scope, 2, require_all=True)

    print("all acceptance-registry controls passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())

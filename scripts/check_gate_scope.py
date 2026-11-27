#!/usr/bin/env python3
"""Check that every required merge gate is mapped to the property it proves (V7).

`.github/pr-workflow-gates.json` defines the required contexts. This checker keeps
the published mapping honest:

* every required context appears exactly once in the gate table of
  `docs/gate-scope.md`, with a non-empty "Proves" and "Does not prove" cell,
* the document keeps the explicit "what no merge gate proves" and "production
  gates still missing" statements,
* every checklist identifier named in the missing-gate section is still open in
  the canonical checklist (a closed gap may not be advertised as a missing gate),
* the document never claims a merge gate proves production readiness.

Usage:
    python3 scripts/check_gate_scope.py [--root <repo>]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

GATES = Path(".github/pr-workflow-gates.json")
SCOPE_DOC = Path("docs/gate-scope.md")
CHECKLIST = Path("docs/production-readiness-gaps.md")
GAP_RE = re.compile(r"^- \[ \] \*\*([A-Z]\d+)\.\s.*?\[(P\d)\]", re.M)
CLAIM_RE = re.compile(
    r"(proves|prove|proving|guarantees?)\s+production readiness", re.IGNORECASE
)


class Invalid(Exception):
    pass


def required_contexts(root: Path) -> list[str]:
    data = json.loads((root / GATES).read_text(encoding="utf-8"))
    names: list[str] = []
    for spec in data.get("workflows", {}).values():
        for value in spec.get("jobs", {}).values():
            if isinstance(value, list):
                names.extend(name for name in value if isinstance(name, str))
    return names


def table_rows(text: str) -> dict[str, tuple[str, str]]:
    rows: dict[str, tuple[str, str]] = {}
    for line in text.splitlines():
        if not line.startswith("| `"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) != 3:
            continue
        name = cells[0].strip("`")
        rows[name] = (cells[1], cells[2])
    return rows


def validate(root: Path) -> int:
    contexts = required_contexts(root)
    if not contexts:
        raise Invalid(f"{GATES} defines no required contexts")
    text = (root / SCOPE_DOC).read_text(encoding="utf-8")
    rows = table_rows(text)

    for name in contexts:
        if name not in rows:
            raise Invalid(f"required context {name!r} is not mapped in {SCOPE_DOC}")
        proves, not_proves = rows[name]
        if not proves or not not_proves:
            raise Invalid(f"context {name!r} must state both what it proves and what it does not")
    extra = set(rows) - set(contexts)
    if extra:
        raise Invalid(
            f"{SCOPE_DOC} maps contexts that are not required: {sorted(extra)}"
        )

    for required in (
        "## What no merge gate proves",
        "## Production gates still missing",
    ):
        if required not in text:
            raise Invalid(f"{SCOPE_DOC} is missing the section {required!r}")

    open_gaps = {gap for gap, _ in GAP_RE.findall((root / CHECKLIST).read_text(encoding="utf-8"))}
    missing_section = text.split("## Production gates still missing", 1)[1]
    missing_section = missing_section.split("## Where the production evidence lives", 1)[0]
    named = set(re.findall(r"`([A-Z]\d+)`", missing_section))
    if not named:
        raise Invalid("the missing-gate section names no checklist identifier")
    closed = named - open_gaps
    if closed:
        raise Invalid(
            f"the missing-gate section advertises gaps that are closed: {sorted(closed)}"
        )

    for match in CLAIM_RE.finditer(text):
        line_start = text.rfind("\n", 0, match.start()) + 1
        line = text[line_start : text.find("\n", match.end())]
        if "does not" in line or "not prove" in line or "no combination" in line:
            continue
        raise Invalid(f"the gate map claims a production pass: {line.strip()!r}")

    return len(contexts)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".")
    args = parser.parse_args(argv)
    try:
        count = validate(Path(args.root).resolve())
    except Invalid as error:
        print(f"Invalid gate scope: {error}", file=sys.stderr)
        return 1
    print(f"Gate scope valid: {count} required contexts mapped, no production claim")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

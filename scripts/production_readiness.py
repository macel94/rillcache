#!/usr/bin/env python3
"""Check readiness documentation and report blockers; never authorize a release."""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHECKLIST = Path("docs/production-readiness-gaps.md")
ENTRY_POINTS = (Path("README.md"), Path("docs/README.md"))
AREAS = "CDASORPLV"
HEADING = re.compile(r"## ([1-9])\. .+")
ITEM = re.compile(r"- \[([ x])\] \*\*([A-Z][1-9][0-9]*)\. (.+?)\*\* \[(P[012])\]")
# A merge can glue the next item onto a paragraph ("text.- [ ] **O7. ..."), which
# parses as prose and silently removes the gap from the inventory.
GLUED = re.compile(r"(?<!\n)\S- \[[ x]\] \*\*([A-Z][1-9][0-9]*)\.")
STATUS = re.compile(r"\*\*Status: (NOT PRODUCTION READY|PRODUCTION READY)\.\*\*")


@dataclass(frozen=True)
class Gap:
    id: str
    title: str
    priority: str
    line: int
    open: bool


def inventory(source: str) -> tuple[str, list[Gap]]:
    status_lines = [line for line in source.splitlines() if line.startswith("**Status:")]
    status = STATUS.fullmatch(status_lines[0]) if len(status_lines) == 1 else None
    if status is None:
        raise ValueError("expected exactly one valid readiness status line")
    gaps: list[Gap] = []
    seen: set[str] = set()
    sections: list[str] = []
    populated: set[str] = set()
    area = None
    for line_no, line in enumerate(source.splitlines(), 1):
        glued = GLUED.search(line)
        if glued is not None:
            raise ValueError(
                f"line {line_no}: checklist item {glued[1]} is glued onto the previous "
                "text (missing newline); split it onto its own line")
        if line.startswith("## "):
            heading = HEADING.fullmatch(line)
            area = AREAS[int(heading[1]) - 1] if heading else None
            if area is not None:
                sections.append(area)
        if not line.startswith("- [") or re.match(r"- \[[^\]]+\]\(", line):
            continue
        match = ITEM.fullmatch(line)
        if not match:
            raise ValueError(f"line {line_no}: malformed checklist item")
        checked, ident, title, priority = match.groups()
        if ident[0] != area or ident in seen:
            raise ValueError(f"line {line_no}: duplicate or misplaced gap {ident}")
        seen.add(ident)
        populated.add(area)
        gaps.append(Gap(ident, title, priority, line_no, checked == " "))
    if sections != list(AREAS) or populated != set(AREAS):
        raise ValueError("expected all nine populated readiness sections in canonical order")
    # A merge can glue the next item onto a paragraph, which parses as prose and
    # silently drops that gap from the inventory. Require contiguous ids per area.
    for area in AREAS:
        numbers = sorted(int(gap.id[len(area):]) for gap in gaps if gap.id.startswith(area))
        if numbers != list(range(1, len(numbers) + 1)):
            expected = ", ".join(f"{area}{index}" for index in range(1, max(numbers, default=0) + 1))
            raise ValueError(
                f"gap ids for area {area} are not contiguous (expected {expected}); "
                "check for a checklist item glued onto the previous paragraph")
    if status[1] == "PRODUCTION READY" and any(is_blocker(gap) for gap in gaps):
        raise ValueError("production-ready status contradicts open P0/P1 items")
    return status[1], gaps


def is_blocker(gap: Gap) -> bool:
    return gap.open and gap.priority in {"P0", "P1"}


def inspect(root: Path) -> tuple[str, list[Gap]]:
    status, gaps = inventory((root / CHECKLIST).read_text(encoding="utf-8"))
    for entry in ENTRY_POINTS:
        target = CHECKLIST.relative_to(entry.parent).as_posix()
        text = (root / entry).read_text(encoding="utf-8")
        if not re.search(r"\[[^\]\n]+\]\(" + re.escape(target) + r"\)", text):
            raise ValueError(f"{entry}: missing direct link to {target}")
    return status, gaps


def main(argv: list[str] | None = None, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="check document integrity only")
    parser.add_argument("--json", action="store_true", help="write the blocker inventory as JSON")
    args = parser.parse_args(argv)
    try:
        status, gaps = inspect(root)
    except (OSError, ValueError) as error:
        print(f"readiness documentation invalid: {error}", file=sys.stderr)
        return 2
    blockers = sorted((gap for gap in gaps if is_blocker(gap)), key=lambda gap: gap.priority)
    if args.json:
        print(json.dumps({
            "declared_status": status,
            "checklist": CHECKLIST.as_posix(),
            "blockers": [asdict(gap) for gap in blockers],
            "open_followups": [asdict(gap) for gap in gaps if gap.open and not is_blocker(gap)],
            "release_evidence_verified": False,
        }, indent=2))
    elif args.check:
        print(f"Readiness documents valid: {len(gaps)} gaps tracked; no release approval implied.")
    else:
        print(f"Declared status: {status}")
        for gap in blockers:
            print(f"{gap.priority} {gap.id}: {gap.title} ({CHECKLIST}:{gap.line})")
        print(f"{len(blockers)} open P0/P1 items; release evidence and approvals not verified.")
    return int(not args.check and (bool(blockers) or status == "NOT PRODUCTION READY"))


if __name__ == "__main__":
    sys.exit(main())

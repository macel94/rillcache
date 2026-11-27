#!/usr/bin/env python3
"""Validate release notes against the candidate's real state (readiness L4).

Every published version must carry notes that are generated from the candidate
and that stay honest as the tree moves:

* `CHANGELOG.md` names the version and links `docs/releases/v<version>.md`,
* the notes contain the sections the release contract requires,
* **Accepted limitations** lists exactly the open P0 gap ids from the canonical
  checklist -- listing a closed gap, or omitting an open one, fails,
* the verification section names the exercised upgrade/rollback drills and every
  command path it references exists in the tree, and
* any journal/format change statement is accompanied by a migration *and* a
  rollback statement that mention the journal.

Usage:
    python3 scripts/check_release_notes.py [--root <repo>]
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

CHECKLIST = Path("docs/production-readiness-gaps.md")
CHANGELOG = Path("CHANGELOG.md")
REQUIRED_SECTIONS = (
    "Compatibility profile",
    "Migration",
    "Rollback",
    "Support window",
    "Accepted limitations",
    "Verification",
)
REQUIRED_DRILLS = ("scripts/upgrade_drill.py", "scripts/restore_drill.py")
VERSION_RE = re.compile(r"^## (\d+\.\d+\.\d+)\b", re.M)
GAP_RE = re.compile(r"^- \[ \] \*\*([A-Z]\d+)\.\s.*?\[(P\d)\]", re.M)
PATH_RE = re.compile(r"`(scripts/[A-Za-z0-9_./-]+)`")


class Invalid(Exception):
    pass


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise Invalid(message)


def open_gaps(root: Path, priority: str) -> set[str]:
    text = (root / CHECKLIST).read_text(encoding="utf-8")
    return {gap for gap, prio in GAP_RE.findall(text) if prio == priority}


def sections(text: str) -> dict[str, str]:
    found: dict[str, str] = {}
    current = None
    body: list[str] = []
    for line in text.splitlines():
        if line.startswith("## "):
            if current is not None:
                found[current] = "\n".join(body).strip()
            current = line[3:].strip()
            body = []
        elif current is not None:
            body.append(line)
    if current is not None:
        found[current] = "\n".join(body).strip()
    return found


def validate(root: Path) -> list[str]:
    changelog = (root / CHANGELOG).read_text(encoding="utf-8")
    versions = VERSION_RE.findall(changelog)
    _require(versions, "CHANGELOG.md has no versioned entry")

    p0 = open_gaps(root, "P0")
    p1 = open_gaps(root, "P1")
    _require(p0, "canonical checklist reports no open P0 gap; refusing to validate")

    checked = []
    for version in versions:
        notes_path = Path(f"docs/releases/v{version}.md")
        _require(
            (root / notes_path).exists(),
            f"v{version}: missing release notes {notes_path}",
        )
        _require(
            str(notes_path) in changelog,
            f"v{version}: CHANGELOG entry does not link {notes_path}",
        )
        text = (root / notes_path).read_text(encoding="utf-8")
        found = sections(text)
        for section in REQUIRED_SECTIONS:
            _require(
                section in found and found[section],
                f"v{version}: section '{section}' is missing or empty",
            )

        declared = set(re.findall(r"`([A-Z]\d+)`", found["Accepted limitations"]))
        missing = p0 - declared
        closed_but_declared = declared - p0 - p1
        _require(
            not missing,
            f"v{version}: accepted limitations omit open P0 gaps: {sorted(missing)}",
        )
        _require(
            not closed_but_declared,
            f"v{version}: accepted limitations list gaps that are not open: "
            f"{sorted(closed_but_declared)}",
        )

        verification = found["Verification"]
        for drill in REQUIRED_DRILLS:
            _require(
                drill in verification,
                f"v{version}: verification does not name the exercised drill {drill}",
            )
            _require(
                (root / drill).exists(),
                f"v{version}: verification references missing drill {drill}",
            )
        for reference in PATH_RE.findall(verification):
            _require(
                (root / reference).exists(),
                f"v{version}: verification references missing path {reference}",
            )

        journal = "journal" in text.lower()
        if journal:
            for section in ("Migration", "Rollback"):
                _require(
                    "journal" in found[section].lower(),
                    f"v{version}: notes mention the journal but section "
                    f"'{section}' does not state its position",
                )
        checked.append(version)
    return checked


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".")
    args = parser.parse_args(argv)
    root = Path(args.root).resolve()
    try:
        versions = validate(root)
    except Invalid as error:
        print(f"Invalid release notes: {error}", file=sys.stderr)
        return 1
    print(
        "Release notes valid: "
        f"{len(versions)} version(s) checked against the open P0 inventory"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

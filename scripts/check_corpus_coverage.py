#!/usr/bin/env python3
"""Account for differential-corpus coverage of the advertised command surface (V2).

Reads the generated command matrix (advertised command names) and the corpus in
`tests/reference_baseline.rs` (command literals used by the assertions), then
validates or refreshes `ci/differential/coverage.json`.

The accounting is deliberately conservative: a command counts as *covered* only
when at least one corpus assertion issues it. A missing command is reported, not
silently treated as covered, and the criterion stays open until the uncovered
list is empty and reviewed.

Standard library only.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

MATRIX = "docs/production-readiness/command-matrix.md"
CORPUS = "tests/reference_baseline.rs"
COVERAGE = "ci/differential/coverage.json"
SCHEMA = 1
CORPUS_CALL = re.compile(r'&\[b"([A-Za-z0-9_]+)"')
MATRIX_ROW = re.compile(r"^\| `([a-z0-9_\-]+)` \|")


def advertised(root: Path) -> list[str]:
    text = (root / MATRIX).read_text(encoding="utf-8")
    start = text.index("## Advertised commands")
    end = text.index("\n## ", start + 1)
    names = []
    for line in text[start:end].splitlines():
        match = MATRIX_ROW.match(line)
        if match:
            names.append(match.group(1))
    return sorted(set(names))


def corpus_commands(root: Path) -> dict[str, int]:
    text = (root / CORPUS).read_text(encoding="utf-8")
    counts: dict[str, int] = {}
    for name in CORPUS_CALL.findall(text):
        key = name.lower()
        counts[key] = counts.get(key, 0) + 1
    return counts


def build(root: Path) -> dict:
    names = advertised(root)
    counts = corpus_commands(root)
    covered = [name for name in names if counts.get(name)]
    uncovered = [name for name in names if not counts.get(name)]
    unknown = sorted(set(counts) - set(names))
    return {
        "schema": SCHEMA,
        "sources": {"matrix": MATRIX, "corpus": CORPUS},
        "advertised": len(names),
        "covered": len(covered),
        "uncovered": len(uncovered),
        "corpus_commands_outside_matrix": unknown,
        "covered_commands": {name: counts[name] for name in covered},
        "uncovered_commands": uncovered,
    }


def problems(root: Path, expected: dict) -> list[str]:
    path = root / COVERAGE
    if not path.exists():
        return [f"{COVERAGE} is missing; run --write"]
    try:
        committed = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        return [f"{COVERAGE} is not valid JSON: {error}"]
    if committed.get("schema") != SCHEMA:
        return [f"{COVERAGE} schema must be {SCHEMA}"]
    for key in ("sources", "advertised", "covered", "uncovered",
                "corpus_commands_outside_matrix", "covered_commands",
                "uncovered_commands"):
        if committed.get(key) != expected[key]:
            return [f"{COVERAGE} drifted from the corpus/matrix at key {key!r}; run --write"]
    return []


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--write", action="store_true", help="refresh coverage.json")
    args = parser.parse_args(argv)
    root = Path(args.root).resolve()
    expected = build(root)
    if args.write:
        (root / COVERAGE).write_text(json.dumps(expected, indent=2) + "\n", encoding="utf-8")
        print(
            f"corpus coverage written: {expected['covered']}/{expected['advertised']} advertised commands covered"
        )
        return 0
    issues = problems(root, expected)
    for issue in issues:
        print(f"FAIL: {issue}")
    if issues:
        return 1
    print(
        f"corpus coverage valid: {expected['covered']}/{expected['advertised']} advertised commands covered, "
        f"{expected['uncovered']} uncovered"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

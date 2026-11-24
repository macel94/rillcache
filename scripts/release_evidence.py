#!/usr/bin/env python3
"""Check the structure of a candidate evidence record without approving a release."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from urllib.parse import urlsplit

STAGES = ("merge", "compatibility", "security", "load-soak", "recovery", "rollout-rollback", "distribution-signoff")
REVIEW_STAGES = {"security", "distribution-signoff"}
IDENTITY = {"candidate": r"[0-9a-f]{40}", "artifact": r"sha256:[0-9a-f]{64}", "profile": r"[0-9a-f]{40}"}
MAX_BYTES = 256 * 1024


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON member")
        result[key] = value
    return result


def load_record(path: Path) -> dict:
    with path.open("rb") as source:
        data = source.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise ValueError("record exceeds byte limit")
    return json.loads(data.decode("utf-8"), object_pairs_hook=unique_object)


def is_text(value) -> bool:
    return isinstance(value, str) and bool(value.strip())


def is_https(value) -> bool:
    if not is_text(value) or any(char.isspace() or ord(char) < 32 for char in value):
        return False
    try:
        link = urlsplit(value)
        return link.scheme == "https" and bool(link.hostname) and link.username is None
    except ValueError:
        return False


def assess(record: dict, expected: dict[str, str]) -> dict:
    if not isinstance(record, dict) or set(record) != {"schema", "results"} or type(record["schema"]) is not int or record["schema"] != 1:
        raise ValueError("expected only schema 1 and results")
    results = record["results"]
    if not isinstance(results, dict) or results.keys() - set(STAGES):
        raise ValueError("expected an object keyed by known evidence stages")
    checks = {}
    for stage in STAGES:
        if stage not in results:
            checks[stage] = ["missing stage"]
            continue
        row = results[stage]
        names = set(IDENTITY) | {"result", "method", "environment", "observed", "evidence_url", "reviewer"}
        if not isinstance(row, dict) or row.keys() - names or row.get("result") not in ("pass", "fail", "not-run"):
            raise ValueError(f"invalid row: {stage}")
        problems = [f"{field} missing or does not match" for field, value in expected.items() if row.get(field) != value]
        if row["result"] != "pass":
            problems.append(f"result is {row['result']}")
        if row["result"] == "pass":
            for field in ("method", "environment", "observed"):
                if not is_text(row.get(field)):
                    problems.append(f"missing {field}")
            if not is_https(row.get("evidence_url")):
                problems.append("missing or invalid HTTPS evidence URL")
            if stage in REVIEW_STAGES and not is_text(row.get("reviewer")):
                problems.append("missing named reviewer")
        checks[stage] = problems
    return {
        "record_complete": not any(checks.values()),
        "release_evidence_verified": False,
        "promotion_authorized": False,
        "stage_problems": checks,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("record", type=Path)
    for field in IDENTITY:
        parser.add_argument(f"--{field}", required=True)
    args = parser.parse_args(argv)
    expected = {field: getattr(args, field) for field in IDENTITY}
    for field, pattern in IDENTITY.items():
        if not re.fullmatch(pattern, expected[field]):
            parser.error(f"invalid expected {field} identity")
    try:
        report = assess(load_record(args.record), expected)
    except (OSError, ValueError, RecursionError):
        # Never echo record contents or a private input path in the report.
        print(json.dumps({"record_complete": False, "release_evidence_verified": False,
                          "promotion_authorized": False, "error": "unreadable or malformed evidence record"}))
        return 2
    print(json.dumps(report, indent=2))
    return 0 if report["record_complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

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
# Corpus calls come in two shapes: a single-line `&[b"CMD", ...]` (used by the
# send/command/pipeline helpers) and a multi-line `assert_command(..., &[\n
# b"CMD", ...])`. Count the first literal in either shape; expected-reply values
# are built with `Resp::` constructors, so `&[b"` there is not a command.
DISPOSITION_KINDS = ("excluded", "nondeterministic", "extension", "pending")

# Reviewed disposition for advertised names that have no corpus assertion yet.
# `excluded` means the declared profile does not implement the surface (see
# docs/unsupported.md); `nondeterministic` means two servers cannot be expected to
# return byte-identical replies (random, clock, allocator/encoding, history or
# connection-negotiation dependent); `pending` means it is coverable and simply
# not written yet. V2 can only be called complete with zero `pending`.
DISPOSITIONS: dict[str, dict[str, str]] = {
    "asking": {"kind": "excluded", "reason": "cluster-only command; cluster routing is excluded (docs/unsupported.md)"},
    "readonly": {"kind": "excluded", "reason": "cluster-only read path; cluster routing is excluded"},
    "readwrite": {"kind": "excluded", "reason": "cluster-only read path; cluster routing is excluded"},
    "failover": {"kind": "excluded", "reason": "failover/promotion is excluded (docs/unsupported.md)"},
    "psync": {"kind": "excluded", "reason": "replication stream is excluded"},
    "sync": {"kind": "excluded", "reason": "replication stream is excluded"},
    "replconf": {"kind": "excluded", "reason": "replication control is excluded"},
    "replicaof": {"kind": "excluded", "reason": "replication/promotion is excluded"},
    "slaveof": {"kind": "excluded", "reason": "replication/promotion is excluded"},
    "function": {"kind": "excluded", "reason": "scripting/functions are excluded (pre-exposure gate)"},
    "script": {"kind": "excluded", "reason": "scripting/functions are excluded (pre-exposure gate)"},
    "save": {"kind": "excluded", "reason": "RDB persistence is unsupported; the journal is the persistence format"},
    "bgsave": {"kind": "excluded", "reason": "RDB persistence is unsupported; the journal is the persistence format"},
    "monitor": {"kind": "excluded", "reason": "MONITOR is excluded from the profile"},
    "move": {"kind": "excluded", "reason": "only database 0 exists; multi-DB is excluded"},
    "swapdb": {"kind": "excluded", "reason": "only database 0 exists; multi-DB is excluded"},
    "select": {"kind": "excluded", "reason": "only SELECT 0 is accepted; multi-DB is excluded"},
    "shutdown": {"kind": "excluded", "reason": "the corpus must not terminate its server; shutdown is covered by the signal/lifecycle tests"},
    "auth": {"kind": "nondeterministic", "reason": "depends on configured credentials; auth paths are pinned by tests/auth.rs"},
    "client": {"kind": "nondeterministic", "reason": "CLIENT LIST/INFO carry dynamic ids and ages"},
    "command": {"kind": "nondeterministic", "reason": "full COMMAND metadata is large and version-dependent"},
    "geodist": {"kind": "nondeterministic", "reason": "distance float formatting is implementation-specific"},
    "geopos": {"kind": "nondeterministic", "reason": "coordinate float formatting is implementation-specific"},
    "georadius": {"kind": "pending", "reason": "deterministic with explicit coordinates; not yet asserted"},
    "georadius_ro": {"kind": "pending", "reason": "deterministic read-only variant; not yet asserted"},
    "georadiusbymember": {"kind": "pending", "reason": "deterministic legacy variant; not yet asserted"},
    "georadiusbymember_ro": {"kind": "pending", "reason": "deterministic read-only variant; not yet asserted"},
    "hrandfield": {"kind": "nondeterministic", "reason": "random field selection"},
    "randomkey": {"kind": "nondeterministic", "reason": "random key selection"},
    "spop": {"kind": "nondeterministic", "reason": "random member removal"},
    "srandmember": {"kind": "nondeterministic", "reason": "random member selection"},
    "zrandmember": {"kind": "nondeterministic", "reason": "random member selection"},
    "keys": {"kind": "nondeterministic", "reason": "iteration order is unspecified"},
    "scan": {"kind": "nondeterministic", "reason": "cursor iteration order is unspecified"},
    "sscan": {"kind": "nondeterministic", "reason": "cursor iteration order is unspecified"},
    "zscan": {"kind": "nondeterministic", "reason": "cursor iteration order is unspecified"},
    "lastsave": {"kind": "nondeterministic", "reason": "wall-clock timestamp"},
    "time": {"kind": "nondeterministic", "reason": "wall-clock value"},
    "latency": {"kind": "nondeterministic", "reason": "history-dependent measurements"},
    "slowlog": {"kind": "nondeterministic", "reason": "history-dependent entries"},
    "memory": {"kind": "nondeterministic", "reason": "allocator/implementation-specific usage"},
    "object": {"kind": "nondeterministic", "reason": "encoding introspection is implementation-specific"},
    "pfdebug": {"kind": "nondeterministic", "reason": "private HLL diagnostics are implementation-specific"},
    "pfselftest": {"kind": "nondeterministic", "reason": "private HLL self-test output"},
    "hello": {"kind": "nondeterministic", "reason": "protocol negotiation changes the connection; covered by the RESP3/client-matrix suites"},
    "xinfo": {"kind": "nondeterministic", "reason": "consumer idle/seen times are wall-clock dependent"},
    "xpending": {"kind": "nondeterministic", "reason": "idle times are wall-clock dependent"},
    "delex": {"kind": "extension", "reason": "published extension absent from both pinned references, so differential equality is impossible; behavior is pinned by tests/conditional_write.rs"},
    "delifeq": {"kind": "extension", "reason": "published extension absent from both pinned references; behavior is pinned by tests/conditional_write.rs"},
    "digest": {"kind": "extension", "reason": "published extension absent from both pinned references; behavior is pinned by the collection tests"},
    "bgrewriteaof": {"kind": "pending", "reason": "returns a deterministic reply; not yet asserted in the corpus"},
    "bitfield": {"kind": "pending", "reason": "deterministic bitfield operations; not yet asserted"},
    "bitfield_ro": {"kind": "pending", "reason": "deterministic read-only bitfield; not yet asserted"},
    "bitop": {"kind": "pending", "reason": "deterministic bit operations; not yet asserted"},
    "bzmpop": {"kind": "pending", "reason": "immediately satisfiable form is deterministic; not yet asserted"},
    "dbsize": {"kind": "pending", "reason": "equal across an identical corpus run; not yet asserted"},
    "hexpireat": {"kind": "pending", "reason": "absolute deadline with a fixed epoch is deterministic; not yet asserted"},
    "hexpiretime": {"kind": "pending", "reason": "readback of a fixed deadline; not yet asserted"},
    "hpexpire": {"kind": "pending", "reason": "relative field deadline; not yet asserted"},
    "hpexpireat": {"kind": "pending", "reason": "absolute field deadline with a fixed epoch; not yet asserted"},
    "hpexpiretime": {"kind": "pending", "reason": "readback of a fixed deadline; not yet asserted"},
    "hgetdel": {"kind": "pending", "reason": "deterministic get-and-delete; not yet asserted"},
    "hgetex": {"kind": "pending", "reason": "deterministic get-and-expire; not yet asserted"},
    "hsetex": {"kind": "pending", "reason": "deterministic set-with-expiry; not yet asserted"},
    "msetex": {"kind": "pending", "reason": "deterministic multi-set with expiry; not yet asserted"},
    "publish": {"kind": "pending", "reason": "returns 0 with no subscribers; not yet asserted"},
    "spublish": {"kind": "pending", "reason": "returns 0 with no shard subscribers; not yet asserted"},
    "sort": {"kind": "pending", "reason": "deterministic without BY/GET patterns; not yet asserted"},
    "sort_ro": {"kind": "pending", "reason": "deterministic read-only sort; not yet asserted"},
    "subscribe": {"kind": "pending", "reason": "deterministic on a dedicated connection; the corpus uses one client"},
    "ssubscribe": {"kind": "pending", "reason": "deterministic on a dedicated connection; the corpus uses one client"},
    "unsubscribe": {"kind": "pending", "reason": "deterministic on a dedicated connection; the corpus uses one client"},
    "sunsubscribe": {"kind": "pending", "reason": "deterministic on a dedicated connection; the corpus uses one client"},
    "waitaof": {"kind": "pending", "reason": "returns deterministic local/replica counters without AOF replicas; not yet asserted"},
    "xautoclaim": {"kind": "pending", "reason": "deterministic with fixed ids; not yet asserted"},
    "xclaim": {"kind": "pending", "reason": "deterministic with fixed ids; not yet asserted"},
    "xreadgroup": {"kind": "pending", "reason": "already asserted in the streams corpus under its literal name; re-check extraction"},
    "xsetid": {"kind": "pending", "reason": "deterministic explicit id assignment; not yet asserted"},
}


SINGLE_LINE_CALL = re.compile(r'&\[b"([A-Za-z0-9_]+)"')
MULTILINE_CALL = re.compile(
    r'(?:assert_command|client\.command|send|fragmented)\s*\(\s*[^;]*?&\[\s*b"([A-Za-z0-9_]+)"',
    re.S,
)
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
    for name in SINGLE_LINE_CALL.findall(text) + MULTILINE_CALL.findall(text):
        key = name.lower()
        counts[key] = counts.get(key, 0) + 1
    return counts


def build(root: Path) -> dict:
    names = advertised(root)
    counts = corpus_commands(root)
    covered = [name for name in names if counts.get(name)]
    uncovered = [name for name in names if not counts.get(name)]
    unknown = sorted(set(counts) - set(names))
    disposition = {
        name: DISPOSITIONS.get(name, {"kind": "pending", "reason": "not covered yet"})
        for name in uncovered
    }
    return {
        "schema": SCHEMA,
        "sources": {"matrix": MATRIX, "corpus": CORPUS},
        "advertised": len(names),
        "covered": len(covered),
        "uncovered": len(uncovered),
        "corpus_commands_outside_matrix": unknown,
        "covered_commands": {name: counts[name] for name in covered},
        "uncovered_commands": uncovered,
        "dispositions": disposition,
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
    if committed.get("dispositions") != expected["dispositions"]:
        # Give a precise reason instead of only a diff of the whole map.
        known = committed.get("dispositions") or {}
        for name in expected["uncovered_commands"]:
            entry = known.get(name)
            if not isinstance(entry, dict) or entry.get("kind") not in DISPOSITION_KINDS:
                return [f"{COVERAGE}: {name} needs a disposition kind in {DISPOSITION_KINDS}"]
            if not entry.get("reason"):
                return [f"{COVERAGE}: {name} needs a disposition reason"]
        for name in expected["covered_commands"]:
            if name in known:
                return [f"{COVERAGE}: {name} is covered and must not carry a disposition"]
        return [f"{COVERAGE} dispositions drifted; run --write"]
    for key in ("sources", "advertised", "covered", "uncovered",
                "corpus_commands_outside_matrix", "covered_commands",
                "uncovered_commands", "dispositions"):
        if committed.get(key) != expected[key]:
            return [f"{COVERAGE} drifted from the corpus/matrix at key {key!r}; run --write"]
    return []


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--write", action="store_true", help="refresh coverage.json")
    parser.add_argument(
        "--require-complete",
        action="store_true",
        help="fail when any uncovered command is still recorded as pending",
    )
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
    pending = [
        name
        for name, entry in expected["dispositions"].items()
        if entry["kind"] == "pending"
    ]
    excluded = sum(1 for e in expected["dispositions"].values() if e["kind"] == "excluded")
    extension = sum(1 for e in expected["dispositions"].values() if e["kind"] == "extension")
    nondeterministic = sum(
        1 for e in expected["dispositions"].values() if e["kind"] == "nondeterministic"
    )
    print(
        f"corpus coverage valid: {expected['covered']}/{expected['advertised']} advertised commands covered, "
        f"{excluded} excluded by profile, {extension} extensions, {nondeterministic} nondeterministic, {len(pending)} pending"
    )
    if args.require_complete and pending:
        print(f"FAIL: pending dispositions remain: {', '.join(pending)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

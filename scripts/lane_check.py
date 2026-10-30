#!/usr/bin/env python3
"""Advisory lane check for parallel work in rillcache.

Classifies the files changed on the current branch against the lane map in
``docs/parallel-work-lanes.md``. This is a local preflight aid, not a required
merge check.

Exit codes:
  0  advisory result (default), or strict result with no violation
  1  strict mode found a shared-surface or unowned edit
  2  git or argument failure
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import subprocess
import sys
from dataclasses import dataclass

SHARED_SURFACES: tuple[str, ...] = (
    "src/engine.rs",
    "src/engine/shard.rs",
    "src/session.rs",
    "src/engine/discovery.rs",
    "src/engine/discovery/mod.rs",
    "docs/requirements.md",
    "docs/requirements/*",
    "docs/production-readiness.md",
    "docs/sharding.md",
    "README.md",
    "tests/discovery.rs",
    "tests/core_dispatch.rs",
)

# Order matters only for readability. A path may match at most one of these
# while the registry is still monolithic; discovery family files are listed
# explicitly for the post-split layout.
LANE_PATTERNS: dict[str, tuple[str, ...]] = {
    "streams": (
        "src/engine/streams.rs",
        "src/engine/discovery/streams.rs",
        "docs/streams.md",
        "tests/streams*.rs",
        "tests/blocking_stream*.rs",
    ),
    "collections": (
        "src/engine/collections.rs",
        "src/engine/collection_commands.rs",
        "src/engine/collection_config.rs",
        "src/engine/sort.rs",
        "src/engine/geo.rs",
        "src/engine/bitfield.rs",
        "src/engine/discovery/collections.rs",
        "src/engine/discovery/hashes.rs",
        "src/engine/discovery/lists.rs",
        "src/engine/discovery/sets.rs",
        "src/engine/discovery/zsets.rs",
        "docs/collections.md",
        "docs/geospatial.md",
        "docs/sort.md",
        "tests/collections*.rs",
        "tests/geo*.rs",
        "tests/sort*.rs",
        "tests/bitfield*.rs",
    ),
    "strings-keyspace": (
        "src/engine/data.rs",
        "src/engine/keyspace.rs",
        "src/engine/multikey.rs",
        "src/engine/string_algorithms.rs",
        "src/engine/string_ranges.rs",
        "src/engine/scan.rs",
        "src/engine/eviction.rs",
        "src/engine/commands.rs",
        "src/engine/discovery/strings.rs",
        "docs/strings.md",
        "docs/string-ranges.md",
        "docs/string-algorithms.md",
        "docs/multikey-strings.md",
        "docs/keyspace.md",
        "docs/eviction.md",
        "tests/string*.rs",
        "tests/multikey*.rs",
        "tests/keyspace*.rs",
        "tests/scan*.rs",
    ),
    "security-clients": (
        "src/engine/auth.rs",
        "src/engine/acl_log.rs",
        "src/engine/clients.rs",
        "src/engine/discovery/acl.rs",
        "docs/security.md",
        "docs/client-sessions.md",
        "tests/acl*.rs",
        "tests/auth*.rs",
        "tests/clients*.rs",
    ),
    "persistence": (
        "src/engine/journal.rs",
        "docs/persistence.md",
        "tests/journal*.rs",
        "tests/persistence*.rs",
    ),
    "observability-admin": (
        "src/engine/slowlog.rs",
        "src/engine/latency.rs",
        "src/engine/monitor.rs",
        "src/engine/administration.rs",
        "src/engine/introspection.rs",
        "src/engine/unsupported.rs",
        "src/engine/discovery/admin.rs",
        "src/engine/discovery/unsupported.rs",
        "docs/observability.md",
        "docs/introspection.md",
        "docs/unsupported.md",
        "tests/slowlog*.rs",
        "tests/latency*.rs",
        "tests/monitor*.rs",
        "tests/observability*.rs",
        "tests/commandstats*.rs",
    ),
    "pubsub": (
        "src/engine/pubsub.rs",
        "src/engine/discovery/pubsub.rs",
        "docs/pubsub.md",
        "tests/pubsub*.rs",
        "tests/notifications*.rs",
    ),
    "io-server": (
        "src/server.rs",
        "src/server/pool.rs",
        "src/protocol.rs",
        "src/bin/*.rs",
        "src/bin/**",
        "docs/client-io-handoffs.md",
        "docs/io-scheduling.md",
        "docs/client-sessions.md",
        "tests/server*.rs",
        "tests/protocol*.rs",
    ),
    "tooling": (
        "scripts/**",
        ".github/**",
        "Makefile",
        "Cargo.toml",
        "Cargo.lock",
        "benchmarks/**",
        "compose.yaml",
        "deploy/**",
        "docs/pr-quality-gate.md",
        "docs/benchmarks.md",
        "docs/benchmark-reports.md",
        "docs/ci-benchmark-review.md",
        "docs/parallel-pr-delivery.md",
        "docs/parallel-work-lanes.md",
    ),
}


@dataclass(frozen=True)
class Report:
    base: str
    files: tuple[str, ...]
    lanes: dict[str, tuple[str, ...]]
    shared: tuple[str, ...]
    unowned: tuple[str, ...]


def is_shared_surface(path: str) -> bool:
    """Return whether ``path`` is a cross-cutting surface file."""
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in SHARED_SURFACES)


def classify(path: str) -> str:
    """Return the lane for ``path``, or ``surface``/``unowned``."""
    if is_shared_surface(path):
        return "surface"
    for lane, patterns in LANE_PATTERNS.items():
        if any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns):
            return lane
    return "unowned"


def summarize(base: str, paths: list[str]) -> Report:
    """Group changed paths by lane and identify shared-surface edits."""
    lanes: dict[str, list[str]] = {}
    shared: list[str] = []
    unowned: list[str] = []
    for path in paths:
        lane = classify(path)
        if lane == "surface":
            shared.append(path)
        elif lane == "unowned":
            unowned.append(path)
        lanes.setdefault(lane, []).append(path)
    ordered = {
        lane: tuple(sorted(set(files)))
        for lane, files in sorted(lanes.items(), key=lambda item: item[0])
    }
    return Report(
        base=base,
        files=tuple(sorted(set(paths))),
        lanes=ordered,
        shared=tuple(sorted(set(shared))),
        unowned=tuple(sorted(set(unowned))),
    )


def merge_base(base: str) -> str:
    """Return the merge base of ``base`` and HEAD."""
    result = subprocess.run(
        ["git", "merge-base", base, "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def changed_files(base: str) -> list[str]:
    """Return non-deleted files changed between ``base`` and HEAD."""
    point = merge_base(base)
    # Compare the merge base with the working tree, not just HEAD, so the
    # check is useful before the first commit.
    result = subprocess.run(
        ["git", "diff", "--name-only", "--diff-filter=ACMRTUXB", point],
        check=True,
        capture_output=True,
        text=True,
    )
    return [line for line in result.stdout.splitlines() if line]


def strict_failure(report: Report, allow_shared: bool) -> bool:
    """Return whether strict mode should fail for this report."""
    return bool(report.unowned) or (bool(report.shared) and not allow_shared)


def render_text(report: Report) -> str:
    lines = [f"Lane check against {report.base} ({len(report.files)} changed files)"]
    if not report.files:
        lines.append("  no changed files")
        return "\n".join(lines)
    for lane, files in report.lanes.items():
        lines.append(f"  {lane}: {len(files)}")
        for path in files:
            lines.append(f"    {path}")
    if report.shared:
        lines.append("")
        lines.append("Shared surfaces touched:")
        for path in report.shared:
            lines.append(f"  {path}  [surface lane]")
        lines.append(
            "  Declare these in the PR body; prefer a separate surface-lane increment."
        )
    if report.unowned:
        lines.append("")
        lines.append("Unowned paths (add them to the lane map or a lane-owned test/doc):")
        for path in report.unowned:
            lines.append(f"  {path}")
    return "\n".join(lines)


def render_json(report: Report) -> str:
    return json.dumps(
        {
            "base": report.base,
            "files": report.files,
            "lanes": report.lanes,
            "shared": report.shared,
            "unowned": report.unowned,
        },
        indent=2,
        sort_keys=True,
    )


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base",
        default="origin/main",
        help="base revision to compare against (default: origin/main)",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="exit 1 on shared-surface or unowned edits",
    )
    parser.add_argument(
        "--allow-shared",
        action="store_true",
        help="do not fail strict mode for shared-surface edits (surface lane)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit machine-readable JSON instead of text",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        report = summarize(args.base, changed_files(args.base))
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        print(f"lane_check: git failed: {detail or error}", file=sys.stderr)
        return 2
    print(render_json(report) if args.json else render_text(report))
    if args.strict and strict_failure(report, args.allow_shared):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

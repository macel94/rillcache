#!/usr/bin/env python3
"""Check the published version matrix against the pinned files (readiness P7).

The page claims which upstream releases, client versions and load shape are
actually exercised. This checker keeps that claim honest:

* every pinned upstream version (`ci/upstream/pins.json`, `ci/daily-pins.json`)
  and client version (`ci/upstream/clients-v1.json`) must appear in
  `docs/compatibility-matrix.md`, with its pin file named;
* the page may not name an upstream or client version that is not pinned;
* every referenced workflow path must exist and the page must still name the
  exercising workflow for each row;
* the retained-limit statements (single hardware generation, no saturation model,
  competitors are not gates, support window lives elsewhere) must remain.

Usage:
    python3 scripts/check_compat_matrix.py [--root <repo>]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

DOC = Path("docs/compatibility-matrix.md")
PINS = Path("ci/upstream/pins.json")
CLIENTS = Path("ci/upstream/clients-v1.json")
DAILY = Path("ci/daily-pins.json")
WORKFLOW_RE = re.compile(r"`(\.github/workflows/[A-Za-z0-9_.-]+\.yml)`")
VERSION_RE = re.compile(r"^\| ([^|]+) \| (v?[0-9][0-9A-Za-z.\-]*) \|", re.M)
REQUIRED_LIMITS = (
    "single hardware generation",
    "saturation model",
    "never a gate",
    "support window",
)


class Invalid(Exception):
    pass


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise Invalid(message)


def pinned_versions(root: Path) -> dict[str, str]:
    versions: dict[str, str] = {}
    upstream = json.loads((root / PINS).read_text(encoding="utf-8"))
    for name, spec in upstream.get("upstreams", {}).items():
        _require("version" in spec, f"{PINS}: upstream {name} has no version")
        versions[name] = spec["version"]
    daily = json.loads((root / DAILY).read_text(encoding="utf-8"))
    for name, spec in daily.items():
        if isinstance(spec, dict) and "version" in spec:
            versions[name] = spec["version"]
    clients = json.loads((root / CLIENTS).read_text(encoding="utf-8"))
    for client in clients.get("clients", []):
        versions[client["id"]] = client["version"]
    return {name.lower(): version for name, version in versions.items()}


def validate(root: Path) -> dict[str, str]:
    text = (root / DOC).read_text(encoding="utf-8")
    versions = pinned_versions(root)
    rows = {}
    for name, version in VERSION_RE.findall(text):
        rows[name.strip().lower()] = version.strip()

    for name, version in versions.items():
        _require(name in rows, f"{DOC}: pinned entry {name!r} is not in the matrix")
        _require(
            rows[name] == version,
            f"{DOC}: {name} is listed as {rows[name]!r} but pinned as {version!r}",
        )
    extra = set(rows) - set(versions)
    _require(not extra, f"{DOC}: rows without a pin: {sorted(extra)}")

    for name in ("redis", "valkey"):
        _require(
            f"`{PINS.as_posix()}`" in text,
            f"{DOC}: the upstream rows must name {PINS.as_posix()} as the pin",
        )
    _require(
        f"`{DAILY.as_posix()}`" in text,
        f"{DOC}: the comparison rows must name {DAILY.as_posix()} as the pin",
    )

    workflows = WORKFLOW_RE.findall(text)
    _require(workflows, f"{DOC}: no exercising workflow is named")
    for workflow in set(workflows):
        _require((root / workflow).exists(), f"{DOC}: missing workflow {workflow}")

    lowered = text.lower()
    for limit in REQUIRED_LIMITS:
        _require(
            limit in lowered,
            f"{DOC}: the retained limits must still state {limit!r}",
        )
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".")
    args = parser.parse_args(argv)
    try:
        rows = validate(Path(args.root).resolve())
    except Invalid as error:
        print(f"Invalid compatibility matrix: {error}", file=sys.stderr)
        return 1
    print(
        f"Compatibility matrix valid: {len(rows)} pinned rows, "
        f"{len(set(WORKFLOW_RE.findall((Path(args.root).resolve() / DOC).read_text(encoding='utf-8'))))} workflows"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

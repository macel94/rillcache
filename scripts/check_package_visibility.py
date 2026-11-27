#!/usr/bin/env python3
"""Check the container package-visibility record (readiness L7).

GHCR packages start private, so a release can pass every automated gate while
remaining unpullable. This checker makes the manual visibility action an explicit,
recorded step:

* `docs/acceptance/package-visibility.md` must carry one row per released version
  from `CHANGELOG.md`, with the tested platform digests, who verified visibility,
  when, and the method used;
* `--check` (default) validates the record's structure and cross-links, which is
  what CI enforces;
* `--require-public` additionally refuses any row that is not `public-verified`,
  and is what `scripts/cut-release.sh --publish` runs, so a release cannot be
  tagged while the package is private or unverified;
* `--verify-remote` asks the GitHub API for the live package visibility when a
  token is available and refuses when it is not public (network mode, release
  owner only).

Usage:
    python3 scripts/check_package_visibility.py --check
    python3 scripts/check_package_visibility.py --require-public --version 0.1.0
    python3 scripts/check_package_visibility.py --verify-remote --version 0.1.0
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

RECORD = Path("docs/acceptance/package-visibility.md")
CHANGELOG = Path("CHANGELOG.md")
PACKAGE = "rillcache"
COLUMNS = (
    "Version",
    "amd64 digest",
    "arm64 digest",
    "Visibility",
    "Verified by",
    "Verified at",
    "Method",
)
STATES = ("public-verified", "unverified")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")


class Invalid(Exception):
    pass


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise Invalid(message)


def released_versions(root: Path) -> list[str]:
    text = (root / CHANGELOG).read_text(encoding="utf-8")
    return re.findall(r"^## (\d+\.\d+\.\d+)\b", text, re.M)


def rows(root: Path) -> dict[str, dict[str, str]]:
    text = (root / RECORD).read_text(encoding="utf-8")
    table: dict[str, dict[str, str]] = {}
    header: list[str] | None = None
    for line in text.splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if header is None:
            if cells == list(COLUMNS):
                header = cells
            continue
        if set("".join(cells)) <= set("-: "):
            continue
        _require(
            len(cells) == len(COLUMNS),
            f"{RECORD}: row has {len(cells)} cells, expected {len(COLUMNS)}",
        )
        entry = dict(zip(COLUMNS, cells))
        version = entry["Version"].strip("`v ")
        _require(
            version not in table, f"{RECORD}: duplicate row for version {version}"
        )
        table[version] = entry
    _require(header is not None, f"{RECORD}: missing the record table header")
    _require(table, f"{RECORD}: the record table has no rows")
    return table


def validate(root: Path, require_public: bool, version: str | None) -> list[str]:
    table = rows(root)
    released = released_versions(root)
    for released_version in released:
        _require(
            released_version in table,
            f"version {released_version} has no package-visibility row",
        )
    for recorded in table:
        _require(
            recorded in released,
            f"package-visibility row {recorded} is not a released version",
        )
    checked = []
    for recorded, entry in sorted(table.items()):
        if version is not None and recorded != version:
            continue
        state = entry["Visibility"]
        _require(
            state in STATES, f"{recorded}: unknown visibility state {state!r}"
        )
        if state == "public-verified":
            for column in ("amd64 digest", "arm64 digest"):
                _require(
                    DIGEST_RE.match(entry[column]) is not None,
                    f"{recorded}: {column} must be a sha256 digest",
                )
            _require(
                entry["Verified by"] not in {"", "-", "unknown"},
                f"{recorded}: public-verified needs the verifying maintainer",
            )
            _require(
                DATE_RE.match(entry["Verified at"]) is not None,
                f"{recorded}: public-verified needs an ISO date",
            )
            _require(
                len(entry["Method"]) > 10,
                f"{recorded}: public-verified needs the verification method",
            )
        elif require_public:
            raise Invalid(
                f"{recorded}: package visibility is {state}; an anonymous pull must be "
                "recorded as public-verified before publishing"
            )
        checked.append(recorded)
    _require(checked, "no package-visibility row matched the requested version")
    return checked


def verify_remote(root: Path, version: str) -> None:
    record = rows(root).get(version)
    _require(record is not None, f"no package-visibility row for {version}")
    try:
        output = subprocess.run(
            [
                "gh",
                "api",
                f"/users/{{owner}}/packages/container/{PACKAGE}",
                "--jq",
                ".visibility",
            ],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise Invalid(
            "remote verification needs an authenticated gh CLI "
            f"(gh api failed: {error})"
        ) from error
    _require(
        output.lower() == "public",
        f"package {PACKAGE} reports visibility {output!r}, not 'public'",
    )
    if record["Visibility"] != "public-verified":
        raise Invalid(
            f"{version}: the live package is public but the record still says "
            f"{record['Visibility']!r}; record the digests and the verification"
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".")
    parser.add_argument("--check", action="store_true", help="validate the record")
    parser.add_argument(
        "--require-public",
        action="store_true",
        help="refuse unverified/private rows (release gate)",
    )
    parser.add_argument("--version", help="restrict to one version")
    parser.add_argument(
        "--verify-remote",
        action="store_true",
        help="query the live package visibility (needs gh auth)",
    )
    args = parser.parse_args(argv)
    root = Path(args.root).resolve()
    try:
        checked = validate(
            root,
            require_public=args.require_public or args.verify_remote,
            version=args.version,
        )
        if args.verify_remote:
            _require(args.version is not None, "--verify-remote needs --version")
            verify_remote(root, args.version)
    except Invalid as error:
        print(f"Invalid package-visibility record: {error}", file=sys.stderr)
        return 1
    mode = "public" if args.require_public or args.verify_remote else "recorded"
    print(
        f"Package visibility record valid ({mode}): "
        f"{len(checked)} version(s): {', '.join(checked)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

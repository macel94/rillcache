#!/usr/bin/env python3
"""Check that the split documentation indexes stay complete and consistent.

Standard library only. This is a local review aid: it is intentionally not wired
into `make ci` or any required workflow.
"""

import argparse
import os
import re
import sys

INDEX_FILES = {
    os.path.join("docs", "requirements.md"): os.path.join("docs", "requirements"),
    os.path.join("docs", "production-readiness.md"): os.path.join("docs", "production-readiness"),
}

REQUIREMENT_RE = re.compile(
    r"\*\*(R(?:-[A-Z]+)?\d+) "
    r"\((?:ubiquitous|event-driven|state-driven|unwanted behavior|optional feature)\):\*\*"
)
STORY_RE = re.compile(r"^\|\s*(S(?:-[A-Z]+|\d+))\s*\|")
MARKDOWN_LINK_RE = re.compile(r"\]\(([^)\s]+)\)")
CONFLICT_MARKERS = ("<<<<<<<", "=======", ">>>>>>>")


def _relative_links(text):
    """Yield relative link targets, dropping anchors and query strings."""
    for match in MARKDOWN_LINK_RE.finditer(text):
        target = match.group(1)
        if target.startswith(("http://", "https://", "mailto:", "#")):
            continue
        if target.startswith("/"):
            continue
        yield target.split("#", 1)[0].split("?", 1)[0]


def _markdown_files(directory):
    if not os.path.isdir(directory):
        return []
    return sorted(
        os.path.join(directory, name)
        for name in os.listdir(directory)
        if name.endswith(".md") and os.path.isfile(os.path.join(directory, name))
    )


def _area_dirs_conflict(root):
    """Return files containing unresolved merge markers."""
    errors = []
    for index, area in INDEX_FILES.items():
        for path in [os.path.join(root, index)] + _markdown_files(os.path.join(root, area)):
            if not os.path.isfile(path):
                continue
            with open(path, encoding="utf-8") as handle:
                for number, line in enumerate(handle, 1):
                    stripped = line.rstrip("\n")
                    if any(stripped.startswith(marker) for marker in CONFLICT_MARKERS):
                        errors.append(
                            "%s:%d: unresolved conflict marker %r"
                            % (os.path.relpath(path, root), number, stripped[:20])
                        )
    return errors


def check(root="."):
    """Return a list of human-readable errors; an empty list means success."""
    root = os.path.abspath(root)
    errors = []

    for index, area in INDEX_FILES.items():
        index_path = os.path.join(root, index)
        area_path = os.path.join(root, area)
        if not os.path.isfile(index_path):
            errors.append("missing index: %s" % index)
            continue
        files = _markdown_files(area_path)
        if not files:
            errors.append("empty split: %s has no Markdown area files" % area)
            continue

        with open(index_path, encoding="utf-8") as handle:
            index_text = handle.read()

        linked = set()
        for target in _relative_links(index_text):
            # Links in the index are relative to the docs/ directory.
            linked.add(os.path.normpath(os.path.join(root, "docs", target)))

        for path in files:
            if os.path.normpath(path) not in linked:
                errors.append(
                    "%s is not linked from %s" % (os.path.relpath(path, root), index)
                )

        for target in _relative_links(index_text):
            resolved = os.path.normpath(os.path.join(root, "docs", target))
            if resolved.startswith(os.path.normpath(area_path)) and not os.path.isfile(resolved):
                errors.append(
                    "%s links to missing area file %s"
                    % (index, os.path.relpath(resolved, root))
                )

    errors.extend(_area_dirs_conflict(root))

    requirement_ids = {}
    story_ids = {}
    for area in INDEX_FILES.values():
        for path in _markdown_files(os.path.join(root, area)):
            with open(path, encoding="utf-8") as handle:
                for number, line in enumerate(handle, 1):
                    for match in REQUIREMENT_RE.finditer(line):
                        requirement_ids.setdefault(match.group(1), []).append(
                            "%s:%d" % (os.path.relpath(path, root), number)
                        )
                    match = STORY_RE.match(line)
                    if match:
                        story_ids.setdefault(match.group(1), []).append(
                            "%s:%d" % (os.path.relpath(path, root), number)
                        )

    if not requirement_ids:
        errors.append("no requirement IDs found under the split area files")
    if not story_ids:
        errors.append("no story IDs found under the split area files")

    for identifier, locations in sorted(requirement_ids.items()):
        if len(locations) > 1:
            errors.append(
                "duplicate requirement ID %s in %s" % (identifier, ", ".join(locations))
            )
    for identifier, locations in sorted(story_ids.items()):
        if len(locations) > 1:
            errors.append("duplicate story ID %s in %s" % (identifier, ", ".join(locations)))

    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root (default: current directory)")
    args = parser.parse_args(argv)

    errors = check(args.root)
    if errors:
        for error in errors:
            print("FAIL: %s" % error, file=sys.stderr)
        print("%d documentation index problem(s) found" % len(errors), file=sys.stderr)
        return 1

    print(
        "OK: both documentation indexes link every area file, IDs are unique, "
        "the split is non-empty and no conflict markers were found"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

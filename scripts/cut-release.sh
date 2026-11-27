#!/usr/bin/env bash
# Cut a Rillcache release: validate the published lifecycle gates, then tag.
#
# The gates are the L2 contract in docs/release-lifecycle.md. This script refuses
# to publish a tag when any gate fails, so the policy is enforced mechanically
# instead of being a paragraph. Only the release owner may run it with --publish.
#
# Usage:
#   scripts/cut-release.sh --version 0.1.0            # validate only (safe)
#   scripts/cut-release.sh --version 0.1.0 --publish  # create tag + notes stub
set -euo pipefail

version=""
publish=0
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

usage() {
    sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) version=${2:-}; shift 2 ;;
        --publish) publish=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: --version MAJOR.MINOR.PATCH is required (got '${version}')" >&2
    exit 2
fi

fail=0
gate() {
    local name=$1; shift
    if "$@" >/dev/null 2>&1; then
        printf 'PASS  %s\n' "$name"
    else
        printf 'FAIL  %s\n' "$name"
        fail=$((fail + 1))
    fi
}

tag="v${version}"
cargo_version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -1)
notes="$repo_root/docs/releases/${tag}.md"

gate "package version matches the tag (${cargo_version} == ${version})" \
    test "$cargo_version" = "$version"
gate "release notes exist at docs/releases/${tag}.md" test -s "$notes"
gate "release notes name the support window" \
    grep -qi "support window" "$notes"
gate "release notes state the compatibility profile" \
    grep -qi "compatib" "$notes"
gate "release notes state migration/rollback position" \
    grep -Eqi "migration|rollback" "$notes"
gate "CHANGELOG.md mentions ${version}" grep -q "$version" "$repo_root/CHANGELOG.md"
gate "canonical checklist keeps the not-ready status while P0 items are open" \
    grep -q '\*\*Status: NOT PRODUCTION READY\.\*\*' "$repo_root/docs/production-readiness-gaps.md"
# Every still-open P0 checklist item must be named in the release notes as an
# accepted limitation; nothing may disappear between the checklist and the notes.
all_open_p0_listed() {
    local id
    while read -r id; do
        [[ -n $id ]] || continue
        grep -q "\`${id}\`" "$notes" || return 1
    done < <(awk '/^- \[ \] \*\*/ && /\[P0\]/ { match($0, /\*\*[A-Z][0-9]+\./); id=substr($0, RSTART+2, RLENGTH-3); print id }' \
        "$repo_root/docs/production-readiness-gaps.md")
    return 0
}
gate "every open P0 item is named as an accepted limitation" all_open_p0_listed

if (( fail > 0 )); then
    echo "refusing to release ${tag}: ${fail} gate(s) failed" >&2
    exit 1
fi

echo "all release gates passed for ${tag}"

if (( publish == 0 )); then
    echo "validation only; re-run with --publish to create the tag"
    exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
    echo "error: gh is required to publish" >&2
    exit 1
fi

head_branch=$(git -C "$repo_root" rev-parse --abbrev-ref HEAD)
if [[ $head_branch != main ]]; then
    echo "error: releases are cut from main (current: ${head_branch})" >&2
    exit 1
fi

if git -C "$repo_root" rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
    echo "error: tag ${tag} already exists" >&2
    exit 1
fi

git -C "$repo_root" tag -a "$tag" -m "Rillcache ${tag}"
git -C "$repo_root" push origin "refs/tags/${tag}"
gh release create "$tag" --repo macel94/rillcache \
    --title "Rillcache ${tag}" \
    --notes-file "$notes"
echo "published ${tag}"

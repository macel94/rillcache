#!/usr/bin/env bash
# Create a fresh worktree from the current origin/main for one lane/task.
set -euo pipefail

usage() {
    printf 'usage: %s <branch> [lane]\n' "${0##*/}" >&2
    printf 'env: WORKTREE_ROOT (default /root/worktrees)\n' >&2
    exit 2
}

[[ $# -ge 1 && $# -le 2 ]] || usage
branch="$1"
lane="${2:-${branch##*/}}"
root="${WORKTREE_ROOT:-/root/worktrees}"
name="${branch//\//-}"
destination="$root/rillcache-$name"

repo="$(git rev-parse --show-toplevel)"
git -C "$repo" fetch origin --prune
git -C "$repo" worktree prune

if git -C "$repo" show-ref --verify --quiet "refs/heads/$branch"; then
    printf 'branch already exists: %s\n' "$branch" >&2
    exit 1
fi
if [[ -e "$destination" ]]; then
    printf 'destination already exists: %s\n' "$destination" >&2
    exit 1
fi

mkdir -p "$root"
git -C "$repo" worktree add -b "$branch" "$destination" origin/main

printf 'worktree: %s\n' "$destination"
printf 'branch:   %s\n' "$branch"
printf 'lane:     %s\n' "$lane"
printf '\nnext:\n'
printf '  cd %s\n' "$destination"
printf '  python3 scripts/lane_check.py\n'
printf '  make ci\n'

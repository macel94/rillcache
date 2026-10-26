#!/usr/bin/env bash
# Trusted default-branch publication only. Never commit or push directly to main.
set -euo pipefail
: "${GITHUB_REPOSITORY:?}" "${GITHUB_RUN_ID:?}" "${GITHUB_RUN_ATTEMPT:?}" "${DEFAULT_BRANCH:?}"
[[ "$GITHUB_REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]
[[ "$GITHUB_RUN_ID" =~ ^[1-9][0-9]*$ && "$GITHUB_RUN_ATTEMPT" =~ ^[1-9][0-9]*$ ]]
[[ "$DEFAULT_BRANCH" == main ]]
branch="automation/benchmark-report-$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT"
owner="${GITHUB_REPOSITORY%%/*}"
created=0
helper="$(dirname -- "${BASH_SOURCE[0]}")/report-pr-delivery.py"
existing=$(gh api --method GET "repos/$GITHUB_REPOSITORY/pulls" \
  -f state=open -f "head=$owner:$branch" -f "base=$DEFAULT_BRANCH" --jq '.[0].number // empty')
if [[ -n "$existing" ]]; then
  number="$existing"
else
  git fetch origin "$DEFAULT_BRANCH"
  git switch --create "$branch" "origin/$DEFAULT_BRANCH"
  ./target/release/rillcache-review publish --out target/incoming-report --destination benchmarks
  git add -- benchmarks/README.md benchmarks/reports
  if git diff --cached --quiet; then
    printf 'Report is already published; no PR or main commit needed.\n'
    exit 0
  fi
  git -c user.name='github-actions[bot]' -c user.email='41898282+github-actions[bot]@users.noreply.github.com' \
    commit -m "docs: publish benchmark report $GITHUB_RUN_ID"
  git push origin "HEAD:refs/heads/$branch"
  body="$RUNNER_TEMP/benchmark-report-pr.txt"
  printf '%s\n' \
    "Generated report from https://github.com/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID." \
    '' 'Only generated benchmark dashboard/history paths are changed.' \
    'This PR has no bypass: tests and the full Valkey performance gate must pass on an up-to-date branch before squash merging.' \
    'The measured source in the report remains explicit; a report update is not new application performance evidence.' > "$body"
  gh pr create --repo "$GITHUB_REPOSITORY" --base "$DEFAULT_BRANCH" --head "$branch" \
    --title "docs: publish benchmark report $GITHUB_RUN_ID" --body-file "$body"
  number=$(gh api --method GET "repos/$GITHUB_REPOSITORY/pulls" \
    -f state=open -f "head=$owner:$branch" -f "base=$DEFAULT_BRANCH" --jq '.[0].number')
  created=1
fi
[[ "$number" =~ ^[1-9][0-9]*$ ]]
# GITHUB_TOKEN PR events can be approval-held. Inspect exact-head run history
# first: real failed evidence is a stop, and existing running/successful checks
# must not be dispatched again. Dispatch definitions must match protected main.
inspection=$(python3 "$helper" inspect --number "$number" --branch "$branch")
read -r head ci_needed gate_needed tcl_needed <<< "$inspection"
[[ "$head" =~ ^[0-9a-f]{40}$ && "$ci_needed$gate_needed$tcl_needed" =~ ^[01]{3}$ ]]
if [[ "$ci_needed" == 1 ]]; then
  gh workflow run ci.yml --repo "$GITHUB_REPOSITORY" --ref "$branch"
fi
if [[ "$gate_needed" == 1 ]]; then
  gh workflow run pr-quality.yml --repo "$GITHUB_REPOSITORY" --ref "$branch" -f "pull_request=$number"
fi
if [[ "$tcl_needed" == 1 ]]; then
  gh workflow run upstream-compatibility.yml --repo "$GITHUB_REPOSITORY" --ref "$branch"
fi
# Enable only our newly-created ready PR. A retry must not silently override a
# user's subsequently disabled auto-merge request on an existing PR.
verification=()
if [[ "$created" == 1 ]]; then
  gh pr merge "$number" --repo "$GITHUB_REPOSITORY" --auto --squash --match-head-commit "$head"
  verification=(--require-enabled)
fi
merge_state=$(python3 "$helper" verify-auto --number "$number" --branch "$branch" --head "$head" "${verification[@]}")
printf 'Benchmark report PR: https://github.com/%s/pull/%s\n%s\n' "$GITHUB_REPOSITORY" "$number" "$merge_state" | tee -a "$GITHUB_STEP_SUMMARY"

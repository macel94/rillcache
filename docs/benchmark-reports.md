# Automatic benchmark reports

Read **[current performance](../benchmarks/README.md)** directly in the repository.
The root README links to it. Full manual/weekly comparative runs propose report
PRs under the [PR-only main/test/Valkey policy](pr-quality-gate.md). After the
required checks/reviews and protected merge, they publish:

- `benchmarks/README.md`: the latest workflow run/review-attempt's performance page.
- `benchmarks/reports/<run-id>-<review-attempt>.md`: permanent per-run Markdown.
- Adjacent JSON manifests: source/run identity, benchmark status, validation and
  AI status. `latest.json` orders publication; `last-successful.json` retains the
  last complete successful numeric comparison.
- The same report in the **review job's Actions summary**.

No artifact download is needed to understand current performance. Raw JSON/logs
remain separate evidence artifacts. The short embedded smoke on ordinary CI is
not a performance comparison and intentionally does not update this page or call AI.

## Automatic Copilot authentication and model selection

Run **Comparative benchmarks** on the default branch. **No personal token or
repository secret needs to be created.** The review job grants
`copilot-requests: write` and supplies its built-in, short-lived `GITHUB_TOKEN`.
Its repository permission remains `contents: read`; inference cannot push source.

GitHub explicitly supports personally owned repositories: usage is billed to the
repository owner's Copilot seat. Organization-owned repositories use the
organization's seat/policy, including its policy allowing workflow-token usage.
Normal Copilot entitlement, model policy and quota still apply; an authenticated
GitHub CLI session alone does not establish inference access. This repository's
[manual authentication check](../.github/workflows/copilot-auth-check.yml) has
verified real inference with the workflow token and recorded **GPT-6 Luna** in
Copilot's usage output, without creating or storing a PAT.

See [Copilot in Actions](https://docs.github.com/en/copilot/how-tos/copilot-cli/automate-copilot-cli/automate-with-actions)
and [authentication and billing](https://docs.github.com/en/copilot/concepts/agent-capabilities/agent-powered-workflows-and-operations/copilot-cli-in-github-actions).
GitHub Models was [retired on July 30, 2026](https://docs.github.com/en/github-models);
this integration uses **GitHub Copilot**, not that API or OpenAI BYOK.

The reviewer first requests `gpt-6-luna`. If the pinned CLI explicitly rejects
that model **before inference**, with an absent or verified zero-usage file, it retries once with Copilot's
`auto` model selection. Authentication/network failures and charged inference
failures are not retried with another model. Accepted output must have complete
Copilot usage accounting with concrete model IDs and generated tokens. Reports
and JSON manifests disclose **the actual models**, preferred/fallback selection,
and workflow/local authentication method. An `auto` label is never passed off as
an identified model, and an alternative is never labelled a Luna assessment.

One review shares a **120-second deadline plus a 10-second termination grace**
and a **30-AI-credit session cap**, with **16 KiB of accepted output**. The only
permitted retry is the pre-inference model rejection above. Thirty credits is the
pinned CLI's minimum session cap, not a claim that each review consumes that
amount. Copilot CLI is pinned to `v1.0.103` with an archive SHA-256; auto-updates
are disabled.

Missing access, unavailable CLI/models, budget exhaustion, timeout or unusable
output produces an explicit **AI unavailable** notice. A successful, validated
default-branch comparison then **fails the review job's AI-completion gate**;
it cannot silently pass as a successful AI review. The public report still
uploads its independently validated numbers and proposes their publication PR. Failed/incomplete
comparisons never invoke Copilot. Review-only reruns reuse the original comparison
artifact ID and benchmark-attempt identity; competitors are not rebuilt.

An ignored `.env` placeholder and tracked `.env.example` describe an **optional
local** `COPILOT_GITHUB_TOKEN` (fine-grained personal token with Copilot Requests
permission). Actions does not need or read that secret. Nothing automatically
loads `.env`. Never copy a broad repository-administration token or paste
credentials into chat/Git. The server, benchmark client, offline report generation
and tests require **no API key**.

## What readers see

The report starts with benchmark status, data quality, model-review status, tested
source, run and attempt identities. A quick SET comparison shows median throughput,
relative rate and **whole-batch p99** for all four CPU/pipeline profiles. Expand
profile sections for all five workloads, three-run throughput spread, CPU,
RSS, scheduling and idle observations.

Numbers are calculated by `rillcache-review`, a safe, dependency-free Rust tool.
It rejects truncated/malformed/duplicate JSON, nonfinite numbers, mismatched source
or parameters, duplicate/missing runs, incorrect batch counts, inconsistent
throughput/percentiles/CPU counters, invalid coverage, overlapping single-core
CPU masks and inconsistent recorded reference/binary provenance. A profile is
shown only if all three servers' fifteen records validate. A cross-profile
binary/hardware/compiler/version mismatch rejects the cohort.

The tool checks the **recorded** executable digests; the benchmark job separately
checks actual reference files with `sha256sum`. Neither operation implies signed
commits, signature verification or equivalent Redis/Valkey functionality.

The AI gets only the deterministic summary and fixed interpretation rules, not
repository contents, raw process environment, credentials or tools. Its narrative
is separately labelled and quoted; HTML and link/image syntax are escaped.
Numbers are never parsed from AI output or replaced by it. Reviewers must still
verify AI conclusions. AI does not decide merge eligibility: the separate,
deterministic [PR gate](pr-quality-gate.md) enforces the owner's narrow throughput
threshold. Shared-runner uncertainty and throughput/tail tradeoffs remain visible;
neither an AI narrative nor a passing PR gate establishes a general victory.

## Workflow security and publication

1. **compare**: read-only repository token; builds/tests/benchmarks as before.
2. **review**: `contents: read` plus `copilot-requests: write` on its short-lived
   workflow token; downloads that job's exact artifact, validates it offline,
   then runs Copilot for eligible default-branch data. The model child has a fresh
   home/working directory, a minimal environment, no available model tools,
   explicit shell/write/URL denials, built-in MCP disabled and custom instructions
   disabled. Copilot private state, output and
   diagnostics are not included in uploaded public reports.
3. **publish**: separate, default-branch-only job with `contents: write`,
   `pull-requests: write` and `actions: write`. It never runs inference or receives
   the inference job's token or billing permission. It checks report/run identity,
   stages only dashboard/history paths on an automation branch, opens/reuses a PR
   and dispatches both mandatory read-only checks on its head. It never pushes main.

Publication is serialized and compares `(workflow run number, review attempt)`:
a late rerun of an older run is archived but cannot overwrite the current page.
A failed latest attempt links back to the last complete successful numeric report.
Archive collisions are rejected, not overwritten. The job creates a uniquely
named report branch from current main and proposes a protected PR, **never
force-pushing** or restoring an old source tree. Existing PRs are reused and
already-published reports no-op. Stale branches must be updated/regenerated and
rechecked, not exempted from the up-to-date requirement. GitHub-token PR events
may require workflow approval; explicit dispatches start checks without a new
credential and attach them to the PR head. Report PRs are not auto-merged or
exempted from the Valkey gate. Non-default branches can generate numeric Actions
reports but receive no inference token and cannot publish directly to main.

A cancelled workflow may not reach reporting; a runner-wide outage can also
prevent publication. These cases do not fabricate new results. If branch push, PR creation or required-check dispatch fails, the report remains
readable in the Actions summary and publication fails visibly. The dashboard
updates only after its report PR satisfies the same protected merge path as code.

## Local verification

`make ci` includes offline tests of the archived 180-record cohort, corrupt-data
rejection, token isolation, model usage/fallback/error/unsafe-output handling,
the AI-completion gate, legacy-manifest compatibility, immutable history,
out-of-order/rerun publication and last-successful links. No inference is used by
tests. `cargo run --locked --offline --bin rillcache-review -- --help` describes
standalone generation/review/publication commands. The CLI prepares files only;
Git commits/pushes belong to the explicitly authorized publication workflow.

The current released `actionlint` v1.7.12 predates `copilot-requests`; when
linting, ignore only `^unknown permission scope "copilot-requests"\.`. GitHub's
workflow parser and live authentication check validate that documented scope;
all other linter diagnostics still fail verification.

Copilot is an external CI review tool, like the optional native reference
servers are external benchmark tools. It is **not** an application dependency,
Rust dependency, native extension, FFI binding or part of the server release.

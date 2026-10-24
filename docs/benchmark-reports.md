# Automatic benchmark reports

Read **[current performance](../benchmarks/README.md)** directly in the repository.
The root README links to it. Full manual/weekly comparative runs publish:

- `benchmarks/README.md`: the latest workflow run/review-attempt's performance page.
- `benchmarks/reports/<run-id>-<review-attempt>.md`: permanent per-run Markdown.
- Adjacent JSON manifests: source/run identity, benchmark status, validation and
  AI status. `latest.json` orders publication; `last-successful.json` retains the
  last complete successful numeric comparison.
- The same report in the **review job's Actions summary**.

No artifact download is needed to understand current performance. Raw JSON/logs
remain separate evidence artifacts. The short embedded smoke on ordinary CI is
not a performance comparison and intentionally does not update this page or call AI.

## Enable GPT-6 Luna

GitHub Models was [retired on July 30, 2026](https://docs.github.com/en/github-models).
This integration uses **GitHub Copilot**, never the retired Models API, OpenAI
BYOK, or a substitute model.

For this personally owned repository:

1. Have a Copilot plan/policy that permits Copilot CLI and `gpt-6-luna`.
2. Create a **personal-account-owned fine-grained PAT** with the **Copilot Requests**
   account permission. Repository write permission is not needed for inference.
   Do not use a classic PAT or copy a broad repository-administration token.
3. Save it as the Actions repository secret **`COPILOT_GITHUB_TOKEN`**. Do not paste
   credentials into issues, Markdown, artifacts or chat.
4. Run **Comparative benchmarks** from Actions. If adding the secret after a run,
   rerun the review/publish jobs: their input uses the original comparison artifact
   ID and benchmark-attempt identity, not a guessed latest artifact name.

The secret's user's Copilot entitlement/billing applies. One review is bounded by
**120 seconds plus a 10-second termination grace, 30 AI credits, and 16 KiB of
accepted output**. Thirty credits is this pinned CLI's minimum permitted session
cap, not a claim that every review consumes that amount. Copilot CLI is pinned to `v1.0.103` with an archive SHA-256;
auto-updates are disabled. The exact requested model is `gpt-6-luna`, not `auto`.
Unknown/unavailable models are failures, not an excuse to select another model.

Missing credentials, unavailable CLI/model access, budget exhaustion, timeout or
unusable output produce an explicit **AI unavailable** notice. Valid numeric
reports are still published. Incomplete/failed comparisons never invoke Copilot.
A missing AI review is **not** represented as a successful model assessment.

An ignored `.env` placeholder and a tracked `.env.example` document the optional
local variable. Nothing automatically loads `.env`; it is not Actions secret
configuration. The server, benchmark client, report generation and offline tests
require **no API key**.

Organization-owned repositories may separately enable GitHub's supported
`GITHUB_TOKEN` Copilot policy; this workflow deliberately uses the explicit,
least-privilege personal credential above rather than assuming that policy exists.
See [Copilot authentication](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli)
and [Copilot in Actions](https://docs.github.com/en/copilot/how-tos/copilot-cli/automate-copilot-cli/automate-with-actions).

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
verify AI conclusions; throughput/tail tradeoffs and shared-runner uncertainty
are not converted into a hard timing gate or a general performance victory.

## Workflow security and publication

1. **compare**: read-only repository token; builds/tests/benchmarks as before.
2. **review**: read-only repository token; downloads that job's exact artifact,
   validates it offline, then optionally runs Copilot. The model child has a fresh
   home/working directory, a minimal environment, no available model tools,
   explicit shell/write/URL denials, built-in MCP disabled and custom instructions
   disabled. Copilot private state, output and
   diagnostics are not included in uploaded public reports.
3. **publish**: separate, default-branch-only job with `contents: write`. It never
   runs inference or receives the Copilot secret. It checks report/run identity,
   stages only the dashboard/history paths and pushes a bot documentation commit.

Publication is serialized and compares `(workflow run number, review attempt)`:
a late rerun of an older run is archived but cannot overwrite the current page.
A failed latest attempt links back to the last complete successful numeric report.
Archive collisions are rejected, not overwritten. The job fetches current main
and retries ordinary pushes after concurrent changes, **never force-pushing** or
restoring an old source tree. Its hard reset operates only in its disposable
Actions checkout, not a developer's workspace. Built-in-token report pushes do
not recursively trigger new benchmark runs. Non-default branches can generate
Actions reports but receive no Copilot secret and cannot publish to main.

A cancelled workflow may not reach reporting; a runner-wide outage can also
prevent publication. These cases do not fabricate new results. If Git push is
blocked by repository policy, the report remains readable in the Actions summary
and publication fails visibly.

## Local verification

`make ci` includes offline tests of the archived 180-record cohort, corrupt-data
rejection, model-unavailable/error/unsafe-output handling, immutable history,
out-of-order/rerun publication and last-successful links. No inference is used by
tests. `cargo run --locked --offline --bin rillcache-review -- --help` describes
standalone generation/review/publication commands. The CLI prepares files only;
Git commits/pushes belong to the explicitly authorized publication workflow.

Copilot is an optional external CI review tool, like the optional native reference
servers are external benchmark tools. It is **not** an application dependency,
Rust dependency, native extension, FFI binding or part of the server release.

# What each merge gate proves

Every required context in `.github/pr-workflow-gates.json` is a **merge-quality**
check on shared CI. This table states the property each one actually proves and,
explicitly, what it does not. No row here is a production acceptance, an SLO, a
durability proof or a security review; `scripts/check_gate_scope.py` fails the
required CI graph if a context is unmapped, if a row loses its "does not prove"
column, or if this document claims a production pass.

## Gate map

| Required context | Proves | Does not prove |
| --- | --- | --- |
| `Tests and offline CI` | Builds the workspace, runs the offline test suite in every execution mode, the dependency/vendored audit and the documentation checks. | Any compatibility claim against Redis or Valkey, a latency or throughput figure, a durability proof, a security review, or a production SLO. |
| `Valkey performance gate` | The candidate beats the pinned Valkey build on the protected single-CPU cohort (20 medians) on the shared CI runner, with recorded cohorts and retained failures. | Absolute capacity, tail latency as a distribution, controlled hardware, other hardware generations, or any non-default mode. |
| `Valkey 4-CPU performance gate` | The candidate beats the pinned Valkey build on the protected four-CPU cohort (10 medians) on the shared runner. | That four-worker mode is the production recommendation, any SLO, or that shared-runner noise cannot cost a single row. |
| `verify` | The container image builds from the pinned toolchain and its in-image verification steps pass on the tested platform. | Runtime durability, compatibility or performance of the image, or any architecture that only builds. |
| `build (amd64, ubuntu-24.04)` | The amd64 container image builds and its in-image checks pass. | That the image is signed, published, reproducible, or behaviourally verified on amd64. |
| `build (arm64, ubuntu-24.04-arm)` | The arm64 container image builds on the arm runner. | Behavioural verification on arm64, which the platform matrix explicitly excludes. |
| `Build pinned TCP compatibility tools` | The candidate and both pinned reference servers build from the pinned sources and the Tcl harness prerequisites resolve. | That any upstream Tcl test executed or passed; it is the harness build stage only. |
| `Tcl redis / string` | The selected Redis 8.10.2 `unit/type/string` file is executed against the candidate and Redis with the pinned options and recorded exclusions. | Hash, list, set, sorted-set, stream, scripting, module, cluster, persistence or replication behaviour. |
| `Tcl redis / incr` | The selected Redis 8.10.2 `unit/type/incr` file is executed against the candidate and Redis. | Integer semantics outside that file, overflow/edge coverage in other families, or any performance property. |
| `Tcl redis / keyspace` | The selected Redis 8.10.2 `unit/keyspace` file is executed against the candidate and Redis, plus the embedded set/bitfield/scan/quit/geo blocks in that context. | Expiry, eviction, keyspace notifications, stream metadata or anything outside those files. |
| `Tcl valkey / string` | The same `unit/type/string` file is executed against Valkey 9.1.2 and the candidate. | That Valkey and Redis agree everywhere, or that unselected files are covered. |
| `Tcl valkey / incr` | The same `unit/type/incr` file is executed against Valkey 9.1.2 and the candidate. | Any family outside that file. |
| `Tcl valkey / keyspace` | The same `unit/keyspace` context is executed against Valkey 9.1.2 and the candidate. | Coverage of the supported profile's other families. |
| `read-only-self-tests` | The gate-inventory self-tests prove the required-check policy file and the workflow wiring still agree, including that this mapping is enforced. | Runtime behaviour of the product, or that any runtime gate passed. |

## What no merge gate proves

- It does not prove production readiness, and no combination of these contexts does:
  the canonical checklist stays open until its own evidence exists.
- It does not replace the independent security review (`S1`) or the independent
  acceptance sign-offs (`V3`), and merge approval by a maintainer is not a release
  sign-off (`docs/release-acceptance.md`).
- It does not measure controlled hardware: every latency and throughput row comes
  from shared GitHub runners (`P2`, `P5`, `P6`).
- It does not cover compatibility beyond the selected Tcl files, the differential
  corpus, the language-client matrix and the declared profile exclusions
  (`V1`, `V2`, `C7`).
- It does not prove durability: fsync policy behaviour, fault injection and recovery
  bounds are covered by `D10`, `D11` and the drill jobs, not by the merge gate.
- It does not exercise the non-default modes (TLS, persistence, Tokio, multi-owner)
  as production recommendations (`P5`, `R2`).

## Production gates still missing

These properties have **no** required merge gate, so they must be produced by the
release process before any production claim. Each identifier is an open checklist
item, and the checker fails if one is closed here without being closed there:

- `S1` — independent security review of the release profile.
- `V3` — independent acceptance sign-offs for profile, security, performance and recovery.
- `V6` — acceptance runbook executed against a release candidate.
- `P2` — controlled-hardware capacity and SLO evidence.
- `P6` — long-duration regression budgets and stability trends.
- `D10` — measured fsync-policy behaviour per durability mode.
- `D11` — recovery across the supported failure and disk matrix.
- `R6` — persistence contention measured against an accepted SLO.

## Where the production evidence lives

- Merge-gate identity and the required set: `.github/pr-workflow-gates.json`.
- Release contract and the gates a tag needs: `docs/release-lifecycle.md`, `scripts/cut-release.sh`.
- Acceptance recording and sign-offs: `docs/production-readiness/acceptance-runbook.md`,
  `docs/release-acceptance.md`, `docs/acceptance/signoffs.tsv`.
- Readiness inventory: `docs/production-readiness-gaps.md` (the authoritative list).

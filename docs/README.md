# Rillcache documentation

Start with the guide that matches what you want to do. Command details,
implementation studies and delivery policies live here rather than in the
project's front page.

## Start here

| Goal | Guide |
| --- | --- |
| Store the first key, build locally or change a limit | [Getting started](getting-started.md) |
| Check whether an application's commands will work | [Compatibility](compatibility.md) |
| Inspect measured throughput and its scope | [Performance](performance.md) |
| Compare all five servers in the daily run | [Daily benchmarks](daily-benchmarks.md) |
| See the draft supported deployment profile | [Production profile](production-profile.md) |
| Understand remaining production work | [Production readiness](production-readiness.md) |
| Find every remaining production blocker | [Production readiness gaps](production-readiness-gaps.md) |
| Check a release candidate before promotion | [Production acceptance runbook](production-readiness/acceptance-runbook.md) |
| Verify container package visibility before publishing | [Package visibility record](acceptance/package-visibility.md) |
| Handle a disk, memory, credential or journal incident | [Incident runbook](incident-runbook.md) |
| Compare durability modes under one matched workload | [Durability performance profile](persistence-performance.md) |
| See which upstream versions and clients are actually exercised | [Tested version matrix](compatibility-matrix.md) |
| Reuse, redistribute or package Rillcache | [Release licensing](release-licensing.md) |
| Cut, support or upgrade a release | [Release and support lifecycle](release-lifecycle.md) |
| See who accepted which production claim | [Production acceptance sign-offs](release-acceptance.md) |
| Give a coding agent repository context | [OpenWiki](../openwiki/quickstart.md) or its [published wiki](https://macel94.github.io/rillcache/) |

## Run and operate

- [Docker, Compose and Kubernetes](containers.md): images, resource isolation, networking and lifecycle.
- [Startup configuration](configuration.md): CLI flags and the bounded `--config` startup file.
- [Security](security.md) and [TLS](tls.md): passwords, ACLs, audit tooling and encrypted listeners.
- [Overload and slow-reader policy](overload-policy.md): enforced thresholds and what clients see when they cross one.
- [Persistence](persistence.md): the optional journal, replay, compaction and durability boundaries.
- [Eviction](eviction.md): payload budgets and memory policies.
- [Observability](observability.md) and [runtime introspection](introspection.md): counters, logs, latency and logical memory reporting.
- [Client sessions](client-sessions.md) and [discovery](discovery.md): protocol negotiation, connection controls and supported metadata.
- [Pinned language-client matrix](language-clients.md): the versioned redis-py, node-redis and go-redis case evidence for the standalone profile.
- [Supported platforms](platforms.md): the tested platform, the built-but-untested targets and the local-filesystem contract.
- [Standalone topology](replication.md) and [unsupported capabilities](unsupported.md): honest replication, cluster and module boundaries.

## Command reference

The [compatibility guide](compatibility.md) groups the available surface. For
option-level behavior and limits, use the reference for the command family:

- [Strings](strings.md), [multi-key strings](multikey-strings.md), [string ranges](string-ranges.md), [bitmaps/LCS/float operations](string-algorithms.md) and [keyspace](keyspace.md).
- [Lists, sets, hashes and sorted sets](collections.md), plus [SORT](sort.md).
- [Geospatial](geospatial.md), [HyperLogLog](hyperloglog.md) and [streams](streams.md).
- [Transactions](transactions.md), [pub/sub](pubsub.md) and [keyspace notifications](notifications.md).

## Develop and investigate

[Contributing](../CONTRIBUTING.md) describes the project goals, AI-generated code
policy, human direction/review, Rust safety boundary, approved dependencies and
required validation. There are no application API keys or separate services to
configure for the ordinary offline development loop.

- [Development cycle](development-cycle.md): focused feedback before full validation.
- [Requirements](requirements.md): implemented stories and testable acceptance criteria.
- [Reference conformance](reference-conformance.md) and [original upstream evidence](upstream-ci.md): what has actually been tested.
- [Benchmark methodology](benchmarks.md), [dashboard](../benchmarks/README.md) and [reporting](benchmark-reports.md): measurements and publication.
- [Protected delivery](pr-quality-gate.md), [check inventory](actions-gate-audit.md), [independent PR delivery](parallel-pr-delivery.md) and [parallel work lanes](parallel-work-lanes.md): repository governance.
- [What each merge gate proves](gate-scope.md): the property behind every required context, the production properties no merge gate covers, and where release evidence lives.

### Architecture and research

[Architecture approaches](architecture-comparison.md) compares Redis, Valkey,
Dragonfly and Rillcache at the execution, I/O, data-placement and topology level,
with diagrams and an explicit borrow/reject ledger.

The default Unix plain-TCP frontend uses one owner-local Mio reactor, with opt-in
socket workers that move a data connection once to its local data owner. The
[hot-path guide](hot-paths.md) and
[performance architecture](plans/performance-architecture.md) explain this design.
[Opt-in Tokio transport](async-transport.md), [multi-owner execution](plans/multi-owner-execution.md),
[owner-local storage](plans/owner-local-storage.md) and the [sharding seam](sharding.md)
are advanced reference, not prerequisites for using the default server.

Earlier [I/O scheduling](io-scheduling.md) and [client handoff](client-io-handoffs.md)
studies preserve historical paths, resource costs and rejected approaches. Do not
read an old experimental result as the behavior or performance of today's default.

[Project home](../README.md)

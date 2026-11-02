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
| Understand remaining production work | [Production readiness](production-readiness.md) |
| Find every remaining production blocker | [Production readiness gaps](production-readiness-gaps.md) |

## Run and operate

- [Docker, Compose and Kubernetes](containers.md): images, resource isolation, networking and lifecycle.
- [Security](security.md) and [TLS](tls.md): passwords, ACLs, audit tooling and encrypted listeners.
- [Persistence](persistence.md): the optional journal, replay, compaction and durability boundaries.
- [Eviction](eviction.md): payload budgets and memory policies.
- [Observability](observability.md) and [runtime introspection](introspection.md): counters, logs, latency and logical memory reporting.
- [Client sessions](client-sessions.md) and [discovery](discovery.md): protocol negotiation, connection controls and supported metadata.
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

### Architecture and research

The default Unix plain-TCP frontend co-locates Mio I/O with its single data
owner. Explicit parallel socket workers move a data connection once to that owner. The [hot-path guide](hot-paths.md) and
[performance architecture](plans/performance-architecture.md) explain this design.
[Opt-in Tokio transport](async-transport.md), [multi-owner execution](plans/multi-owner-execution.md),
[owner-local storage](plans/owner-local-storage.md) and the [sharding seam](sharding.md)
are advanced reference, not prerequisites for using the default server.

Earlier [I/O scheduling](io-scheduling.md) and [client handoff](client-io-handoffs.md)
studies preserve historical paths, resource costs and rejected approaches. Do not
read an old experimental result as the behavior or performance of today's default.

[Project home](../README.md)

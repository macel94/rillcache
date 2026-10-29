# Parallel work lanes

This document is the operating map for multiple independent AI agents working on
this repository at the same time, on one machine or many. It is advisory: the
[PR quality gate](pr-quality-gate.md) and the protected branch rules remain the
only merge authority.

The goal is to parallelize **development** while keeping **delivery** serialized
and keeping every agent's edits inside a small, predictable set of files.

## Why this map exists

Merge history shows that the expensive contention is not simultaneous compile
time. It is shared files plus a moving base:

- `docs/requirements.md` was touched by 58 of 60 consecutive merges;
  `docs/production-readiness.md` by 49; `src/engine/discovery.rs` by 38;
  `src/engine.rs` by 29; `src/session.rs` by 23.
- The median PR lifetime was 8 minutes, but a few long-lived PRs dominated the
  cost: one waited 10.1 hours through 30 main merges, another 5.5 hours through
  14. Short-lived PRs typically saw only one main advance.
- The strict 20/20 Valkey gate is invalidated by a main advance. Refreshing
  every open PR at once therefore creates a herd of new heads, new gate runs and
  new noise failures.
- More machines increase development capacity, not merge capacity. Past roughly
  two or three in-flight gate-bearing PRs, an extra machine mostly creates
  rebases and gate reruns for the others.

## Lane map

A lane owns a path while a task is in flight. It is a lease for coordination, not
permanent code ownership. Pick one lane per task and record it in the PR.

| Lane | Primary paths | Typical work |
| --- | --- | --- |
| `streams` | `src/engine/streams.rs`, `docs/streams.md`, `tests/streams*.rs` | stream commands, consumer groups, blocking stream reads |
| `collections` | `src/engine/collections.rs`, `collection_commands.rs`, `collection_config.rs`, `sort.rs`, `geo.rs`, `bitfield.rs` | lists, sets, hashes, sorted sets, geo, sort, bitfields |
| `strings-keyspace` | `src/engine/data.rs`, `keyspace.rs`, `multikey.rs`, `string_algorithms.rs`, `string_ranges.rs`, `scan.rs`, `eviction.rs` | strings, expiry, keyspace, scan, payload budget |
| `security-clients` | `src/engine/auth.rs`, `acl_log.rs`, `clients.rs` | authentication, ACLs, client registry and kills |
| `persistence` | `src/engine/journal.rs`, `docs/persistence.md` | append-only journal, replay, rewrite and fsync |
| `observability-admin` | `src/engine/slowlog.rs`, `latency.rs`, `monitor.rs`, `administration.rs`, `introspection.rs`, `unsupported.rs` | INFO, stats, slowlog, latency, monitor, admin surfaces |
| `pubsub` | `src/engine/pubsub.rs`, `docs/pubsub.md` | publish/subscribe, notifications delivery, sharded channels |
| `io-server` | `src/server.rs`, `src/server/pool.rs`, `src/protocol.rs`, `src/bin/**` | listener, pool, worker handoff, protocol framing |
| `surface` | `src/engine.rs`, `src/engine/shard.rs`, `src/session.rs`, `src/engine/discovery/mod.rs`, `docs/requirements.md`, `docs/requirements/**`, `docs/production-readiness.md`, `docs/sharding.md`, `README.md`, `tests/discovery.rs`, `tests/core_dispatch.rs` | cross-cutting dispatch, session state, registry and requirement indexes, keyspace routing seam |
| `tooling` | `scripts/**`, `.github/**`, `Makefile`, `Cargo.toml`, `benchmarks/**` | gates, automation, tooling and benchmark harness |

After the command registry is split into `src/engine/discovery/<family>.rs`, those
family files belong to the matching feature lane. Only `discovery/mod.rs` stays a
shared surface.

`tests/**` and `docs/**` files follow the lane of the behavior they cover unless a
more specific row above assigns them. New documentation that is not tied to a
feature belongs to `tooling`.

## Rules for each agent

1. **One task, one worktree, one branch, one PR.** Start from a fresh
   `origin/main`; never reuse a merged branch or a stale worktree.
2. **One active gate-bearing PR per machine**, and at most two or three across the
   whole repository. If a PR is not close to mergeable within about 60 minutes,
   split it or park it instead of keeping it open through many main advances.
3. **Declare the lane and shared surfaces** in the PR body. If the task must edit a
   `surface` file, say why; prefer a separate surface-lane increment.
4. **Do not share a working tree over NFS or a network mount.** Each machine gets
   its own clone/worktree. Run `git worktree prune` after finishing.
5. **Do not run the strict Valkey gate locally from several machines.** Keep the
   gate on one dedicated or serialized runner. Concurrent benchmark runs create
   the noise that fails the 20/20 medians.
6. **Only the queue-head PR should be armed and refreshed.** Other PRs wait
   without auto-merge. When the head lands, refresh the next candidate once.
   The serialized-updater change is tracked separately; until it lands, do not
   manually refresh every open PR.
7. **No force-push to shared branches, no admin bypass, no gate weakening and no
   rerun-until-green.** Preserve failed evidence and report genuine blockers.

## Daily loop

```text
fetch origin/main
  -> create a fresh worktree for one lane/task
  -> implement, test and document
  -> run: python3 scripts/lane_check.py
  -> fix shared-surface spill if it was accidental
  -> make ci (serialize with flock on a busy host)
  -> commit, push, open a ready PR with the lane declared
  -> arm auto-merge only when this PR is the delivery head
  -> land, then refresh the next candidate once
```

Use the helper for a fresh worktree:

```sh
scripts/new-lane-worktree.sh feat/streams-claim streams
cd /root/worktrees/rillcache-feat-streams-claim
python3 scripts/lane_check.py
```

The advisory checker classifies the current branch's changed paths, lists the
lanes involved and calls out shared-surface edits:

```sh
python3 scripts/lane_check.py                 # human-readable, advisory
python3 scripts/lane_check.py --json          # machine-readable
python3 scripts/lane_check.py --strict        # fail on shared-surface spill
python3 scripts/lane_check.py --allow-shared  # surface-lane PR
```

`--strict` is intended for local preflight. It is not wired into required CI so
that it cannot accidentally become a new merge policy without owner review.

## Delivery topology

- **Two or three feature machines**, each with one lane and one unarmed PR.
- **One surface/docs lane** only while the registry and requirements files remain
  monolithic. After the splits, this lane shrinks to rare cross-cutting changes.
- **One dedicated gate runner** (or GitHub-hosted serialized gate). Do not turn
  developer machines into concurrent benchmark runners.
- **One shared claim board** (a GitHub Project, issue label or short table in the
  PR body) so two agents do not choose the same lane. This is a visibility aid,
  not a central implementation orchestrator.

## Metrics that tell us whether it is working

Track these per week, not per PR:

- PR lead-time median and p90, and the number of main merges during each PR.
- Percentage of PRs touching a `surface` file outside the surface lane.
- Gate reruns per merged PR and failures caused by a single noisy row.
- Rebase/refresh events per PR and conflict-resolution time.
- Merge throughput (currently about three merges per hour).

The target is not "more open PRs". It is more completed increments per hour with
fewer rebases and fewer invalidated gate runs.

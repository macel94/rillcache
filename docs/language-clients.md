# Pinned language-client matrix (draft)

**Status: draft evidence for C7. NOT a support commitment.**

This page describes the versioned language-client matrix in
[`ci/upstream/clients-v1.json`](../ci/upstream/clients-v1.json) (matrix
`rillcache-language-clients`, version `1`). It is C7 evidence for the
[standalone profile](production-profile.md), and it is executed by
`scripts/upstream/client_matrix.py` in the
`Pinned language-client matrix` job of
[`upstream-compatibility.yml`](../.github/workflows/upstream-compatibility.yml).

A green matrix is per-client evidence for the exact tested head only. It is not a
support window, release acceptance, or a claim that other client versions or
languages behave identically.

## Pinned clients

| Client | Pinned version | Language | Lock file | Harness |
| --- | --- | --- | --- | --- |
| `redis-py` | `5.2.1` | Python | `scripts/redis-py-requirements.txt` (version + wheel sha256) | `scripts/redis_py_conformance.py --matrix` |
| `node-redis` | `6.4.0` | JavaScript | `ci/clients/node-redis/package-lock.json` (tarball integrity) | `ci/clients/node-redis/harness.mjs` |
| `go-redis` | `v9.14.0` | Go | `ci/clients/go-redis/go.mod` + `go.sum` (module checksums) | `ci/clients/go-redis/main.go` |

The runner refuses to run when the manifest version differs from the pinned lock
file, when a harness is absent, or when a client reports a different version at
runtime than its pin.

## Executed case set

Every client executes the same declared cases against fresh candidate
listeners. The runner starts three servers from the same built candidate: a
plaintext listener with a password/ACL file, a TLS listener presenting the test
CA identity, and a TLS listener presenting a certificate without the target IP
SAN.

| Case | What it proves |
| --- | --- |
| `handshake-auth` | Named-user authentication and `PING` succeed |
| `acl-allowed` | The `+@all` named user round-trips its own key |
| `acl-denied-command` | The limited user receives `NOPERM` for a command outside its set |
| `resp2-roundtrip` | RESP2 covers `PING`, binary `SET`/`GET`, `INCR` and `HSET`/`HGETALL` |
| `resp3-roundtrip` | RESP3 (`HELLO 3`) covers the same round-trip |
| `pipeline` | A batched pipeline returns ordered replies for every queued command |
| `transaction` | `MULTI`/`EXEC` returns ordered results and commits state |
| `pubsub` | A subscriber receives a published payload from a second connection |
| `error-wrong-type` | Wrong-type access surfaces `WRONGTYPE` and leaves the client usable |
| `error-unknown-command` | An unknown command surfaces an error without disconnecting |
| `error-auth-required` | A connection without credentials receives an authentication error |
| `reconnect` | A dropped connection reconnects and still reads committed data |
| `tls-positive` | TLS with the test CA authenticates and completes `SET`/`GET` |
| `tls-wrong-identity` | A certificate without the target IP SAN is rejected |

A missing, failed, duplicated or undeclared case fails the job. A client may
declare an exclusion for a case only with a reviewable reason in the manifest;
an excluded case that still executes is a failure.

## How the job runs

The job depends on the pinned build bundle, installs each client from its lock
file, builds the Go harness, and runs
`python3 scripts/upstream/client_matrix.py --output target/upstream/evidence/client-matrix --candidate target/upstream/rillcache`.
It is a declared auxiliary dependency of the required Tcl contexts, so a failed
client case blocks the PR without adding or relaxing a required check.

The evidence directory retains `result.json`, `summary.md`, one `cases.json` and
`harness.log` per client, and the owned server logs with the exact argv and ACL
path. Artifacts upload even on failure.

The first hosted evidence run for head `e0807d9e` is `Pinned language-client
matrix` job 140227942914 in run
[45913387009](https://github.com/macel94/rillcache/actions/runs/45913387009):
14/14 declared cases passed for each of the three pinned clients with no
failures. Treat it as per-head evidence, not a standing compatibility claim.

Validate the manifest offline without servers or client runs:

```sh
python3 scripts/upstream/client_matrix.py --check
```

## Boundaries

The matrix covers one pinned version per client, one connection per case and the
TLS identity above. It does not cover client connection pools under load,
cluster/Sentinel routing, client-side caching, pipelining throughput, RESP3 push
tracking, certificate revocation, mutual TLS or other client languages and
versions. Those are not part of this profile; the remaining C7 work and the
wider readiness gaps stay in the
[canonical checklist](production-readiness-gaps.md).

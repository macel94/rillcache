# Changelog

All notable changes to Rillcache are recorded here. Format follows the release
contract in `docs/release-lifecycle.md`: every entry groups changes by release,
states migration and rollback position, and names the evidence behind any
compatibility claim.

## Unreleased

_No unreleased changes recorded yet._

## 0.1.0 — 2026-11-27

Release notes: [docs/releases/v0.1.0.md](docs/releases/v0.1.0.md). The tag
`v0.1.0` exists; the readiness checklist is unchanged by tagging and still
reports the project as not production ready.

First versioned release candidate. The release action remains with the release
owner and is gated by `scripts/cut-release.sh`.

### Included

- RESP2/RESP3 server surface with the compatibility profile recorded in
  `docs/compatibility.md`; unsupported capabilities are listed in
  `docs/unsupported.md` rather than implied.
- Optional journal persistence with replay, compaction and documented durability
  boundaries (`docs/persistence.md`).
- Authentication, ACLs, TLS listeners and audit tooling (`docs/security.md`,
  `docs/tls.md`).
- Observability and runtime introspection counters (`docs/observability.md`,
  `docs/introspection.md`).
- Container images with signed SBOM attestations and provenance for the tested
  platform digests (`container/`).

### Migration

None: `0.1.0` is the first versioned artifact, so there is no previous release
format to migrate from. Data written by a pre-release `main` build of the same
journal format replays unchanged; a pre-release build with an older format must be
dumped and restored.

### Rollback

Rolling back from `0.1.0` to an untagged `main` build of the same journal format
is supported. Rolling back across a journal format change is unsupported and must
be documented per release.

### Known limitations

- Standalone only: replication, failover and cluster topology are excluded from
  the profile (`docs/unsupported.md`).
- Remaining production-readiness gaps are tracked in
  `docs/production-readiness-gaps.md`; passing merge gates is not equivalent to
  those acceptance criteria.

# Rillcache wiki brief

Write for coding agents and maintainers who need to change Rillcache safely, not
for end users choosing a cache. Prioritize:

- the real request path from socket acceptance through parsing, dispatch, owner
  execution, reply encoding and persistence;
- ownership and concurrency rules: socket workers, data owners, mailboxes,
  exclusive gates, sharding and shutdown;
- data-model invariants: expiration, payload accounting, eviction, journal
  ordering and replay;
- protocol and client-session behavior, including RESP2/RESP3 limits, blocking,
  transactions, pub/sub and notifications;
- security and operations boundaries: authentication, ACLs, TLS, audit,
  configuration, observability and health;
- proof: tests, benchmark gates and the documented production-readiness gaps.

Use British English and sentence-case headings. Link related concepts instead of
duplicating whole explanations. Name exact source paths and line anchors in
Grounded Claims. Do not describe the repository as production ready or claim
Redis compatibility beyond what tests and the compatibility guide establish.
Do not document `vendor/`, `target/` or generated benchmark output.

## Maintenance

The visualizer deploys automatically on every `main` push. A separate workflow
checks versioned source evidence on main pushes and weekly; when evidence is stale,
it opens or updates a maintenance issue listing affected pages and citations. The
audit never rewrites or re-signs Claims: stale evidence needs semantic review.

Refresh this wiki from the repository root with a configured, approved OpenWiki
provider:

```sh
openwiki --update
```

Review the generated diff and run the freshness audit before committing. Keep the
source tree and tests authoritative; the wiki is a navigation and evidence aid,
not a replacement for them.

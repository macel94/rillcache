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

There is no checked-in scheduled OpenWiki workflow. Refresh this wiki manually
from the repository root with a configured OpenWiki provider:

```sh
openwiki --update
```

Review the generated diff and run the relevant tests before committing. Keep the
source tree and tests authoritative; the wiki is a navigation and evidence aid,
not a replacement for them.

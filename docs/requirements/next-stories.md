# Next independently deliverable stories

Part of the [requirements and incremental delivery index](../requirements.md). Requirements are preserved verbatim from the former monolithic EARS sections; add new requirements only in the matching area file.

Not implemented or implied by the completed increments:

1. Validate HELLO/CLIENT session negotiation against real Python/Node/Go language
   clients; pinned `redis-cli` RESP2/RESP3/auth coverage is implemented (S103).
   Add command discovery and multiple databases only as separate compatible
   increments.
2. Global/cross-process rate limiting, collector sender authentication/TLS/
   federation, and full-chain/key-ID/hot-rotation audit custody. A per-connection
   failed-auth budget (S105), a per-peer-IP throttle (S106), a bounded local ACL
   denial audit (S104), best-effort UDP off-host shipping (S107), per-line HMAC
   signing (S109), a bounded UDP collector (S110), shared-key collector
   verification (S111) and manual audit keyring rotation (S115) are implemented.
   Native TLS 1.3 via rustls provides the current transport-encryption boundary;
   SNI/hot reload, OCSP/CRL and independent security review remain separate.
   Durability completeness, AOF rewrite/RDB, backup/restore drills and disk-fault
   recovery are explicitly deferred for this phase.
3. Richer data types, scripts, pub/sub, expanded streams, replication and
   clustering: follow the [production-readiness plan](../production-readiness.md).
   NX plus TTL alone is not a complete distributed-lock protocol; token-safe
   release, failover and fencing semantics are not implemented.
4. Extend the [measured client-I/O gains](../../benchmarks/studies/client-handoffs/README.md)
   beyond the current cohort: investigate idle accept polling, explicit fixed-worker
   and multicore tails, mixed TTL/traffic and client/server saturation, using PING
   as a no-key-map control. Evaluate
   a safe readiness boundary before speculative sharding; retain idle CPU,
   backpressure, shutdown and multi-key atomicity gates. Unsafe project code and
   unreviewed Rust dependencies outside the manifest's authorized exceptions
   remain forbidden.
   One-owner local execution and the external compatibility bridge are in place
   ([ownership design](../plans/owner-local-storage.md)); complete multi-owner
   coordination and the nonblocking thread-per-core target remain unimplemented.
5. Controlled-hardware performance budgets and repeated current-release comparisons;
   later, richer types, pub/sub and additional stream capabilities driven by product judgment.

See the [definition of done](../requirements.md#definition-of-done) in the index.

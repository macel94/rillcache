# Client protocol and discovery

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Client protocol and discovery | Partial implementation: HELLO 2/3, bounded CLIENT ID/name/library metadata, compact CLIENT INFO, SELECT 0, frame-aware RESP3 null conversion, all-mode ownership tests, and bounded COMMAND/INFO discovery including LIST FILTERBY pattern/category/module for implemented commands. Pinned `redis-cli` RESP2/RESP3, AUTH and client-metadata interop is covered (S103); still required: Python/Node/Go language-client handshake/interoperability tests and any separately justified richer protocol/database support. No forged Redis version or successful no-op for unsupported behavior. |

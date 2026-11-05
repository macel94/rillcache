# Files

- [Network frontends and client handoff](network-frontends.md) - How Rillcache accepts sockets, schedules client I/O, moves data connections to the owner and selects the optional Tokio and TLS frontends.
- [Ownership, mailboxes and sharding](ownership-and-sharding.md) - How Rillcache keeps mutable data on one thread, routes keys through virtual partitions and coordinates cross-owner commands, blocking waits and shutdown.
- [Request lifecycle](request-lifecycle.md) - Trace a Rillcache command from buffered socket bytes through bounded RESP decoding, session policy, owner execution, journal capture and reply writeback.
- [Runtime architecture overview](runtime-overview.md) - A map of Rillcache's binary, transport, session, engine, storage and persistence layers, including which thread owns each kind of state.

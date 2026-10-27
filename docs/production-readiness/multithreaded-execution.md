# Multithreaded execution

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Multithreaded execution | Managed per-client workers plus opt-in fixed 1..64 I/O/execution lanes; test socket behavior and benchmarks in per-client/1/4-worker modes, actual worker counts, idle clients, backpressure, fairness, limits and joined shutdown. No linear-scaling claim. |

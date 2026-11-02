# Files

- [Data model, expiration and memory accounting](data-model-and-expiry.md) - Rillcache's per-key Entry, absolute key and field expiry, lazy and maintenance reaping, payload accounting, WATCH versions and bounded eviction policies.
- [Journal persistence and recovery](persistence-and-recovery.md) - The private RESP append-only journal, its deterministic final-state records, fsync acknowledgement windows, replay and truncation rules, synchronous rewrite and remaining durability gaps.

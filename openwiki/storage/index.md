# Files

- [Data model, expiration and memory accounting](data-model-and-expiry.md) - Rillcache's per-key Entry, absolute key and field expiry, lazy and maintenance reaping, payload accounting, WATCH versions and bounded eviction policies.
- [Journal persistence and recovery](persistence-and-recovery.md) - The private RESP append-only journal, CRC32 and optional HMAC records, fsync acknowledgement windows, replay and truncation rules, cooperative rewrite, offline backup and remaining durability gaps.

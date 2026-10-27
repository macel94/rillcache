# Bounded SORT compatibility

`SORT` and `SORT_RO` order the elements of a list, set or sorted set without
copying them into a second index. The source key and any `STORE` destination
share the normal type, expiry, payload-accounting and keyspace behavior.

## Supported syntax

```text
SORT key [ASC|DESC] [ALPHA] [LIMIT offset count] [STORE destination]
SORT_RO key [ASC|DESC] [ALPHA] [LIMIT offset count]
```

- Numeric sorting is the default. Elements must parse as finite or infinite
  doubles; NaN and trailing garbage are rejected. Invalid numeric input returns
  `ERR One or more scores can't be converted into double` without a partial
  response or store.
- `ALPHA` sorts raw binary member bytes lexicographically.
- `ASC` is the default; `DESC` reverses the ordering. Equal keys are ordered by
  member bytes for deterministic replies (Redis leaves equal-key ordering
  unspecified).
- `LIMIT offset count` clamps a negative offset to zero, treats a negative count
  as unbounded, returns an empty result when the offset is beyond the source, and
  applies after sorting.
- Missing source keys return an empty array.
- `STORE destination` writes the selected values to a list, replaces any existing
  destination type, clears its TTL, returns the stored count and deletes the
  destination when the selected result is empty. `source == destination` reads the
  pre-command source before replacing it. A wrong-type source or payload-budget
  OOM leaves the destination unchanged.
- `SORT_RO` is the read-only variant and rejects `STORE`.
- A successful store publishes `sortstore`; an empty store that deletes an
  existing destination publishes `del`.

The source key and an optional `STORE` destination are included in ACL key
checks. The commands expose `@generic` plus `@list`, `@set`, `@zset` and
`@dangerous` approximations in command discovery.

## Limits

This is deliberately a bounded subset: `BY` and `GET` patterns are rejected
rather than silently ignored. Their derived keys depend on source contents, so
supporting them safely requires a separate dynamic-key ACL and lookup-bound
design. There is no cluster routing, no external sort operator and no fabricated
partial result. Collection size and payload limits are the existing one-million
element and configured byte budgets. SORT is not a durable index; rerun it after
source mutations.

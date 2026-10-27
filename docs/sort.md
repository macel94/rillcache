# Bounded SORT compatibility

`SORT` and `SORT_RO` order the elements of a list, set or sorted set without
copying them into a second index. The source key and any `STORE` destination
share the normal type, expiry, payload-accounting and keyspace behavior.

## Supported syntax

```text
SORT key [BY pattern] [LIMIT offset count] [GET pattern ...] [ASC|DESC]
          [ALPHA] [STORE destination]
SORT_RO key [BY pattern] [LIMIT offset count] [GET pattern ...] [ASC|DESC] [ALPHA]
```

- Numeric sorting is the default. Elements must parse as finite or infinite
  doubles; NaN and trailing garbage are rejected. Invalid numeric input returns
  `ERR One or more scores can't be converted into double` without a partial
  response or store.
- `ALPHA` sorts raw binary member bytes lexicographically.
- `ASC` is the default; `DESC` reverses the ordering. Equal keys are ordered by
  member bytes for deterministic replies (Redis leaves equal-key ordering
  unspecified).
- `BY pattern` substitutes the first `*` in the pattern with each element to
  read a weight. A `->field` suffix dereferences a hash field. Missing or
  wrong-type weight keys contribute numeric zero or an empty ALPHA weight;
  existing but non-numeric numeric weights return the documented conversion
  error. A `BY` pattern without `*` disables sorting and preserves list/zset
  native order (set order is byte-sorted for determinism), reversed by DESC.
- `LIMIT offset count` clamps a negative offset to zero, treats a negative count
  as unbounded, returns an empty result when the offset is beyond the source, and
  applies after sorting.
- `GET pattern ...` substitutes each selected element into each pattern and
  returns the referenced string/hash value, with `GET #` returning the element
  itself. Missing or wrong-type references return null in replies and an empty
  string when stored. Multiple GET patterns multiply the reply length.
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
checks. Because `BY pattern` with `*` and every `GET pattern` derive key names
from source contents, ACL authorization requires the user to have all-key access
(`allkeys`); restricted users are denied before any derived lookup. A constant
`BY` pattern (no `*`) is allowed with source-key access. The commands expose
`@generic` plus `@list`, `@set`, `@zset` and `@dangerous` approximations in
command discovery.

## Limits

Derived BY/GET lookups are linear in the selected source elements; there is no
cluster routing, external sort operator or cached index. Missing derived keys are
represented as zero/empty/null rather than fabricated values. Collection size and
payload limits are the existing one-million element and configured byte budgets.
SORT is not a durable index; rerun it after source mutations. Users with
`allkeys` can read any derived key, so grant that permission only to trusted
operators.

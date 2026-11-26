# Geospatial compatibility

Rillcache stores geographic points in ordinary sorted sets: a member's score is
the 52-bit interleaved WGS-84 geohash used by Redis (26 longitude bits and 26
latitude bits). TYPE, EXISTS, DEL, TTL, expiry, RENAME, COPY, SCAN, ZADD/ZREM and
payload accounting therefore apply to geo keys without a second index. The
latitude range is the EPSG:900913/Web-Mercator range `-85.05112878..85.05112878`;
longitude is `-180..180`.

## Supported commands

- `GEOADD key [NX|XX] [CH] longitude latitude member ...` validates every
  coordinate before touching the key, encodes all triples, and applies the normal
  sorted-set `ZADD` semantics. `NX`, `XX` and `CH` behave as in `ZADD`; existing
  TTLs are preserved and failed/admission-limited writes leave the previous value
  intact. `NX` with `XX` is a syntax error.
- `GEOPOS key member [member ...]` returns one `[longitude, latitude]` pair per
  member with 17 fractional digits and trailing zeroes removed, or a null array
  for a missing or non-decodable member. A missing key returns one null array per
  requested member.
- `GEOHASH key member [member ...]` decodes each point and re-encodes it with the
  standard `-180..180`/`-90..90` geohash ranges, returning 11-character base-32
  strings. Missing members return null.
- `GEODIST key member1 member2 [M|KM|FT|MI]` returns the great-circle distance in
  the requested unit (metres by default) with exactly four fractional digits, or
  null if either member is missing. Units are case-insensitive.
- `GEOSEARCH key ((FROMMEMBER member | FROMLONLAT longitude latitude)
  (BYRADIUS radius unit | BYBOX width height unit) | BYPOLYGON vertex-count
  longitude latitude ...)
  [ASC|DESC] [COUNT count [ANY]] [WITHCOORD] [WITHDIST] [WITHHASH]` returns
  matching members. Radius/box searches require one origin and one shape;
  polygon searches take at least three coordinate pairs and reject an origin.
  Polygon membership uses the pinned Valkey ray-casting rule in longitude/latitude
  space; its spherical Cartesian vertex centroid supplies the distance/sort origin.
  Radius/box dimensions must be non-negative; `COUNT` must be positive and `ANY`
  requires `COUNT`. `COUNT` without an order implies ascending distance. `WITHDIST`
  reports the distance in the search unit, `WITHHASH` the stored 52-bit score and
  `WITHCOORD` the decoded `[longitude, latitude]` pair.
- `GEORADIUS key longitude latitude radius unit [options]` and
  `GEORADIUSBYMEMBER key member radius unit [options]` provide the legacy radius
  API over the same search. Options may appear in any order: `WITHCOORD`,
  `WITHDIST`, `WITHHASH`, `COUNT count [ANY]`, `ASC`/`DESC`, `STORE key` and
  `STOREDIST key`. The last store option wins; `STORE` writes geohash scores and
  `STOREDIST` writes distances in the requested unit. The read-only
  `GEORADIUS_RO`/`GEORADIUSBYMEMBER_RO` variants reject both store options.
  Combining a store option with `WITHCOORD`/`WITHDIST`/`WITHHASH` fails with the
  legacy GEORADIUS incompatibility error.
- `GEOSEARCHSTORE destination source ... [STOREDIST]` stores the same matches as
  a sorted set and returns the stored count. The destination is replaced
  atomically, its previous TTL is cleared, and an empty result deletes it.
  `STOREDIST` stores the distance in the requested unit instead of the geohash
  score. `WITHCOORD`/`WITHDIST`/`WITHHASH` are rejected for the store variant.
- `FROMMEMBER` on an existing source with a missing or non-decodable member
  returns the selected vendor's pinned error wording. A missing source yields an
  empty reply, or deletes the store destination and returns zero.

Searches derive the same center-plus-neighbor geohash cells used by the pinned
servers, then scan the source's bounded sorted-set items in cell-prefix order.
The implementation retains O(N) source-scan work (up to nine score ranges) rather
than a native skiplist range index, and memory stays bounded by the existing
one-million-element/payload limits. It applies the exact Redis haversine formula
(`EARTH_RADIUS_IN_METERS = 6372797.560856`), rectangle checks, or polygon
ray-casting. Unsorted results and `COUNT ... ANY` retain deterministic geohash-cell
traversal order; sorted searches order by distance. Native-first `unit/geo` is the
compatibility gate; a source change alone is not a compatibility claim.

`GEOADD` changes publish a `zadd` keyspace event. `GEOSEARCHSTORE` publishes
`geosearchstore` when it writes a destination and `del` when an empty result
removes one. Legacy `GEORADIUS*` store variants commit journal state but publish
no separate destination event, matching standalone Redis.

## Limits

This is not full geospatial compatibility. The legacy `GEORADIUS` family is
implemented as an adapter over the same bounded geohash-cell traversal. The
candidate does not have the native sorted-set range index, so each search still
scans source items and is O(N).
Scores written by ordinary `ZADD` are decoded best-effort: non-finite or negative
scores are treated as non-geographic, while arbitrary finite scores may map to
clamped coordinates. Geo keys are still sorted sets, so `ZREM`/`ZADD` can change
them outside the geo API. Replication, native RDB interchange and cross-process
durability retain their general project limits; the append-only journal rebuilds
geo keys as `ZADD` records.

Reference research: pinned Redis `src/geo.c`, `src/geohash.c`,
`src/geohash_helper.c` and the Redis command metadata for `GEOADD`, `GEOPOS`,
`GEODIST`, `GEOHASH`, `GEORADIUS*`, `GEOSEARCH` and `GEOSEARCHSTORE`.

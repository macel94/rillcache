#!/usr/bin/env bash
# Optional external container tooling; not part of Cargo's dependency graph.
set -euo pipefail
image=${1:?usage: bash scripts/container-smoke.sh IMAGE}
engine=${CONTAINER_ENGINE:-docker}
cargo=${CARGO:-cargo}
evidence=${CONTAINER_EVIDENCE:-target/container-smoke}
mkdir -p "$evidence"
tmp=$(mktemp -d)
id=''
cleanup() {
    status=$?
    if [[ -n "$id" ]]; then
        "$engine" logs "$id" > "$evidence/failure.log" 2>&1 || true
        "$engine" kill --signal KILL "$id" >/dev/null 2>&1 || true
        "$engine" rm --force "$id" >/dev/null 2>&1 || true
    fi
    rm -rf -- "$tmp"
    exit "$status"
}
trap cleanup EXIT
"$engine" image inspect "$image" > "$evidence/image.json"
[[ $("$engine" image inspect --format '{{.Config.User}}' "$image") == 65532:65532 ]]
[[ $("$engine" image inspect --format '{{json .Config.Entrypoint}}' "$image") == '["/rillcache","--bind","0.0.0.0:6379"]' ]]
[[ $("$engine" image inspect --format '{{json .Config.Healthcheck.Test}}' "$image") == '["CMD","/rillcache","--healthcheck","127.0.0.1:6379"]' ]]
size=$("$engine" image inspect --format '{{.Size}}' "$image")
[[ "$size" =~ ^[0-9]+$ && "$size" -lt 8388608 ]]
printf 'Image size (uncompressed): %s bytes\n' "$size"
for mode in 0 1 4; do
    args=()
    if [[ "$mode" != 0 ]]; then args=(--threads "$mode"); fi
    # No command override for mode 0: verify the actual image defaults.
    id=$("$engine" run --detach --read-only --cap-drop ALL \
        --security-opt no-new-privileges --pids-limit 256 --memory 256m --cpus 1 \
        --publish 127.0.0.1::6379 "$image" "${args[@]}")
    ready=0
    for ((attempt=0; attempt<50; attempt++)); do
        if "$engine" exec "$id" /rillcache --healthcheck 127.0.0.1:6379 >/dev/null 2>&1; then
            ready=1; break
        fi
        sleep 0.1
    done
    [[ "$ready" == 1 ]]
    address=$("$engine" port "$id" 6379/tcp)
    [[ "$address" =~ ^127\.0\.0\.1:[0-9]+$ ]]
    ./target/release/rillcache-bench --addr "$address" \
        --requests 256 --clients 8 --pipeline 8 --keyspace 32 --value-size 64 \
        --warmup 4 --runs 1 > "$evidence/mode-$mode.json"
    RILLCACHE_REFERENCE_ADDR="$address" "$cargo" test --locked --offline --release \
        --test reference supported_subset_matches_current_reference_wire_behavior \
        -- --ignored > "$evidence/conformance-$mode.log" 2>&1
    "$engine" exec "$id" /rillcache --healthcheck 127.0.0.1:6379
    if [[ "$mode" == 0 ]]; then
        "$engine" cp "$id:/rillcache" "$tmp/rillcache"
        readelf --program-headers "$tmp/rillcache" > "$evidence/elf-program-headers.txt"
        readelf --dynamic "$tmp/rillcache" > "$evidence/elf-dynamic.txt"
        if grep -Eq 'INTERP|\(NEEDED\)' "$evidence"/elf-*.txt; then
            printf 'Runtime binary must have no interpreter or shared-library dependencies\n' >&2
            exit 1
        fi
    fi
    "$engine" logs "$id" > "$evidence/server-$mode.log" 2>&1
    # Test cleanup deliberately kills this disposable volatile instance, not a graceful drain.
    "$engine" kill --signal KILL "$id" >/dev/null
    "$engine" rm --force "$id" >/dev/null
    id=''
    printf 'Validated container worker mode: %s (0=per-client)\n' "$mode"
done

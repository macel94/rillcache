SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.ONESHELL:
.DELETE_ON_ERROR:
.DEFAULT_GOAL := help

CARGO ?= cargo
CARGO_FLAGS := --locked --offline
BENCH_ARGS ?= --requests 100000 --clients 8 --pipeline 16 --keyspace 1024 --value-size 64 --warmup 1000 --runs 3
COMPETITOR_JOBS ?= 2
include benchmarks/targets.mk

REDIS_SERVER ?= $(CURDIR)/target/competitors/redis-$(REDIS_VERSION)/src/redis-server
VALKEY_SERVER ?= $(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)/src/valkey-server
RILLCACHE_PORT ?= 16379
REDIS_PORT ?= 16380
VALKEY_PORT ?= 16381

.PHONY: help fmt fmt-check lint test check release run smoke bench-core bench metadata ci package competitors compare
# Always validate cached tools: an interrupted linker may leave a truncated file.
.PHONY: $(CURDIR)/target/competitors/redis-$(REDIS_VERSION)/src/redis-server $(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)/src/valkey-server

help:
	@printf '%s\n' 'make test       Fast unit, TCP, benchmark and policy tests' 'make check      Formatting, all-target Clippy and tests (offline)' 'make ci         Same fast verification as GitHub, plus release benchmark smoke' 'make run        Run on loopback:6379' 'make bench      Microbenchmarks and repeated local TCP benchmarks' 'make compare    Build pinned competitors once; compare three native servers' 'make package    Build a release archive and SHA-256 checksum'

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy $(CARGO_FLAGS) --all-targets --all-features -- -D warnings

test:
	$(CARGO) test $(CARGO_FLAGS)

check: fmt-check lint test

release:
	$(CARGO) build $(CARGO_FLAGS) --release --bins

run:
	$(CARGO) run $(CARGO_FLAGS) --bin rillcache

metadata:
	@mkdir -p target/benchmarks
	{
		printf 'Source revision: '; git rev-parse HEAD
		printf '\nWorking-tree status (empty means clean):\n'; git status --short
		printf '\nCompiler:\n'; rustc -vV
		printf '\nKernel and CPU:\n'; uname -a
		if command -v lscpu >/dev/null; then lscpu; fi
	} > target/benchmarks/environment.txt

smoke: release metadata
	mkdir -p target/benchmarks
	./target/release/rillcache-bench --requests 2000 --clients 4 --pipeline 8 --keyspace 128 --warmup 32 --runs 1 > target/benchmarks/smoke.json

bench-core:
	mkdir -p target/benchmarks
	$(CARGO) bench $(CARGO_FLAGS) --bench core > target/benchmarks/core.jsonl

bench: release metadata bench-core
	./target/release/rillcache-bench $(BENCH_ARGS) > target/benchmarks/local.json

ci: check smoke bench-core

package: release
	name="rillcache-$$(uname -s | tr '[:upper:]' '[:lower:]')-$$(uname -m)"
	mkdir -p target/dist
	tar -czf "target/dist/$$name.tar.gz" -C target/release rillcache rillcache-bench -C "$(CURDIR)" README.md docs/requirements.md docs/benchmarks.md
	(cd target/dist && sha256sum "$$name.tar.gz" > "$$name.tar.gz.sha256")

# These are optional, isolated benchmark tools. They never enter Cargo's graph,
# the application release archive, or the normal check/CI path.
target/competitors/redis-$(REDIS_VERSION).tar.gz: benchmarks/targets.mk
	mkdir -p target/competitors
	curl --proto '=https' --tlsv1.2 --fail --location --show-error --retry 3 --connect-timeout 10 --max-time 180 'https://github.com/redis/redis/archive/refs/tags/$(REDIS_VERSION).tar.gz' -o '$@.tmp'
	printf '%s  %s\n' '$(REDIS_SHA256)' '$@.tmp' | sha256sum --check --status
	mv '$@.tmp' '$@'

target/competitors/valkey-$(VALKEY_VERSION).tar.gz: benchmarks/targets.mk
	mkdir -p target/competitors
	curl --proto '=https' --tlsv1.2 --fail --location --show-error --retry 3 --connect-timeout 10 --max-time 180 'https://github.com/valkey-io/valkey/archive/refs/tags/$(VALKEY_VERSION).tar.gz' -o '$@.tmp'
	printf '%s  %s\n' '$(VALKEY_SHA256)' '$@.tmp' | sha256sum --check --status
	mv '$@.tmp' '$@'

$(CURDIR)/target/competitors/redis-$(REDIS_VERSION)/src/redis-server: target/competitors/redis-$(REDIS_VERSION).tar.gz
	tar -xzf '$<' -C target/competitors
	if ! '$@' --version >/dev/null 2>&1; then rm -f '$@'; fi
	GIT_CEILING_DIRECTORIES='$(CURDIR)/target/competitors' $(MAKE) -C target/competitors/redis-$(REDIS_VERSION)/src -j$(COMPETITOR_JOBS) redis-server
	'$@' --version

$(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)/src/valkey-server: target/competitors/valkey-$(VALKEY_VERSION).tar.gz
	tar -xzf '$<' -C target/competitors
	if ! '$@' --version >/dev/null 2>&1; then rm -f '$@'; fi
	GIT_CEILING_DIRECTORIES='$(CURDIR)/target/competitors' $(MAKE) -C target/competitors/valkey-$(VALKEY_VERSION)/src -j$(COMPETITOR_JOBS) valkey-server
	'$@' --version

competitors: $(REDIS_SERVER) $(VALKEY_SERVER)

compare: release metadata competitors
	mkdir -p target/benchmarks/comparison/{redis-data,valkey-data}
	pids=()
	cleanup() {
		for pid in "$${pids[@]}"; do kill "$$pid" 2>/dev/null || true; done
		for pid in "$${pids[@]}"; do wait "$$pid" 2>/dev/null || true; done
	}
	trap cleanup EXIT
	trap 'exit 130' INT
	trap 'exit 143' TERM
	./target/release/rillcache --bind '127.0.0.1:$(RILLCACHE_PORT)' > target/benchmarks/comparison/rillcache.log 2>&1 &
	pids+=("$$!")
	'$(REDIS_SERVER)' --bind 127.0.0.1 --port '$(REDIS_PORT)' --save '' --appendonly no --daemonize no --loglevel warning --dir '$(CURDIR)/target/benchmarks/comparison/redis-data' > target/benchmarks/comparison/redis.log 2>&1 &
	pids+=("$$!")
	'$(VALKEY_SERVER)' --bind 127.0.0.1 --port '$(VALKEY_PORT)' --save '' --appendonly no --daemonize no --loglevel warning --dir '$(CURDIR)/target/benchmarks/comparison/valkey-data' > target/benchmarks/comparison/valkey.log 2>&1 &
	pids+=("$$!")
	cp target/benchmarks/environment.txt target/benchmarks/comparison/environment.txt
	{
		printf '\nCompetitor versions:\n'
		'$(REDIS_SERVER)' --version
		'$(VALKEY_SERVER)' --version
		printf '\nDefault source pins (custom binary overrides may differ):\n'
		printf 'Redis %s sha256:%s\nValkey %s sha256:%s\n' '$(REDIS_VERSION)' '$(REDIS_SHA256)' '$(VALKEY_VERSION)' '$(VALKEY_SHA256)'
		printf '\nC compiler:\n'; if command -v cc >/dev/null; then cc --version; fi
		printf '\nBinary checksums:\n'
		sha256sum target/release/rillcache '$(REDIS_SERVER)' '$(VALKEY_SERVER)'
		printf '\nClient arguments: %s\n' '$(BENCH_ARGS)'
		printf '\nCompetitor runtime: loopback TCP; no RDB/AOF; default I/O threads; no configuration file or modules loaded.\n'
		printf 'Competitor builds: upstream default compiler optimization and allocator; no TLS.\n'
	} >> target/benchmarks/comparison/environment.txt
	names=(rillcache redis valkey)
	ports=('$(RILLCACHE_PORT)' '$(REDIS_PORT)' '$(VALKEY_PORT)')
	for i in "$${!names[@]}"; do
		kill -0 "$${pids[$$i]}"
		# Readiness is validated over RESP, never guessed using a fixed sleep.
		./target/release/rillcache-bench --addr "127.0.0.1:$${ports[$$i]}" --requests 1 --clients 1 --pipeline 1 --keyspace 1 --warmup 0 --runs 1 --workload ping > /dev/null
		kill -0 "$${pids[$$i]}"
		if (( i > 0 )); then
			RILLCACHE_REFERENCE_ADDR="127.0.0.1:$${ports[$$i]}" $(CARGO) test $(CARGO_FLAGS) --test reference -- --ignored
		fi
		./target/release/rillcache-bench $(BENCH_ARGS) --addr "127.0.0.1:$${ports[$$i]}" --label "$${names[$$i]}" > "target/benchmarks/comparison/$${names[$$i]}.json"
		kill -0 "$${pids[$$i]}"
	done

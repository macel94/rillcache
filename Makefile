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
SERVER_CPUS ?=
CLIENT_CPUS ?=
BENCH_ORDER ?= rillcache redis valkey
PROFILE ?= 0
PROFILE_IDLE_MS ?= 1000
REQUIRE_THREAD_MODES ?= 0
CONTAINER_ENGINE ?= docker
CONTAINER_IMAGE ?= rillcache:local

.PHONY: help fmt fmt-check lint test check release run smoke bench-core bench bench-threads metadata ci package competitors compare container container-smoke
# Always validate cached tools: an interrupted linker may leave a truncated file.
.PHONY: $(CURDIR)/target/competitors/redis-$(REDIS_VERSION)/src/redis-server $(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)/src/valkey-server

help:
	@printf '%s\n' 'make test       Fast unit, TCP, benchmark and policy tests' 'make check      Formatting, all-target Clippy and tests (offline)' 'make ci         Same fast verification as GitHub, plus release benchmark smoke' 'make run        Run on loopback:6379' 'make bench      Microbenchmarks and repeated local TCP benchmarks' 'make compare    Build pinned competitors once; compare three native servers' 'make package    Build a release archive and SHA-256 checksum' 'make container  Build the optional thin, non-root container image' 'make container-smoke  Test a built image in all three worker modes'

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy $(CARGO_FLAGS) --all-targets --all-features -- -D warnings

test:
	for workers in 0 1 4; do
		printf '\nTesting server mode (0=thread-per-client): %s\n' "$$workers"
		RILLCACHE_TEST_THREADS="$$workers" $(CARGO) test $(CARGO_FLAGS)
	done

check: fmt-check lint test

release:
	$(CARGO) build $(CARGO_FLAGS) --release --bins

run:
	$(CARGO) run $(CARGO_FLAGS) --bin rillcache

metadata:
	@mkdir -p target/benchmarks
	{
		if [[ '$(REQUIRE_THREAD_MODES)' == 1 ]]; then printf 'Thread-mode measurements: required\n'; fi
		printf 'Source revision: '; git rev-parse HEAD
		printf '\nWorking-tree status (empty means clean):\n'; git status --short
		printf '\nCompiler:\n'; rustc -vV
		printf '\nKernel and CPU:\n'; uname -a
		if command -v lscpu >/dev/null; then lscpu; fi
	} > target/benchmarks/environment.txt

smoke: release metadata
	mkdir -p target/benchmarks
	./target/release/rillcache-bench --requests 2000 --clients 4 --pipeline 8 --keyspace 128 --warmup 32 --runs 1 > target/benchmarks/smoke.json
	for workers in 1 4; do
		./target/release/rillcache-bench --server-threads "$$workers" --requests 2000 --clients 8 --pipeline 8 --keyspace 128 --warmup 32 --runs 1 > "target/benchmarks/smoke-threads-$$workers.json"
	done

bench-core:
	mkdir -p target/benchmarks
	$(CARGO) bench $(CARGO_FLAGS) --bench core > target/benchmarks/core.jsonl

bench-threads: release metadata
	mkdir -p target/benchmarks/thread-modes
	rm -f target/benchmarks/thread-modes/threads-{1,4}.json
	cp target/benchmarks/environment.txt target/benchmarks/thread-modes/environment.txt
	for workers in 1 4; do
		./target/release/rillcache-bench --server-threads "$$workers" $(BENCH_ARGS) > "target/benchmarks/thread-modes/threads-$$workers.json"
	done

bench: release metadata bench-core bench-threads
	./target/release/rillcache-bench $(BENCH_ARGS) > target/benchmarks/local.json

ci: check smoke bench-core

package: release
	name="rillcache-$$(uname -s | tr '[:upper:]' '[:lower:]')-$$(uname -m)"
	mkdir -p target/dist
	tar -czf "target/dist/$$name.tar.gz" -C target/release rillcache rillcache-bench -C "$(CURDIR)" README.md docs compose.yaml deploy
	(cd target/dist && sha256sum "$$name.tar.gz" > "$$name.tar.gz.sha256")

# Container tools/network access are optional and never enter normal offline CI.
container:
	revision="$$(git rev-parse HEAD)"
	if [[ -n "$$(git status --porcelain)" ]]; then revision="$$revision-dirty"; fi
	$(CONTAINER_ENGINE) build --build-arg "VCS_REF=$$revision" --tag '$(CONTAINER_IMAGE)' .

container-smoke: release
	CONTAINER_ENGINE='$(CONTAINER_ENGINE)' CARGO='$(CARGO)' bash scripts/container-smoke.sh '$(CONTAINER_IMAGE)'

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

# Extract each verified archive only once. Empty, isolated Git metadata prevents
# upstream version scripts from discovering this repository OR interpreting
# `git diff ../src ../deps` outside Git as a changing directory comparison.
$(CURDIR)/target/competitors/redis-$(REDIS_VERSION)/.rillcache-source: target/competitors/redis-$(REDIS_VERSION).tar.gz
	printf '%s  %s\n' '$(REDIS_SHA256)' '$<' | sha256sum --check --status
	rm -rf '$(CURDIR)/target/competitors/redis-$(REDIS_VERSION)'
	tar -xzf '$<' -C target/competitors
	git -c init.defaultBranch=benchmark init --quiet --template= '$(CURDIR)/target/competitors/redis-$(REDIS_VERSION)'
	printf '%s\n' '$(REDIS_SHA256)' > '$@'

$(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)/.rillcache-source: target/competitors/valkey-$(VALKEY_VERSION).tar.gz
	printf '%s  %s\n' '$(VALKEY_SHA256)' '$<' | sha256sum --check --status
	rm -rf '$(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)'
	tar -xzf '$<' -C target/competitors
	git -c init.defaultBranch=benchmark init --quiet --template= '$(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)'
	printf '%s\n' '$(VALKEY_SHA256)' > '$@'

$(CURDIR)/target/competitors/redis-$(REDIS_VERSION)/src/redis-server: $(CURDIR)/target/competitors/redis-$(REDIS_VERSION)/.rillcache-source
	if ! '$@' --version >/dev/null 2>&1; then rm -f '$@'; fi
	GIT_CEILING_DIRECTORIES='$(CURDIR)/target/competitors' $(MAKE) -C target/competitors/redis-$(REDIS_VERSION)/src -j$(COMPETITOR_JOBS) redis-server
	'$@' --version

$(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)/src/valkey-server: $(CURDIR)/target/competitors/valkey-$(VALKEY_VERSION)/.rillcache-source
	if ! '$@' --version >/dev/null 2>&1; then rm -f '$@'; fi
	GIT_CEILING_DIRECTORIES='$(CURDIR)/target/competitors' $(MAKE) -C target/competitors/valkey-$(VALKEY_VERSION)/src -j$(COMPETITOR_JOBS) valkey-server
	'$@' --version

competitors: $(REDIS_SERVER) $(VALKEY_SERVER)

compare: release metadata competitors
	mkdir -p target/benchmarks/comparison/{redis-data,valkey-data}
	# An aborted rerun must never leave another server's old JSON as new evidence.
	rm -f target/benchmarks/comparison/{rillcache,redis,valkey}.{json,log} target/benchmarks/comparison/environment.txt
	read -r -a order <<< '$(BENCH_ORDER)'
	if [[ $${#order[@]} != 3 || "$$(printf '%s\n' "$${order[@]}" | sort | tr '\n' ' ')" != 'redis rillcache valkey ' ]]; then
		printf 'BENCH_ORDER must contain rillcache, redis and valkey exactly once\n' >&2
		exit 2
	fi
	server_prefix=()
	client_prefix=()
	if [[ -n '$(SERVER_CPUS)' ]]; then
		taskset --cpu-list '$(SERVER_CPUS)' true
		server_prefix=(taskset --cpu-list '$(SERVER_CPUS)')
	fi
	if [[ -n '$(CLIENT_CPUS)' ]]; then
		taskset --cpu-list '$(CLIENT_CPUS)' true
		client_prefix=(taskset --cpu-list '$(CLIENT_CPUS)')
	fi
	if [[ '$(PROFILE)' != 0 && '$(PROFILE)' != 1 ]]; then
		printf 'PROFILE must be 0 or 1\n' >&2; exit 2
	fi
	pid=''
	cleanup() {
		if [[ -n "$$pid" ]]; then
			kill "$$pid" 2>/dev/null || true
			wait "$$pid" 2>/dev/null || true
			pid=''
		fi
	}
	trap cleanup EXIT
	trap 'exit 130' INT
	trap 'exit 143' TERM
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
		printf 'Server CPU request: %s\nClient CPU request: %s\nServer order: %s\n' '$(or $(SERVER_CPUS),inherited)' '$(or $(CLIENT_CPUS),inherited)' '$(BENCH_ORDER)'
		printf 'Server lifecycle: one live comparison server at a time\nProfiling: %s\nProfile idle milliseconds: %s\n' '$(PROFILE)' '$(PROFILE_IDLE_MS)'
		if command -v taskset >/dev/null; then
			printf 'Actual client affinity:\n'
			"$${client_prefix[@]}" bash -c 'taskset -pc "$$$$"'
		fi
		printf '\nCompetitor runtime: loopback TCP; no RDB/AOF; default I/O threads; no configuration file or modules loaded.\n'
		printf 'Competitor builds: upstream default compiler optimization and allocator; no TLS.\n'
	} >> target/benchmarks/comparison/environment.txt
	for name in "$${order[@]}"; do
		# Idle polling/cron threads from other candidates must not consume the
		# measured server's CPU budget. Start, measure and reap one at a time.
		case "$$name" in
			rillcache)
				port='$(RILLCACHE_PORT)'
				"$${server_prefix[@]}" ./target/release/rillcache --bind "127.0.0.1:$$port" > target/benchmarks/comparison/rillcache.log 2>&1 & ;;
			redis)
				port='$(REDIS_PORT)'
				"$${server_prefix[@]}" '$(REDIS_SERVER)' --bind 127.0.0.1 --port "$$port" --save '' --appendonly no --daemonize no --loglevel warning --dir '$(CURDIR)/target/benchmarks/comparison/redis-data' > target/benchmarks/comparison/redis.log 2>&1 & ;;
			valkey)
				port='$(VALKEY_PORT)'
				"$${server_prefix[@]}" '$(VALKEY_SERVER)' --bind 127.0.0.1 --port "$$port" --save '' --appendonly no --daemonize no --loglevel warning --dir '$(CURDIR)/target/benchmarks/comparison/valkey-data' > target/benchmarks/comparison/valkey.log 2>&1 & ;;
		esac
		pid=$$!
		kill -0 "$$pid"
		# Readiness is validated over RESP, never guessed using a fixed sleep.
		"$${client_prefix[@]}" ./target/release/rillcache-bench --addr "127.0.0.1:$$port" --requests 1 --clients 1 --pipeline 1 --keyspace 1 --warmup 0 --runs 1 --workload ping > /dev/null
		kill -0 "$$pid"
		if command -v taskset >/dev/null; then
			printf '\n%s server affinity:\n' "$$name" >> target/benchmarks/comparison/environment.txt
			taskset -apc "$$pid" >> target/benchmarks/comparison/environment.txt
		fi
		if [[ $$name != rillcache ]]; then
			RILLCACHE_REFERENCE_ADDR="127.0.0.1:$$port" RILLCACHE_REFERENCE_KIND="$$name" $(CARGO) test $(CARGO_FLAGS) --test reference -- --ignored
		fi
		profile_args=()
		if [[ '$(PROFILE)' == 1 ]]; then
			profile_args=(--server-pid "$$pid" --profile-idle-ms '$(PROFILE_IDLE_MS)')
		fi
		"$${client_prefix[@]}" ./target/release/rillcache-bench $(BENCH_ARGS) --addr "127.0.0.1:$$port" --label "$$name" "$${profile_args[@]}" > "target/benchmarks/comparison/$$name.json"
		kill -0 "$$pid"
		cleanup
	done

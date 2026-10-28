# Security and operational prerequisites — plan only

**Status: MIXED — SEC-1's single-password baseline (S27) and a Redis-style multi-user ACL/file subset (S28) are implemented; the remaining stories are design only.** The server now has password and ACL-file authentication, ordered command/category rules, read/write key patterns, Redis 7 selectors, channel enforcement, live `NOAUTH`/`NOPERM` denial, an `ACL` management/save/load/`LOG`/`DRYRUN`/`GENPASS` surface and authenticated `CONFIG SET requirepass`, but still lacks subcommand-specific categories, first-class native TLS, config reload, signal-driven graceful shutdown and a complete production observability contract. Nothing here is an approved security design or a production-readiness claim. Keep the standalone/library loopback default and the documented private-network/container restrictions unchanged; use a password and a TLS terminator before exposing this beyond a trusted network. See [security](../security.md).

This plan decomposes the security and operations prerequisites into small stories. Names below are proposed ownership boundaries for future implementation PRs, not files changed by this plan. Preserve safe Rust, zero external Rust dependencies, binary safety, bounded I/O, atomic errors and joined shutdown. Security changes require human security review; do not treat passing compatibility tests as a security sign-off.

## Current implementation facts and boundaries

- `src/server.rs` owns `Config`, TCP accept, client admission, per-client workers and managed `ServerHandle::shutdown`/`Drop`; these paths join managed workers. `src/server/pool.rs` owns fixed-worker connections. A socket is TCP, not encrypted.
- `src/main.rs` exposes bind/client/worker/payload/idle-timeout and `--requirepass`/`--requirepass-file` options. Bind defaults to `127.0.0.1`; container entrypoint explicitly binds `0.0.0.0`. There is a startup secret file/argument loader but no general config-file reload; runtime credential rotation uses authenticated `CONFIG SET requirepass`.
- `src/engine/auth.rs` owns users, hashed credentials, ordered command rules, read/write key patterns and selectors; `src/engine.rs` exposes atomic ACL checks and ACL-file save/load; `src/session.rs` holds the per-connection user identity and rejects `NOAUTH`/`NOPERM` before dispatch. Channel patterns are enforced for pub/sub. `src/protocol.rs` parses bounded RESP2 frames.
- Library-managed shutdown is available, but `Server::run` has no portable signal-handling path. `docs/containers.md` already states that process termination is not graceful draining and in-flight replies can be lost.
- `max_data_bytes` accounts key/value payload only, not allocator/map capacity, expiry index, thread stacks, connection buffers, kernel socket buffers or process RSS. Existing profiling reports Linux RSS samples but does not enforce a memory limit or eviction policy.

## Proposed INVEST stories and EARS acceptance

### SEC-1 — Authentication state and rejection boundary (implemented baseline, S27)

**Value:** a configured client can authenticate before issuing data commands. **Scope:** a fixed, explicitly configured user set for the initially supported command subset; do not promise full Redis ACL parity by naming a minimal allowlist “ACL”. AUTH semantics must be agreed against the pinned Redis/Valkey references before implementation. Keep unauthenticated mode as an explicit local-development choice only; no silently enabled empty/default password and no default network exposure.

- **When** a connection is accepted in authentication-required mode, **then** its session shall begin unauthenticated and only the explicitly allowed negotiation/auth commands shall execute.
- **When** a valid credential is presented, **then** the connection session shall transition to authenticated state and reply according to the agreed Redis-compatible AUTH grammar; **if** credentials are invalid, malformed, or the account is disabled, **then** it shall return a generic authentication error without revealing username/account existence.
- **If** a data, administrative, or unknown command arrives before authentication, **then** it shall be rejected before command dispatch/store lookup/mutation; a rejected request shall not leak key existence, value bytes or command-specific validation results.
- **If** pipelining places data before AUTH, or AUTH fails before a subsequent data command, **then** no pipelined data command shall run. Test both command orderings and fragmentation; define whether the connection remains open after each failure from upstream behavior rather than guessing.
- **If** AUTH arity, username/password encoding, repeated AUTH, empty values, NUL/binary values, or unsupported username/password forms are invalid, **then** errors shall be bounded, safely encoded and non-reflective. Never log password bytes.
- Tests shall cover unauthenticated read/write, malformed and incorrect credentials, valid credentials, concurrent clients, disconnect/reconnect, pipelines, unknown commands, expiry-triggered removal and proof via a test seam that the store was not accessed on rejection.

**Ownership/prerequisite:** session implementers own proposed `src/server/session.rs` (or the session branch's chosen module), connection lifecycle/state, and narrowly scoped hooks in `src/server.rs`, `src/server/pool.rs`, and `src/engine.rs`. Security implementers own proposed `src/security.rs` plus `src/security/credentials.rs`; do not add code until the sessions owner exposes an agreed session contract. Coordinator must resolve module/API names and merge ordering first.

### SEC-2 — ACL permissions and command metadata (M, depends on SEC-1 and command taxonomy)

**Value:** credentials can be least-privilege rather than global passwords. Redis ACLs include users/passwords, command/category and key-pattern permissions (and channel permissions for pub/sub); these capabilities are broader than the current string command set. Implement an explicitly bounded subset with honest compatibility notes, or defer ACL while shipping only a deliberate fixed-user AUTH story.

- **When** an authenticated user invokes a command outside its configured command permissions, **then** the server shall reject it before data access or mutation.
- **When** a command addresses one or more keys, **then** each key shall be checked against that user's permissions before any effect; a multi-key command shall fail atomically if any key is unauthorized. Preserve duplicate-key command semantics without duplicate permission bypasses.
- **If** permissions use unknown commands, malformed patterns, invalid UTF-8/RESP boundaries, ambiguous aliases/case, or unsupported selectors/categories, **then** configuration shall fail closed before listening, with no partial policy activation.
- Unit/wire tests shall include permitted/denied reads and writes, multiple keys, binary keys and pattern metacharacters; test every implemented command's metadata so new commands cannot accidentally inherit access. Differentially compare the deliberately supported ACL subset with Redis, not a loose “works like ACL” claim.

**Ownership/prerequisite:** proposed `src/security/acl.rs` and command/key metadata in `src/engine/commands.rs` or a dedicated shared `src/engine/metadata.rs`; depends on SEC-1's authenticated user identity and a stable dispatch metadata API. Typed-store work must contribute canonical key extraction for each future command before those commands become ACL-enabled. Pub/sub channel policy is out of scope until channels exist.

### SEC-3 — Credential provisioning, rotation and secret hygiene (S, depends on config design)

**Value:** operators can deploy and rotate credentials without baking them into source/images or exposing them in logs/process arguments. Decide explicitly whether credentials are provisioned through a root-readable file, protected control plane, or another standard-library-compatible source. Command-line password flags and environment variables are poor default secret channels; never put secrets into diagnostics, health output, benchmark JSON or container labels.

- **When** credential configuration is loaded, **then** parsing/validation shall finish before opening the listener; malformed, duplicate, empty, oversized or unreadable input shall fail closed with actionable redacted errors.
- **When** an operator rotates credentials, **then** new connections shall use the new credential generation and existing authenticated sessions shall follow a documented deterministic policy (retain until disconnect, bounded reauthentication, or immediate revoke); no mixed/partially published generation is permitted.
- **If** a reload fails, **then** retain the last known valid configuration and report failure without replacing it; if atomic reload cannot be guaranteed, reject reload rather than partially applying credentials/permissions.
- Verify file permission behavior on supported platforms, symlink/replacement races, truncation, oversized files, duplicate users, process crash during replacement, stale sessions and rotation overlap. Document that credentials sent over plaintext TCP are observable to network intermediaries.

**Ownership/prerequisite:** proposed `src/config.rs` plus `src/config/credentials.rs` or cohesive `src/security/config.rs`; CLI stays in `src/main.rs`. Requires a written secret source/ownership and supported-platform decision, then SEC-1. No external secrets dependency or speculative `.env` convention.

### SEC-4 — Explicit TLS termination boundary (XS documentation/deployment, independent)

**Value:** users can protect transport without an invented cryptographic implementation. Rust `std::net` provides TCP primitives; the standard library does not provide a TLS client/server API or portable signal-handler API. This project forbids external Rust dependencies, FFI and native extensions. Therefore **do not claim native TLS**, do not add hand-rolled crypto, and do not forward plaintext from an untrusted ingress to an exposed backend.

- **When** deployments require network confidentiality, **then** documentation/examples shall require an independently operated, maintained TLS terminator with certificate issuance/renewal, peer/hostname validation at its client-facing side, restricted backend reachability and an explicit trust boundary.
- **If** the proxy-to-server leg is plaintext, **then** it shall remain on a same-host/private isolated network with network policy/firewall and must be described as plaintext; credentials are not protected on that leg. A terminator that merely encrypts client-to-proxy does not establish end-to-end encryption.
- **If** the terminator is unavailable, misconfigured, presents expired/wrong-name certificates, or backend ingress becomes reachable around it, **then** deployment validation shall fail/alert rather than silently bypass TLS. Document that native Redis TLS compatibility is not present.

**Ownership/prerequisite:** independent docs work may begin immediately in `docs/containers.md` and deployment examples (`compose.yaml`, `deploy/kubernetes.yaml`) only in a separate authorized change. No Rillcache TLS code or cryptography. Review actual proxy manifests and network policy; never broaden server bind defaults or image policy to simplify proxying.

### OPS-1 — Termination and bounded graceful drain (S, standard-library constraint)

**Value:** operators know exactly what termination guarantees exist and can improve managed shutdown without claiming portable OS signal capture. The library's explicit `ServerHandle` path already joins workers; the standalone process has no portable `std` signal subscription. `TcpStream::shutdown` can wake socket I/O, but it does not implement a process signal handler or persistence guarantee.

- **When** a managed embedding calls shutdown, **then** stop accepting, prevent new work, close/wake active sockets, and join every started worker in per-client, 1-worker and 4-worker modes within the documented bound; partial startup and worker failure shall also join all started threads.
- **When** an operator terminates the standalone process, **then** docs shall state current behavior and possible lost in-flight reply/data. Until an approved portable/dependency-free design exists, rely on a supervisor's configured timeout/forced kill and do not describe that as graceful draining.
- **If** a future signal/control mechanism is proposed, **then** it shall be reviewed for async-signal safety, platform support, shutdown idempotency, deadline/forced-stop behavior, accept-loop wakeup and joined client workers. No unsafe FFI, detached helper thread or unbounded drain. A control socket/pipe is not signal handling and needs its own threat model.
- Test admission racing shutdown, idle and blocked writers, slow readers, partial requests/replies, worker panic/start failure, repeated shutdown/drop and bounded termination using managed handles and no readiness sleeps.

**Ownership/prerequisite:** `src/server.rs`, `src/server/pool.rs`, `src/main.rs`, and `docs/containers.md`; independent managed-handle tests can begin now. Process signal integration is blocked on an explicit portability/dependency policy decision and security/code-owner review; no dependency exception is presumed.

### OPS-2 — Memory, resource bounds and eviction semantics (M, typed-store dependency)

**Value:** distinguish a key/value admission quota from actual process memory and choose an eviction/resource policy before production deployment. Current `max_data_bytes` is exact payload accounting and OOM behavior is atomic; it is not RSS accounting. Threads, per-connection bounded buffers, indexes, allocator overhead and kernel memory also consume resources. Eviction changes observable Redis semantics and must not be silently introduced as a memory fix.

- **When** payload admission succeeds/fails, **then** existing exact accounting, expiry reclamation and atomic overwrite/error invariants remain true. Tests shall prove rejected writes preserve the old value and TTL in debug and release.
- **When** operators inspect resource metrics, **then** payload bytes, configured payload ceiling, key count, admitted connections, worker mode/count, expirations, rejected clients/commands, and OS RSS (where supported) shall be named separately with units and sampling limitations; no metric shall label payload as memory/RSS.
- **If** adding an RSS/container limit or eviction policy, **then** document it as a distinct approximate/external bound or explicit semantic policy; test allocator overhead, key-heavy/empty-value workloads, expiry churn, large connections, slow readers, OOM and eviction ordering. Never claim a hard RSS ceiling from the payload counter.

**Ownership/prerequisite:** metrics/config surfaces likely in `src/server.rs`, `src/engine.rs`, a future `src/engine/data.rs`/typed-store abstraction, and docs. A read-only status/metrics story can start independently after metric names and privacy are agreed. Exact eviction requires typed-store/accounting owner decision and product policy; do not evict until TTL/atomicity and future types have canonical accounting hooks.

### OPS-3 — Configuration contract and reload (S, depends on SEC-3)

**Value:** predictable startup and operations across binary, library and container. Current CLI flags and `Config` are fixed at bind time; there is no general config file or reload. Maintain loopback default and explicit container bind override.

- **When** configuration is loaded, **then** validate all limits, worker counts, bind address, secret references and incompatible options before binding/listening; errors shall be non-secret and fail closed.
- **If** a reload requests immutable listener/worker changes, **then** reject it or use an explicitly designed listener replacement; do not partially change settings. For mutable limits/ACL/credentials, atomically publish a complete validated generation and define treatment of in-flight commands/sessions.
- Cover defaults, precedence, unknown/duplicate keys, numeric overflow, malformed addresses, permissions, symlinks, empty/truncated/oversized files, config replacement races, reload rollback, and CLI/container override interactions.

**Ownership/prerequisite:** proposed `src/config.rs` (or focused additions to `src/main.rs` and `server::Config`), with docs in `docs/containers.md`; startup config parsing can be independent, reload is blocked on session/config-generation API and SEC-3. Do not create an implicit environment-variable or `.env` contract.

### OPS-4 — Observability, soak, faults and rollout (S, independent planning; implementation gated)

**Value:** detect overload and unsafe rollout without exposing secrets or making unsupported SLO claims.

- **When** a status/metrics surface is added, **then** it shall report only agreed low-cardinality operational facts and no keys, values, credentials, raw command arguments or client secrets. Specify whether counters are process lifetime, reset on restart and available in library/CLI/container modes. Health/readiness shall not imply persistence, replication, security or low load.
- **When** admission, protocol, auth, ACL, timeout, OOM, expiry, worker, config, shutdown or persistence errors occur, **then** operators shall have redacted bounded diagnostics/counters sufficient to distinguish them; credentials and arbitrary binary input shall never be echoed.
- **Before** a rollout, **then** define tested image/source digest, config compatibility, client compatibility, private ingress policy, resource requests/limits, backup/restore where persistence exists, canary duration, abort thresholds, rollback procedure and data-loss behavior. Soak/fault testing shall include prolonged mixed traffic, idle/slow clients, reconnect storms, fragmented/pipelined requests, credential rotation, expiry churn, resource pressure, network partition/reset and termination during writes.
- **If** a required invariant, error budget or resource ceiling fails, **then** abort/rollback and preserve failing evidence; do not rerun until green or relax quality/security gates.

**Ownership/prerequisite:** future tests under server/engine module test ownership and deployment docs/manifests; metrics source in server/store code. Workload definition and numeric alert thresholds require operator/product input, not invention in this plan. Persistence-specific rollout/backup steps wait for a separately designed persistence story.

## Recommended dependency and ownership order

1. **Can start now, independent:** managed-shutdown boundary tests/documentation; TLS-termination boundary documentation plus isolated-network examples; resource/metric vocabulary and adversarial test-plan skeleton. These changes must not claim implementation or weaken defaults.
2. **Sessions branch integration:** merge/agree the session API first (per-connection state shared consistently by per-client and fixed-worker dispatch; initialization, teardown, command gate, and test seam). The current checked-out tree has no session module. AUTH is not a global parser flag: authorization state is connection-scoped.
3. **Security foundation:** choose credential source/ACL subset/default posture; implement atomic config validation and per-session authentication before adding permissions. Then add command/key metadata and narrowly defined ACL operations. Typed-store commands must supply canonical key extractors before access-control coverage expands.
4. **Operations:** metrics and resource status can proceed after names/privacy review; config reload must follow session generation and credential rotation semantics. Eviction, persistence-aware shutdown and production rollout remain separate stories.
5. For every implementation, keep files in the owning module; avoid broad rewrites of `src/server.rs`/engine dispatch by landing stable session/security interfaces first. Update compatibility docs only for tested behavior.

Potential future file ownership (all proposed, not existing APIs):

| Owner story | Candidate files | Prerequisite |
| --- | --- | --- |
| Session integration | `src/server/session.rs`, `src/server.rs`, `src/server/pool.rs` | Concurrent session API agreement; identical gating in both modes |
| Auth/config | `src/security.rs`, `src/security/credentials.rs`, `src/main.rs` | Secret-source decision; session identity/lifecycle |
| ACL metadata | `src/security/acl.rs`, `src/engine/metadata.rs`, `src/engine/commands.rs` | Auth identity, canonical command/key metadata; typed-store command extractors |
| TLS boundary | `docs/containers.md`, `compose.yaml`, `deploy/kubernetes.yaml` | Operator-selected terminator and verified network boundary; no native TLS claim |
| Graceful shutdown | `src/server.rs`, `src/server/pool.rs`, `src/main.rs`, `docs/containers.md` | Managed tests independent; signal path requires policy/portability review |
| Config/reload | `src/config.rs`, `src/main.rs`, `src/server.rs` | Config schema + secret source; session generation for mutable security state |
| Metrics/resource tests | `src/server.rs`, `src/engine/data.rs`, module tests, docs | Metric definitions; typed-store owner for exact payload/expiry invariants |

## Verification commands for the future implementation owners

Do not run these as evidence that this plan is implemented. At minimum, every code story should run (offline, pinned toolchain, `CARGO_BUILD_JOBS=2`):

```sh
cargo fmt --all -- --check
cargo test --offline
cargo test --offline --release
cargo clippy --offline --all-targets -- -D warnings
make ci
```

`make ci` is the repository's authoritative offline suite; run it in addition to focused unit and wire tests. Add ephemeral-port tests using managed handles, no readiness sleeps, with per-client/one-worker/four-worker matrix. For protocol/auth semantics use a pinned current Redis reference and Valkey where behavior is intentionally shared; validate exact replies, pipelines, unauthorized-before-data-access and failure atomicity before timing. Security tests should include malformed binary credentials, wrong/disabled users, pipelines and fragmentation, duplicate/overlapping ACL rules, key-pattern edge cases, credential rotation/reload races, secret leakage scans, and all worker modes. Preserve failing evidence; never weaken tests, alter quality gates, or claim performance/security based on a subset. No benchmark should run concurrently with other workers.

## Risks and review gates

- **Authentication is not encryption.** Credentials sent over plain TCP are exposed to network observers. Do not market AUTH as secure transport.
- **ACLs are easy to under-specify.** Command aliases/subcommands, multi-key commands, binary keys, future typed commands and pattern semantics can create authorization bypasses. Require a human security reviewer and positive/negative corpus for each supported command.
- **Rotation can strand sessions or retain revoked access.** Specify revocation semantics and test old/new connections before implementation.
- **Secret loading can leak or race.** Review file permissions, symlink handling, error redaction, memory lifetime and reload atomicity; do not place secrets in CLI args, logs, benchmark artifacts or images.
- **TLS terminator is an operational dependency.** Verify certificate validation/renewal, backend-only reachability, proxy health and bypass resistance. A plaintext backend is still plaintext.
- **Signal behavior is platform-specific.** Do not assert std has signal registration; avoid unsafe FFI or dependencies without a separately reviewed policy change. Existing managed library shutdown is not proof the executable drains on SIGTERM.
- **Payload is not RSS; process RSS is not an exact admission counter.** Resource limits/eviction need distinct semantics and tests; OOM kills can still lose the in-memory cache.
- **No production readiness claim follows.** Persistence, backups, replication/failover, high availability, realistic SLOs and independent security review remain open.

## Research links and assumption validation

Checked during planning; links point to official references. Redis AUTH documents `AUTH [username] password` and connection authentication; Redis ACL documentation covers user rules, command permissions, key permissions and ACL-file configuration; Redis TLS docs describe Redis's own TLS support. These references are compatibility inputs, not proof this implementation supports them.

- [Redis AUTH command](https://redis.io/docs/latest/commands/auth/)
- [Redis ACL guide](https://redis.io/docs/latest/operate/oss_and_stack/management/security/acl/)
- [Redis TLS guide](https://redis.io/docs/latest/operate/oss_and_stack/management/security/encryption/)
- [Rust `std::net`](https://doc.rust-lang.org/std/net/index.html) — TCP APIs; no TLS API.
- [Rust Unix standard-library extensions](https://doc.rust-lang.org/std/os/unix/index.html) — platform-specific OS extensions do not establish a portable signal API. Confirm against pinned toolchain before considering any platform-specific implementation.
- [Rust `TcpStream`](https://doc.rust-lang.org/std/net/struct.TcpStream.html) — socket I/O/shutdown primitives, not process signal handling or TLS.
- [Container deployment boundary](../containers.md), [requirements/compatibility limits](../requirements.md), [production-readiness plan](../production-readiness.md).

No implementation, command behavior, security assurance or production-readiness item is asserted as complete by this plan.
